#![cfg_attr(not(test), windows_subsystem = "windows")]
#[cfg(not(windows))]
compile_error!("MCGM only supports Windows 10+ (Win32 GUI and Windows Source Engine SDK tools).");

mod anvil;
mod assets;
mod atlas;
mod cli;
mod collision;
mod compiler;
mod glua;
mod gui;
mod instances;
mod mesher;
mod skybox;
mod smd_writer;
mod vmf_builder;

#[cfg(test)]
mod tests;

use crate::anvil::{load_world_region, WorldCoords};
use crate::assets::{AssetIndex, ResolvedBlockModel};
use crate::atlas::{
    build_sign_atlas_with_assets, build_texture_atlas, collect_animated_materials,
    compute_atlas_cache_hash, export_door_textures, load_atlas_cache_meta, save_atlas_cache_meta,
    FontRenderer,
};
use crate::cli::CliArgs;
use crate::collision::build_world_collision_with_models;
use crate::compiler::{
    compile_bsp_map_cancellable, compile_chunk_models_cached, detect_minecraft_saves,
    detect_tools_and_save, load_tool_paths, run_env_check,
    write_addon_metadata, ToolPaths,
};
use crate::glua::install_mc_physics_files;
use crate::gui::{run_gui_with_config, GuiLaunchConfig, UiLang};
use crate::mesher::{
    compute_world_origin_mc, extract_features_and_requests, mc_point_to_source,
    stream_world_chunks_with_animated,
};
use crate::skybox::generate_minecraft_skybox;
use crate::smd_writer::emit_chunk_smds_and_qcs;
use crate::vmf_builder::{build_vmf_checked, VmfBuildParams};
use clap::{parser::ValueSource, CommandFactory, FromArgMatches};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct PipelineOptions {
    pub world_dir: PathBuf,
    pub coords: WorldCoords,
    pub dimension: String,
    pub name: String,
    pub category: String,
    pub map_name: String,
    pub output_dir: PathBuf,
    pub config_path: Option<PathBuf>,
    pub import_to_gmod: bool,
    pub export_vmf: bool,
    pub lighting_style: String,
    pub hq_light: bool,
    pub fast_vrad: bool,
    pub mc_physics: bool,
    pub force_vmf_brushes: bool,
    pub scale: f64,
    pub chunk_size: usize,
    pub max_workers: usize,
    pub max_convex: usize,
    pub reuse_cache: bool,
    pub cancel_flag: Option<Arc<AtomicBool>>,
    pub debug_log: Option<PathBuf>,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            world_dir: PathBuf::new(),
            coords: WorldCoords {
                min_x: -64,
                min_y: -64,
                min_z: -64,
                max_x: 64,
                max_y: 128,
                max_z: 64,
            },
            dimension: "overworld".to_string(),
            name: "mc_export".to_string(),
            category: "custom_mc".to_string(),
            map_name: "gm_mc_export".to_string(),
            output_dir: PathBuf::from("output"),
            config_path: None,
            import_to_gmod: true,
            export_vmf: false,
            lighting_style: "minecraft".to_string(),
            hq_light: false,
            fast_vrad: false,
            mc_physics: true,
            force_vmf_brushes: false,
            scale: 39.37,
            chunk_size: 32,
            max_workers: 0,
            max_convex: 0,
            reuse_cache: false,
            cancel_flag: None,
            debug_log: None,
        }
    }
}

pub fn resolve_debug_log_path(
    explicit: Option<&Path>,
    env_override: Option<&str>,
    work_dir: &Path,
) -> PathBuf {
    if let Some(p) = explicit {
        if !p.as_os_str().is_empty() {
            return p.to_path_buf();
        }
    }
    if let Some(env_s) = env_override {
        let trimmed = env_s.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    work_dir.join("mcgm_debug.log")
}

struct DebugLogFile {
    writer: Option<std::io::BufWriter<std::fs::File>>,
}

impl DebugLogFile {
    fn open(path: &Path) -> Self {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(parent);
        }
        let writer = std::fs::File::create(path)
            .ok()
            .map(std::io::BufWriter::new);
        Self { writer }
    }

    fn write_line(&mut self, elapsed_secs: f64, msg: &str) {
        use std::io::Write;
        if let Some(ref mut w) = self.writer {
            if writeln!(w, "[+{:7.2}s] {}", elapsed_secs, msg).is_ok() {
                let _ = w.flush();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PipelineSummary {
    pub bsp_path: PathBuf,
    pub gma_path: Option<PathBuf>,
    pub chunk_count: usize,
    pub door_count: usize,
    pub total_secs: f64,
    pub stage_timings: Vec<(String, f64)>,
}

fn resolve_config_path(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    let cwd_cfg = PathBuf::from("config.json");
    if cwd_cfg.is_file() {
        return Some(cwd_cfg);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.json")))
        .filter(|p| p.is_file())
}

fn is_valid_world_dir(dir: &Path) -> bool {
    dir.is_dir()
        && (dir.join("level.dat").is_file()
            || dir.join("region").is_dir()
            || dir.join("DIM-1").join("region").is_dir()
            || dir.join("DIM1").join("region").is_dir()
            || dir
                .join("dimensions")
                .join("minecraft")
                .join("overworld")
                .join("region")
                .is_dir()
            || dir
                .join("dimensions")
                .join("minecraft")
                .join("the_nether")
                .join("region")
                .is_dir()
            || dir
                .join("dimensions")
                .join("minecraft")
                .join("the_end")
                .join("region")
                .is_dir())
}

fn auto_select_world(tools: &ToolPaths) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(ref wp) = tools.world_path {
        roots.push(wp.clone());
    }
    roots.extend(detect_minecraft_saves());
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("world"));
        roots.push(cwd.join("saves").join("world"));
        roots.push(cwd.join("input").join("world"));
    }

    for cand in roots {
        if is_valid_world_dir(&cand) {
            return Some(cand);
        }
        // If cand is a saves/ directory containing world subdirectories, pick newest valid save
        if cand.is_dir() {
            let mut sub_worlds: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&cand) {
                for ent in rd.flatten() {
                    let sub = ent.path();
                    if is_valid_world_dir(&sub) {
                        let mtime = sub
                            .join("level.dat")
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::UNIX_EPOCH);
                        sub_worlds.push((mtime, sub));
                    }
                }
            }
            sub_worlds.sort_by_key(|a| std::cmp::Reverse(a.0));
            if let Some((_, best)) = sub_worlds.into_iter().next() {
                return Some(best);
            }
        }
    }
    None
}

pub fn run_export_pipeline(
    mut opts: PipelineOptions,
    mut on_progress: impl FnMut(u32, &str),
) -> Result<PipelineSummary, String> {
    let t_total = Instant::now();
    opts.name = cli::sanitize_safe_name(&opts.name, "mc_export");
    opts.map_name = cli::sanitize_safe_name(&opts.map_name, "gm_mc_export");
    opts.category = cli::sanitize_safe_category(&opts.category);
    let base_output_dir = if opts.output_dir.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(&opts.output_dir))
            .unwrap_or_else(|_| opts.output_dir.clone())
    } else {
        opts.output_dir.clone()
    };
    let addon_dir =
        if base_output_dir.file_name().and_then(|s| s.to_str()) == Some(opts.name.as_str()) {
            base_output_dir.clone()
        } else {
            base_output_dir.join(&opts.name)
        };
    let work_dir = base_output_dir.join("_work").join(&opts.name);
    std::fs::create_dir_all(&addon_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;

    let env_dbg = std::env::var("MCGM_DEBUG_LOG").ok();
    let debug_log_path = resolve_debug_log_path(
        opts.debug_log.as_deref(),
        env_dbg.as_deref(),
        &work_dir,
    );
    let dbg_log = std::sync::Arc::new(Mutex::new(DebugLogFile::open(&debug_log_path)));
    if let Ok(mut g) = dbg_log.lock() {
        g.write_line(
            0.0,
            &format!(
                "=== MCGM Export Debug Log ===\n  world={} dim={} coords=({},{},{})..({},{},{})\n  map={} name={} category={} scale={} chunk_size={}\n  hq_light={} fast_vrad={} reuse_cache={} import_to_gmod={} force_vmf_brushes={}",
                opts.world_dir.display(),
                opts.dimension,
                opts.coords.min_x,
                opts.coords.min_y,
                opts.coords.min_z,
                opts.coords.max_x,
                opts.coords.max_y,
                opts.coords.max_z,
                opts.map_name,
                opts.name,
                opts.category,
                opts.scale,
                opts.chunk_size,
                opts.hq_light,
                opts.fast_vrad,
                opts.reuse_cache,
                opts.import_to_gmod,
                opts.force_vmf_brushes
            ),
        );
    }
    println!("[DEBUG] Writing live debug log: {}", debug_log_path.display());
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let mut timings = Vec::new();

    let check_cancel = || -> Result<(), String> {
        if let Some(ref flag) = opts.cancel_flag {
            if flag.load(Ordering::Relaxed) {
                return Err("Export cancelled by user.".to_string());
            }
        }
        Ok(())
    };

    macro_rules! emit {
        ($pct:expr, $msg:expr) => {{
            let m: &str = $msg;
            if let Ok(mut g) = dbg_log.lock() {
                g.write_line(t_total.elapsed().as_secs_f64(), &format!("[{:>3}%] {}", $pct, m));
            }
            println!("[{:>3}%] {}", $pct, m);
            let _ = std::io::Write::flush(&mut std::io::stdout());
            on_progress($pct, m);
        }};
    }

    check_cancel()?;

    let config_cand = resolve_config_path(opts.config_path.as_deref());
    let tools = load_tool_paths(config_cand.as_deref());
    if let Ok(mut g) = dbg_log.lock() {
        g.write_line(
            t_total.elapsed().as_secs_f64(),
            &format!(
                "Tools: vbsp={} vvis={} vrad={} studiomdl={} vtfcmd={} game_dir={}",
                tools.vbsp.display(),
                tools.vvis.display(),
            tools.vrad.display(),
            tools.studiomdl.display(),
            tools.vtfcmd.display(),
            tools.game_dir.display()
        ),
        );
    }

    let eff_scale = if opts.scale.is_finite() && opts.scale > 0.0 {
        opts.scale
    } else {
        tools.scale_factor
    };
    let eff_chunk_size = if opts.chunk_size > 0 {
        opts.chunk_size
    } else {
        32
    };
    let eff_max_convex = if opts.max_convex > 0 {
        opts.max_convex
    } else if tools.max_convex_pieces > 64 {
        tools.max_convex_pieces
    } else {
        2048
    };
    let eff_max_workers = if opts.max_workers > 0 {
        opts.max_workers
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get().min(16))
            .unwrap_or(4)
    };

    // Stage 1: Parse Anvil .mca + NBT in parallel with Vanilla/Mod JAR discovery
    check_cancel()?;
    emit!(1, "Parsing Minecraft Anvil regions and indexing JAR assets...");
    let t0 = Instant::now();
    let (grid_res, assets) = rayon::join(
        || load_world_region(&opts.world_dir, &opts.dimension, opts.coords, eff_scale),
        || AssetIndex::discover_and_load(tools.vanilla_jar.as_deref(), Some(&opts.world_dir), &[]),
    );
    let grid = grid_res?;
    let dt_anvil = t0.elapsed().as_secs_f64();
    timings.push(("Anvil Parse".to_string(), dt_anvil));
    emit!(
        5,
        &format!(
            "Loaded region {}x{}x{} ({:.2}M voxels, {} unique states, {} signs) in {:.2}s",
            grid.coords.size_x(),
            grid.coords.size_y(),
            grid.coords.size_z(),
            grid.blocks.len() as f64 / 1_000_000.0,
            grid.palette.len(),
            grid.signs.len(),
            dt_anvil
        )
    );

    // Stage 2: Resolve Blockstates/Models in parallel
    check_cancel()?;
    emit!(6, "Resolving block models and extracting map features...");
    let t1 = Instant::now();
    let resolved_models: Vec<ResolvedBlockModel> = grid
        .palette
        .par_iter()
        .map(|bs| assets.resolve_block_model(bs))
        .collect();
    let origin_mc = compute_world_origin_mc(&grid);
    let (mut features, tex_requests) =
        extract_features_and_requests(&grid, &resolved_models, origin_mc, eff_scale);
    let dt_assets = t1.elapsed().as_secs_f64();
    timings.push(("Assets & Models".to_string(), dt_assets));
    emit!(
        9,
        &format!(
            "Resolved {} models ({} textures, {} lights, {} doors, {} buttons) in {:.2}s",
            resolved_models.len(),
            tex_requests.len(),
            features.lights.len(),
            features.doors.len(),
            features.buttons.len(),
            dt_assets
        )
    );

    // Stage 3: Build Texture Atlas, Sign Atlas, Door Textures, Water, Preview & Skybox in parallel
    check_cancel()?;
    emit!(10, "Building texture atlas, sign atlas, animated textures & skybox...");
    let t2 = Instant::now();
    let atlas_hash8 = compute_atlas_cache_hash(&tex_requests, eff_scale, &opts.lighting_style, 8);
    let atlas_png = work_dir.join(format!("atlas_{}.png", atlas_hash8));
    let atlas_meta = work_dir.join(format!("atlas_{}.json", atlas_hash8));
    let sign_png = work_dir.join("sign_atlas.png");
    let door_png_dir = work_dir.join("doors");
    let water_png_dir = work_dir.join("water");
    let sky_png_dir = work_dir.join("skybox");

    let mut atlas_opt = None;
    let mut atlas_cache_hit = false;
    let mut sign_atlas_opt = None;
    let mut atlas_save_res: Result<(), String> = Ok(());
    let mut sign_save_res: Result<(), String> = Ok(());
    let mut door_pngs = std::collections::HashMap::new();
    let mut water_res: Result<(PathBuf, PathBuf), String> = Err("uninitialized".to_string());
    let mut map_preview_512 = image::RgbaImage::new(1, 1);
    let mut sky_res: Result<Vec<(String, PathBuf)>, String> = Err("uninitialized".to_string());
    let mut anim_res = (std::collections::HashMap::new(), Vec::new());

    rayon::scope(|s| {
        s.spawn(|_| {
            if opts.reuse_cache && atlas_png.is_file() && atlas_meta.is_file() {
                if let Ok(cached_atlas) = load_atlas_cache_meta(&atlas_meta) {
                    atlas_cache_hit = true;
                    atlas_opt = Some(cached_atlas);
                    return;
                }
            }
            let a = build_texture_atlas(&tex_requests, &assets, 8);
            let png_res = crate::atlas::save_png_fast(&a.image, &atlas_png);
            let meta_res = if png_res.is_ok() {
                save_atlas_cache_meta(&a, &atlas_meta, 8)
            } else {
                Ok(())
            };
            atlas_save_res = png_res.and(meta_res);
            atlas_opt = Some(a);
        });
        s.spawn(|_| {
            anim_res = collect_animated_materials(&tex_requests, &assets);
        });
        s.spawn(|_| {
            let font = FontRenderer::from_assets(&assets);
            let sa = build_sign_atlas_with_assets(&grid, &assets, &font);
            sign_save_res = crate::atlas::save_png_fast(&sa.image, &sign_png);
            sign_atlas_opt = Some(sa);
        });
        s.spawn(|_| {
            door_pngs = export_door_textures(&features.door_block_names, &assets, &door_png_dir);
        });
        s.spawn(|_| {
            water_res = compiler::export_animated_water_textures(&assets, &water_png_dir);
        });
        s.spawn(|_| {
            map_preview_512 = compiler::render_orthographic_map_preview(
                &grid,
                &assets,
                &opts.map_name,
                &opts.lighting_style,
            );
        });
        s.spawn(|_| {
            sky_res = generate_minecraft_skybox(&opts.lighting_style, &sky_png_dir);
        });
    });

    drop(assets);
    atlas_save_res?;
    sign_save_res?;
    let atlas = atlas_opt.unwrap();
    let sign_atlas = sign_atlas_opt.unwrap();
    let (water_still_png, water_flow_png) = water_res?;
    let skybox_pngs = sky_res?;
    let (anim_lookup, anim_entries) = anim_res;
    let dt_atlas = t2.elapsed().as_secs_f64();
    timings.push(("Atlas & Skybox".to_string(), dt_atlas));
    emit!(
        13,
        &format!(
            "{} atlas {}x{} ({}), {} animated textures, {} door textures in {:.2}s",
            if atlas_cache_hit { "Reused cached" } else { "Built" },
            atlas.width,
            atlas.height,
            atlas_hash8,
            anim_entries.len(),
            door_pngs.len(),
            dt_atlas
        )
    );

    // Stage 4: 3D Greedy Collision (runs first so temporary voxel masks are freed before meshing)
    check_cancel()?;
    emit!(
        14,
        &format!(
            "Computing {}x{} chunk 3D collision geometry...",
            eff_chunk_size, eff_chunk_size
        )
    );
    let t3 = Instant::now();
    let mut col_res =
        build_world_collision_with_models(&grid, Some(&resolved_models), origin_mc, eff_scale, eff_chunk_size);
    let dt_col = t3.elapsed().as_secs_f64();
    timings.push(("Greedy Collision".to_string(), dt_col));
    emit!(
        20,
        &format!(
            "Built collision: {} solid boxes, {} water pools, {} hazards, {} physics zones in {:.2}s",
            col_res.all_solid_boxes.len(),
            col_res.water_pools.len(),
            col_res.hazard_boxes.len(),
            col_res.physics_zones.len(),
            dt_col
        )
    );

    // Stage 5: Streaming Chunk Meshing + Direct SMD & QC Generation
    check_cancel()?;
    emit!(21, "Meshing chunks and writing .smd / _phys.smd / .qc files...");
    let t4 = Instant::now();
    let smd_dir = work_dir.join("smd");
    if !opts.reuse_cache {
        let _ = std::fs::remove_dir_all(&smd_dir);
    }
    std::fs::create_dir_all(&smd_dir).map_err(|e| e.to_string())?;

    let expected_chunks = grid.coords.size_x().div_ceil(eff_chunk_size.max(1))
        * grid.coords.size_z().div_ceil(eff_chunk_size.max(1));
    let done_chunks = AtomicUsize::new(0);
    let total_tris_written = AtomicU64::new(0);
    let mesh_prog_gate = Mutex::new((Instant::now(), 0usize));

    let streamed = stream_world_chunks_with_animated(
        &grid,
        &resolved_models,
        &atlas,
        &sign_atlas,
        &features,
        origin_mc,
        eff_scale,
        eff_chunk_size,
        &anim_lookup,
        |cm| {
            let tri_count = cm.solid_tris.len() + cm.translucent_tris.len();
            let c_bmin = cm.bounds_min;
            let c_bmax = cm.bounds_max;
            let empty_boxes = Vec::new();
            let c_boxes = col_res
                .per_chunk
                .get(&(cm.cx, cm.cz))
                .unwrap_or(&empty_boxes);
            let art = emit_chunk_smds_and_qcs(
                &cm,
                c_boxes,
                &smd_dir,
                &opts.category,
                eff_max_convex,
                &opts.name,
            )?;
            drop(cm);
            let done = done_chunks.fetch_add(1, Ordering::Relaxed) + 1;
            let tris_acc = total_tris_written.fetch_add(tri_count as u64, Ordering::Relaxed) + tri_count as u64;
            if let Ok(mut gate) = mesh_prog_gate.try_lock() {
                let (last_t, last_done) = *gate;
                if done == expected_chunks || (done - last_done >= 10 && last_t.elapsed().as_secs_f64() >= 1.2) {
                    *gate = (Instant::now(), done);
                    let pct = 21 + ((done * 14) / expected_chunks.max(1)) as u32;
                    let msg = format!(
                        "Meshing & SMD/QC: {}/{} chunks ({:.2}M tris, {:.1}s elapsed)",
                        done,
                        expected_chunks,
                        tris_acc as f64 / 1_000_000.0,
                        t4.elapsed().as_secs_f64()
                    );
                    if let Ok(mut g) = dbg_log.lock() {
                        g.write_line(t_total.elapsed().as_secs_f64(), &format!("[{:>3}%] {}", pct.min(35), msg));
                    }
                    println!("[{:>3}%] {}", pct.min(35), msg);
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
            }
            Ok((art, c_bmin, c_bmax))
        },
    )?;

    let mut bmin = [f64::INFINITY; 3];
    let mut bmax = [f64::NEG_INFINITY; 3];
    let mut chunk_artifacts = Vec::with_capacity(streamed.len());
    for (art, c_bmin, c_bmax) in streamed {
        for i in 0..3 {
            bmin[i] = bmin[i].min(c_bmin[i]);
            bmax[i] = bmax[i].max(c_bmax[i]);
        }
        chunk_artifacts.push(art);
    }

    // Free heavy voxel/atlas/per-chunk structures immediately before StudioMDL compilation
    drop(grid);
    drop(resolved_models);
    drop(atlas);
    drop(sign_atlas);
    drop(anim_lookup);
    col_res.per_chunk.clear();
    col_res.per_chunk.shrink_to_fit();
    features.skip_mesh_bits.clear();
    features.skip_mesh_bits.shrink_to_fit();

    let dt_smd = t4.elapsed().as_secs_f64();
    timings.push(("Meshing & SMD/QC".to_string(), dt_smd));
    emit!(
        35,
        &format!(
            "Generated {} chunk SMDs ({:.2}M tris) in {:.2}s",
            chunk_artifacts.len(),
            total_tris_written.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            dt_smd
        )
    );

    struct StagedGameFilesGuard {
        game_dir: PathBuf,
        model_rel_paths: Vec<String>,
        vmt_paths: Vec<PathBuf>,
    }
    impl Drop for StagedGameFilesGuard {
        fn drop(&mut self) {
            if !self.game_dir.as_os_str().is_empty() && !self.model_rel_paths.is_empty() {
                compiler::cleanup_staged_model_files(&self.game_dir, &self.model_rel_paths);
            }
            if !self.vmt_paths.is_empty() {
                compiler::cleanup_staged_files(&self.vmt_paths);
            }
        }
    }
    let mut staged_guard = StagedGameFilesGuard {
        game_dir: if !opts.import_to_gmod {
            tools.game_dir.clone()
        } else {
            PathBuf::new()
        },
        model_rel_paths: if !opts.import_to_gmod {
            let mut paths = Vec::new();
            for ch in &chunk_artifacts {
                if !ch.model_rel_path.is_empty() {
                    paths.push(ch.model_rel_path.clone());
                }
                if let Some(ref tm) = ch.translucent_model_rel_path {
                    if !tm.is_empty() {
                        paths.push(tm.clone());
                    }
                }
                for (_, col_mdl) in &ch.extra_collision_models {
                    if !col_mdl.is_empty() {
                        paths.push(col_mdl.clone());
                    }
                }
            }
            paths
        } else {
            Vec::new()
        },
        vmt_paths: Vec::new(),
    };

    // Stage 6: Compile Materials & Static Props (VTFCmd + StudioMDL)
    check_cancel()?;
    emit!(36, "Compiling VTF/VMT materials and StudioMDL props...");
    let t5 = Instant::now();
    let all_cached = opts.reuse_cache
        && !chunk_artifacts.is_empty()
        && chunk_artifacts.iter().all(|ch| {
            let main_mdl = addon_dir.join(&ch.model_rel_path);
            let main_ok = !ch.qc_path.is_file()
                || (main_mdl.is_file()
                    && (!compiler::qc_has_collision_model(&ch.qc_path)
                        || main_mdl.with_extension("phy").is_file())
                    && compiler::qc_cache_signature_matches(&ch.qc_path, &main_mdl));
            let trans_ok = match (&ch.translucent_qc_path, &ch.translucent_model_rel_path) {
                (Some(tqc), Some(tm)) => {
                    addon_dir.join(tm).is_file()
                        && compiler::qc_cache_signature_matches(tqc, &addon_dir.join(tm))
                }
                (None, Some(tm)) => addon_dir.join(tm).is_file(),
                _ => true,
            };
            let extra_col_ok = ch.extra_collision_models.iter().all(|(cqc, m)| {
                let mdl_p = addon_dir.join(m);
                mdl_p.is_file()
                    && mdl_p.with_extension("phy").is_file()
                    && compiler::qc_cache_signature_matches(cqc, &mdl_p)
            });
            main_ok && trans_ok && extra_col_ok
        });

    let compile_mats_fn = || -> Result<(), String> {
        compiler::compile_materials(
            &tools,
            &opts.category,
            &atlas_png,
            &sign_png,
            &door_pngs,
            &skybox_pngs,
            &addon_dir,
            opts.import_to_gmod,
            opts.reuse_cache,
        )?;
        compiler::compile_water_materials(
            &tools,
            &opts.category,
            &water_still_png,
            &water_flow_png,
            &addon_dir,
            opts.import_to_gmod,
        )?;
        compiler::compile_animated_materials(
            &tools,
            &opts.category,
            &anim_entries,
            &addon_dir,
            opts.import_to_gmod,
        )
    };

    if all_cached {
        compile_mats_fn()?;
        compiler::stage_cached_models_to_game_dir(&addon_dir, &tools.game_dir, &chunk_artifacts)?;
        emit!(75, "Fast rebuild (--reuse-cache): Reusing existing compiled chunk .mdl files.");
    } else {
        let mdl_prog_gate = Mutex::new(Instant::now());
        let (mat_res, mdl_res) = rayon::join(compile_mats_fn, || {
            compile_chunk_models_cached(
                &tools,
                &mut chunk_artifacts,
                &addon_dir,
                true,
                eff_max_workers,
                opts.reuse_cache,
                |done, total, cached, chunk_name| {
                    if let Ok(mut last_t) = mdl_prog_gate.try_lock() {
                        if done == total || last_t.elapsed().as_secs_f64() >= 1.5 {
                            *last_t = Instant::now();
                            let elapsed = t5.elapsed().as_secs_f64();
                            let pct = (36 + ((done * 39) / total.max(1)) as u32).min(75);
                            let eta_str = if done > cached && done < total && elapsed > 0.5 {
                                let rem = (total - done) as f64;
                                let rate = (done - cached) as f64 / elapsed;
                                if rate > 0.01 {
                                    format!(", ETA ~{:.0}s", rem / rate)
                                } else {
                                    String::new()
                                }
                            } else {
                                String::new()
                            };
                            let msg = format!(
                                "StudioMDL: {}/{} models ({} cached, {:.1}s elapsed{}) [{}]",
                                done, total, cached, elapsed, eta_str, chunk_name
                            );
                            if let Ok(mut g) = dbg_log.lock() {
                                g.write_line(t_total.elapsed().as_secs_f64(), &format!("[{:>3}%] {}", pct, msg));
                            }
                            println!("[{:>3}%] {}", pct, msg);
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                    }
                },
            )
        });
        mat_res?;
        mdl_res?;
    }
    drop(anim_entries);
    let dt_mdl = t5.elapsed().as_secs_f64();
    timings.push(("VTFCmd & StudioMDL".to_string(), dt_mdl));
    emit!(
        75,
        &format!("Compiled {} chunk models and materials in {:.2}s", chunk_artifacts.len(), dt_mdl)
    );

    // Stage 7: Build VMF
    check_cancel()?;
    emit!(76, "Generating Source Engine VMF level...");
    let t6 = Instant::now();
    if !bmin[0].is_finite() {
        bmin = [-512.0, -512.0, 0.0];
        bmax = [512.0, 512.0, 512.0];
    }

    let spawn_pts_src: Vec<[f64; 3]> = features
        .spawn_points_mc
        .iter()
        .map(|sp| {
            let s = mc_point_to_source(*sp, origin_mc, eff_scale);
            [s[0] as f64, s[1] as f64, s[2] as f64]
        })
        .collect();

    let vmf_path = work_dir.join(format!("{}.vmf", opts.map_name));
    let vmf_problems = build_vmf_checked(&VmfBuildParams {
        output_vmf: &vmf_path,
        category: &opts.category,
        bounds_min: bmin,
        bounds_max: bmax,
        chunks: &chunk_artifacts,
        all_solid_boxes: &col_res.all_solid_boxes,
        ladder_boxes: &col_res.ladder_boxes,
        water_pools: &col_res.water_pools,
        hazard_boxes: &col_res.hazard_boxes,
        core_occluders: &col_res.core_occluders,
        doors: &features.doors,
        buttons: &features.buttons,
        lights: &features.lights,
        spawn_points_source: &spawn_pts_src,
        lighting_style: &opts.lighting_style,
        hq_light: opts.hq_light,
        scale_factor: eff_scale,
        force_vmf_brushes: opts.force_vmf_brushes,
    })?;
    let chunk_count = chunk_artifacts.len();
    let door_count = features.doors.len();
    drop(chunk_artifacts);
    let lights_count = features.lights.len();
    // Free remaining VMF geometry/feature buffers before spawning VBSP/VVIS/VRAD
    col_res.all_solid_boxes.clear();
    col_res.all_solid_boxes.shrink_to_fit();
    col_res.ladder_boxes.clear();
    col_res.ladder_boxes.shrink_to_fit();
    col_res.water_pools.clear();
    col_res.water_pools.shrink_to_fit();
    col_res.hazard_boxes.clear();
    col_res.hazard_boxes.shrink_to_fit();
    col_res.core_occluders.clear();
    col_res.core_occluders.shrink_to_fit();
    drop(features);

    if opts.export_vmf {
        let vmf_filename = format!("{}.vmf", opts.map_name);
        let addon_vmf = addon_dir.join(&vmf_filename);
        if let Err(e) = std::fs::copy(&vmf_path, &addon_vmf) {
            eprintln!(
                "[Pipeline] Warning: Failed copying VMF to {}: {}",
                addon_vmf.display(),
                e
            );
        } else {
            println!("[Pipeline] Exported VMF: {}", addon_vmf.display());
        }
        if opts.import_to_gmod && !tools.game_dir.as_os_str().is_empty() {
            let mapsrc_dir = tools.game_dir.join("mapsrc");
            if let Err(e) = std::fs::create_dir_all(&mapsrc_dir) {
                eprintln!(
                    "[Pipeline] Warning: Failed creating mapsrc dir {}: {}",
                    mapsrc_dir.display(),
                    e
                );
            } else {
                let game_vmf = mapsrc_dir.join(&vmf_filename);
                if let Err(e) = std::fs::copy(&vmf_path, &game_vmf) {
                    eprintln!(
                        "[Pipeline] Warning: Failed copying VMF to {}: {}",
                        game_vmf.display(),
                        e
                    );
                }
            }
        }
    }
    for warn in vmf_problems {
        let wmsg = format!("[Pipeline] Warning (VMF check): {}", warn);
        if let Ok(mut g) = dbg_log.lock() {
            g.write_line(t_total.elapsed().as_secs_f64(), &wmsg);
        }
        println!("{}", wmsg);
        on_progress(78, &wmsg);
    }
    let dt_vmf = t6.elapsed().as_secs_f64();
    timings.push(("VMF Generation".to_string(), dt_vmf));
    emit!(
        78,
        &format!(
            "Generated VMF ({} props, {} lights, {} doors) in {:.2}s",
            chunk_count, lights_count, door_count, dt_vmf
        )
    );

    // Stage 8: Compile BSP (VBSP + VVIS + VRAD) & GLua Physics
    check_cancel()?;
    let span_x = (opts.coords.max_x - opts.coords.min_x).abs();
    let span_z = (opts.coords.max_z - opts.coords.min_z).abs();
    let auto_fast_vrad = !opts.hq_light && (span_x > 250 || span_z > 250);
    let effective_fast_vrad = opts.fast_vrad || auto_fast_vrad;
    if auto_fast_vrad && !opts.fast_vrad {
        emit!(
            78,
            &format!(
                "Large region ({}x{} blocks > 250): auto-enabling -fast VRAD/VVIS.",
                span_x, span_z
            )
        );
    }

    if !opts.import_to_gmod && !tools.game_dir.as_os_str().is_empty() {
        staged_guard
            .vmt_paths
            .extend(compiler::stage_collision_vmts_to_game_dir(
                &addon_dir,
                &tools.game_dir,
                &opts.category,
            )?);
        staged_guard
            .vmt_paths
            .extend(compiler::stage_water_vmts_to_game_dir(
                &addon_dir,
                &tools.game_dir,
                &opts.category,
            )?);
    }

    emit!(79, "Compiling BSP (VBSP / VVIS / VRAD)...");
    let t7 = Instant::now();
    let mut bsp_pct = 79u32;
    let bsp_res = compile_bsp_map_cancellable(
        &tools,
        &vmf_path,
        &opts.map_name,
        &addon_dir,
        opts.import_to_gmod,
        effective_fast_vrad,
        opts.hq_light,
        &opts.lighting_style,
        opts.cancel_flag.as_deref(),
        |line| {
            let next_pct = if line.contains("Emitting static props") {
                81
            } else if line.contains("Running VVIS") {
                84
            } else if line.contains("Running VRAD") {
                88
            } else if line.contains("BuildFacelights") {
                90
            } else if line.contains("BounceLight") {
                92
            } else if line.contains("ComputeStaticPropLighting") {
                94
            } else if line.contains("ComputeLeafAmbient") {
                96
            } else {
                bsp_pct
            };
            if next_pct > bsp_pct {
                bsp_pct = next_pct;
                if let Ok(mut g) = dbg_log.lock() {
                    g.write_line(t_total.elapsed().as_secs_f64(), &format!("[{:>3}%] {}", bsp_pct, line));
                }
                println!("[{:>3}%] {}", bsp_pct, line);
            } else {
                if let Ok(mut g) = dbg_log.lock() {
                    g.write_line(t_total.elapsed().as_secs_f64(), line);
                }
                println!("{}", line);
            }
            let _ = std::io::Write::flush(&mut std::io::stdout());
            on_progress(bsp_pct, line);
        },
    );
    drop(staged_guard);
    let final_bsp = match bsp_res {
        Ok(p) => p,
        Err(e) => {
            if let Ok(mut g) = dbg_log.lock() {
                g.write_line(t_total.elapsed().as_secs_f64(), &format!("Status: FAILED - {}", e));
            }
            return Err(e);
        }
    };
    let dt_bsp = t7.elapsed().as_secs_f64();
    timings.push(("VBSP/VVIS/VRAD".to_string(), dt_bsp));

    check_cancel()?;
    emit!(98, "Installing GLua Minecraft physics scripts & metadata...");
    let addon_lua = addon_dir.join("lua");
    let game_lua = tools.game_dir.join("lua");
    let mut lua_dests = vec![addon_lua.as_path()];
    if opts.import_to_gmod && !tools.game_dir.as_os_str().is_empty() {
        lua_dests.push(game_lua.as_path());
    }
    install_mc_physics_files(
        &opts.map_name,
        &lua_dests,
        eff_scale,
        opts.mc_physics,
        &col_res.physics_zones,
    )?;

    if let Err(e) = write_addon_metadata(&addon_dir, &opts.map_name) {
        eprintln!("[Pipeline] Warning: failed writing addon.json: {}", e);
    }
    if let Err(e) = compiler::save_map_thumbnail_image(
        &map_preview_512,
        &opts.map_name,
        &addon_dir,
        if opts.import_to_gmod && !tools.game_dir.as_os_str().is_empty() {
            Some(tools.game_dir.as_path())
        } else {
            None
        },
    ) {
        eprintln!("[Pipeline] Warning: failed saving orthographic map preview: {}", e);
    }

    emit!(99, "Creating standalone .gma addon package...");
    let gma_out = addon_dir
        .parent()
        .unwrap_or(&opts.output_dir)
        .join(format!("{}.gma", opts.map_name));
    let gma_path = match compiler::create_gma_addon(&tools, &addon_dir, &gma_out, &opts.map_name, |msg| {
        if let Ok(mut g) = dbg_log.lock() {
            g.write_line(t_total.elapsed().as_secs_f64(), msg);
        }
        println!("{}", msg);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        on_progress(99, msg);
    }) {
        Ok(p) => Some(p),
        Err(e) => {
            if let Ok(mut g) = dbg_log.lock() {
                g.write_line(t_total.elapsed().as_secs_f64(), &format!("[Pipeline] Warning: failed creating .gma addon: {}", e));
            }
            eprintln!("[Pipeline] Warning: failed creating .gma addon: {}", e);
            None
        }
    };

    let total_secs = t_total.elapsed().as_secs_f64();
    let bsp_mb = final_bsp
        .metadata()
        .map(|m| m.len() as f64 / (1024.0 * 1024.0))
        .unwrap_or(0.0);
    emit!(
        100,
        &format!(
            "Done! Map: {} ({:.1} MB) in {:.2}s",
            final_bsp.display(),
            bsp_mb,
            total_secs
        )
    );
    if let Ok(mut g) = dbg_log.lock() {
        g.write_line(total_secs, "Status: SUCCESS");
        for (stage, secs) in &timings {
            g.write_line(total_secs, &format!("  {:<28} {:>6.3}s", stage, secs));
        }
        g.write_line(total_secs, &format!("  {:<28} {:>6.3}s", "TOTAL", total_secs));
    }
    println!("Status: SUCCESS");
    println!("Playable Map: maps/{}.bsp -> {}", opts.map_name, final_bsp.display());
    if let Some(ref gp) = gma_path {
        println!("Standalone GMA Addon: {}", gp.display());
    }
    println!("=== Stage Timings ===");
    for (stage, secs) in &timings {
        println!("  {:<28} {:>6.3}s", stage, secs);
    }
    println!("  {:<28} {:>6.3}s", "TOTAL", total_secs);

    Ok(PipelineSummary {
        bsp_path: final_bsp,
        gma_path,
        chunk_count,
        door_count,
        total_secs,
        stage_timings: timings,
    })
}

fn is_cli_explicit(matches: &clap::ArgMatches, id: &str) -> bool {
    matches.value_source(id) == Some(ValueSource::CommandLine)
}

#[cfg(all(windows, not(test)))]
fn attach_parent_console_if_cli() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
        fn GetStdHandle(n_std_handle: u32) -> isize;
    }
    const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;

    let mut argc = 0usize;
    let mut has_gui = false;
    let mut has_help_or_ver = false;
    for arg in std::env::args_os() {
        argc += 1;
        if argc > 1 {
            let s = arg.to_string_lossy();
            if s == "--gui" {
                has_gui = true;
            } else if matches!(s.as_ref(), "-h" | "--help" | "-V" | "--version") {
                has_help_or_ver = true;
            }
        }
    }
    if argc <= 1 || (has_gui && !has_help_or_ver) {
        return;
    }
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h == 0 || h == -1 {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() {
    #[cfg(all(windows, not(test)))]
    attach_parent_console_if_cli();
    // Deeply nested modded NBT chunks can exceed Windows' default 1 MB thread stack.
    let _ = rayon::ThreadPoolBuilder::new()
        .stack_size(16 * 1024 * 1024)
        .build_global();
    let raw_argc = std::env::args_os().len();
    let matches = CliArgs::command().get_matches();
    let args = CliArgs::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    if args.check_env {
        let cfg = resolve_config_path(args.config.as_deref());
        let ok = run_env_check(cfg.as_deref(), args.clean, args.effective_lang(), args.json);
        std::process::exit(if ok { 0 } else { 1 });
    }

    if args.detect_tools {
        let target_cfg = args.config.clone().unwrap_or_else(|| {
            resolve_config_path(None).unwrap_or_else(|| PathBuf::from("config.json"))
        });
        match detect_tools_and_save(&target_cfg) {
            Ok(tools) => {
                if args.json {
                    let payload = serde_json::json!({
                        "config_path": target_cfg.display().to_string(),
                        "studiomdl_path": tools.studiomdl.display().to_string(),
                        "vtfcmd_path": tools.vtfcmd.display().to_string(),
                        "vbsp_path": tools.vbsp.display().to_string(),
                        "vvis_path": tools.vvis.display().to_string(),
                        "vrad_path": tools.vrad.display().to_string(),
                        "game_dir": tools.game_dir.display().to_string(),
                        "hammer_path": tools.hammer.as_ref().map(|p| p.display().to_string()),
                        "vanilla_jar_path": tools.vanilla_jar.as_ref().map(|p| p.display().to_string()),
                        "world_path": tools.world_path.as_ref().map(|p| p.display().to_string()),
                        "scale_factor": tools.scale_factor,
                        "max_convex_pieces": tools.max_convex_pieces,
                    });
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
                    );
                } else {
                    println!("\n================ Auto-Detected External Tools ================");
                    println!("  studiomdl_path   : {}", tools.studiomdl.display());
                    println!("  vtfcmd_path      : {}", tools.vtfcmd.display());
                    println!("  vbsp_path        : {}", tools.vbsp.display());
                    println!("  vvis_path        : {}", tools.vvis.display());
                    println!("  vrad_path        : {}", tools.vrad.display());
                    println!("  game_dir         : {}", tools.game_dir.display());
                    println!(
                        "  hammer_path      : {}",
                        tools
                            .hammer
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "<not set>".to_string())
                    );
                    println!(
                        "  vanilla_jar_path : {}",
                        tools
                            .vanilla_jar
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "<not set>".to_string())
                    );
                    println!(
                        "  world_path       : {}",
                        tools
                            .world_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "<not set>".to_string())
                    );
                    println!("==============================================================");
                    println!("Configuration written to: {}\n", target_cfg.display());
                }
                return;
            }
            Err(e) => {
                eprintln!("Error detecting tools: {}", e);
                std::process::exit(1);
            }
        }
    }

    if args.list_instances {
        let cfg = resolve_config_path(args.config.as_deref());
        let found = instances::discover_all_instances(cfg.as_deref(), &[]);
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&found).unwrap_or_else(|_| "[]".to_string())
            );
        } else {
            let is_ru = args.effective_lang().eq_ignore_ascii_case("ru");
            println!("{}", "=".repeat(78));
            if is_ru {
                println!("  MCGM — ОБНАРУЖЕННЫЕ ИНСТАНСЫ MINECRAFT ({})", found.len());
            } else {
                println!("  MCGM — DISCOVERED MINECRAFT INSTANCES ({})", found.len());
            }
            println!("{}", "=".repeat(78));
            if found.is_empty() {
                if is_ru {
                    println!("  Инстансы и папки saves не найдены.");
                } else {
                    println!("  No instances or saves folders found.");
                }
            } else {
                for inst in &found {
                    let meta = match (inst.mc_version.as_deref(), inst.loader.as_deref()) {
                        (Some(v), Some(l)) => format!("{} / {}", v, l),
                        (Some(v), None) => v.to_string(),
                        (None, Some(l)) => l.to_string(),
                        (None, None) => "-".to_string(),
                    };
                    let world_names: Vec<&str> =
                        inst.worlds.iter().map(|w| w.name.as_str()).collect();
                    println!(
                        "  [{:<7}] {} ({})",
                        inst.launcher.short_label(),
                        inst.name,
                        meta
                    );
                    println!("            saves: {}", inst.saves_dir.display());
                    if !world_names.is_empty() {
                        println!(
                            "            {}: {}",
                            if is_ru { "миры" } else { "worlds" },
                            world_names.join(", ")
                        );
                    }
                }
            }
            println!("{}", "=".repeat(78));
        }
        return;
    }

    if args.should_launch_gui(raw_argc) {
        let parsed_coords = if args.coords.is_empty() {
            None
        } else {
            args.parsed_coords().ok()
        };
        let lang_opt = args.lang.as_deref().map(|l| {
            if l.trim().eq_ignore_ascii_case("ru") {
                UiLang::Ru
            } else {
                UiLang::En
            }
        });
        let import_opt = if args.no_import_to_gmod {
            Some(false)
        } else if is_cli_explicit(&matches, "import_to_gmod") {
            Some(true)
        } else {
            None
        };
        let mc_phys_opt = if args.no_mc_physics {
            Some(false)
        } else if is_cli_explicit(&matches, "mc_physics") {
            Some(true)
        } else {
            None
        };
        let hq_opt = if is_cli_explicit(&matches, "hq_light")
            || args.preset.as_deref() == Some("hq")
        {
            Some(args.effective_hq_light())
        } else {
            None
        };
        let fast_opt = if is_cli_explicit(&matches, "fast_vrad") || args.preset.is_some() {
            Some(args.effective_fast_vrad())
        } else {
            None
        };
        let brushes_opt = if args.vmf_brushes || is_cli_explicit(&matches, "collision_mode") {
            Some(args.effective_vmf_brushes())
        } else {
            None
        };

        let gui_world = if args.world.as_ref().is_some_and(|p| p.is_dir()) {
            args.world.clone()
        } else if let Some(ref inst_query) = args.instance {
            let cfg = resolve_config_path(args.config.as_deref());
            let all = instances::discover_all_instances(cfg.as_deref(), &[]);
            instances::find_instance_by_query(&all, inst_query)
                .and_then(|inst| instances::resolve_world_in_instance(inst, args.world.as_deref()))
                .or_else(|| args.world.clone())
        } else {
            args.world.clone()
        };

        let cfg = GuiLaunchConfig {
            world: gui_world,
            coords: parsed_coords,
            dimension: is_cli_explicit(&matches, "dimension").then(|| args.dimension.clone()),
            name: args.name.clone(),
            category: is_cli_explicit(&matches, "category").then(|| args.category.clone()),
            map_name: args.map.clone(),
            output_dir: is_cli_explicit(&matches, "output_dir").then(|| args.output_dir.clone()),
            config_path: args.config.clone(),
            lighting_style: is_cli_explicit(&matches, "lighting_style")
                .then(|| args.lighting_style.clone()),
            scale: args.scale,
            mc_physics: mc_phys_opt,
            hq_light: hq_opt,
            fast_vrad: fast_opt,
            export_vmf: args.export_vmf.then_some(true),
            import_to_gmod: import_opt,
            force_vmf_brushes: brushes_opt,
            auto_start: args.auto_start,
            lang: lang_opt,
        };
        if let Err(e) = run_gui_with_config(cfg) {
            eprintln!("GUI error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    let coords = match args.parsed_coords() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error parsing --coords: {}", e);
            std::process::exit(2);
        }
    };

    let config_cand = resolve_config_path(args.config.as_deref());
    let tools = load_tool_paths(config_cand.as_deref());

    let mut auto_selected_world = false;
    let world_dir = if let Some(ref inst_query) = args.instance {
        let all = instances::discover_all_instances(config_cand.as_deref(), &[]);
        let Some(inst) = instances::find_instance_by_query(&all, inst_query) else {
            let avail: Vec<String> = all
                .iter()
                .map(|i| format!("{} ({})", i.name, i.launcher.short_label()))
                .collect();
            eprintln!(
                "Error: instance '{}' not found. Available instances: {}",
                inst_query,
                if avail.is_empty() {
                    "<none>".to_string()
                } else {
                    avail.join(", ")
                }
            );
            std::process::exit(2);
        };
        if let Some(resolved) = instances::resolve_world_in_instance(inst, args.world.as_deref()) {
            println!(
                "[Pipeline] Selected instance '{}' -> world: {}",
                inst.name,
                resolved.display()
            );
            resolved
        } else {
            eprintln!(
                "Error: no matching world found in instance '{}' (saves: {}).",
                inst.name,
                inst.saves_dir.display()
            );
            std::process::exit(2);
        }
    } else if let Some(w) = args.world.clone() {
        w
    } else if let Some(auto_w) = auto_select_world(&tools) {
        auto_selected_world = true;
        println!("[Pipeline] Auto-selected Minecraft world: {}", auto_w.display());
        auto_w
    } else {
        eprintln!("Error: --world is required in CLI mode (and no default world was found).");
        std::process::exit(2);
    };

    let eff_mc_physics = if args.no_mc_physics {
        false
    } else if args.mc_physics || auto_selected_world {
        true
    } else {
        args.effective_mc_physics()
    };

    let eff_name = args.effective_name();
    let eff_map = args.effective_map();
    let eff_cat = args.effective_category();

    let opts = PipelineOptions {
        world_dir,
        coords,
        dimension: args.dimension.clone(),
        name: eff_name,
        category: eff_cat,
        map_name: eff_map,
        output_dir: args.output_dir.clone(),
        config_path: args.config.clone(),
        import_to_gmod: args.effective_import_to_gmod(),
        export_vmf: args.export_vmf,
        lighting_style: args.lighting_style.clone(),
        hq_light: args.effective_hq_light(),
        fast_vrad: args.effective_fast_vrad(),
        mc_physics: eff_mc_physics,
        force_vmf_brushes: args.effective_vmf_brushes(),
        scale: args.effective_scale(tools.scale_factor),
        chunk_size: args.chunk_size,
        max_workers: args.max_workers.unwrap_or(0),
        max_convex: args.max_convex.unwrap_or(0),
        reuse_cache: args.effective_reuse_cache(),
        cancel_flag: None,
        debug_log: args.debug_log.clone(),
    };

    match run_export_pipeline(opts, |_, _| {}) {
        Ok(summary) => {
            println!("Export complete: {}", summary.bsp_path.display());
        }
        Err(e) => {
            eprintln!("Status: FAILED - {}", e);
            std::process::exit(1);
        }
    }
}
