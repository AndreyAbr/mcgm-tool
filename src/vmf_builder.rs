use crate::collision::{CollisionBox, HazardBox};
use crate::mesher::{BlockLight, InteractableButton, InteractableDoor};
use crate::skybox::palette_for_style;
use crate::smd_writer::CompiledChunkArtifact;
use std::collections::HashSet;
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct LightingPreset {
    pub sky_name: &'static str,
    pub sun_angles: &'static str,
    pub sun_pitch: &'static str,
    pub sun_light: &'static str,
    pub sun_ambient: &'static str,
    pub sun_color: &'static str,
    pub min_light: &'static str,
    pub fog_color: &'static str,
    pub fog_start: &'static str,
    pub fog_end: &'static str,
    pub prop_disableselfshadowing: &'static str,
    pub ambient_fill: bool,
    pub ambient_fill_light: &'static str,
}

pub fn get_lighting_preset(style: &str) -> LightingPreset {
    match style.trim().to_lowercase().as_str() {
        "gmod" => LightingPreset {
            sky_name: "mc_sky_gmod",
            sun_angles: "0 220 0",
            sun_pitch: "-55",
            sun_light: "255 240 200 450",
            sun_ambient: "120 145 180 140",
            sun_color: "255 240 200",
            min_light: "0.0",
            fog_color: "178 215 255",
            fog_start: "4500",
            fog_end: "8500",
            prop_disableselfshadowing: "0",
            ambient_fill: false,
            ambient_fill_light: "0 0 0 0",
        },
        "bright" => LightingPreset {
            sky_name: "mc_sky_bright",
            sun_angles: "0 180 0",
            sun_pitch: "-85",
            sun_light: "255 255 245 480",
            sun_ambient: "230 240 255 450",
            sun_color: "255 255 245",
            min_light: "0.20",
            fog_color: "190 225 255",
            fog_start: "4500",
            fog_end: "8500",
            prop_disableselfshadowing: "1",
            ambient_fill: true,
            ambient_fill_light: "200 210 230 60",
        },
        "night" => LightingPreset {
            sky_name: "mc_sky_night",
            sun_angles: "0 45 0",
            sun_pitch: "-45",
            sun_light: "100 130 180 140",
            sun_ambient: "40 50 80 60",
            sun_color: "100 130 180",
            min_light: "0.04",
            fog_color: "20 30 50",
            fog_start: "3000",
            fog_end: "6500",
            prop_disableselfshadowing: "1",
            ambient_fill: true,
            ambient_fill_light: "60 70 100 20",
        },
        _ => LightingPreset {
            sky_name: "mc_sky_minecraft",
            sun_angles: "0 225 0",
            sun_pitch: "-55",
            sun_light: "255 248 228 310",
            sun_ambient: "205 220 245 270",
            sun_color: "255 248 228",
            min_light: "0.10",
            fog_color: "178 215 255",
            fog_start: "4500",
            fog_end: "8500",
            prop_disableselfshadowing: "0",
            ambient_fill: true,
            ambient_fill_light: "195 210 230 140",
        },
    }
}

pub struct VmfBuildParams<'a> {
    pub output_vmf: &'a Path,
    pub category: &'a str,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub chunks: &'a [CompiledChunkArtifact],
    pub all_solid_boxes: &'a [CollisionBox],
    pub ladder_boxes: &'a [[f64; 6]],
    pub water_pools: &'a [[f64; 6]],
    pub hazard_boxes: &'a [HazardBox],
    pub core_occluders: &'a [[f64; 6]],
    pub doors: &'a [InteractableDoor],
    pub buttons: &'a [InteractableButton],
    pub lights: &'a [BlockLight],
    pub spawn_points_source: &'a [[f64; 3]],
    pub lighting_style: &'a str,
    pub hq_light: bool,
    pub scale_factor: f64,
    pub force_vmf_brushes: bool,
}

fn sanitize_brush_material(material: &str, clean_cat: &str, fallback: &str) -> String {
    let trimmed = material.replace('\\', "/").trim().trim_matches('"').to_string();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("tools/") {
        return trimmed;
    }
    if lower.starts_with("concrete/") || lower.starts_with("nature/") {
        return trimmed;
    }
    let mut clean = if lower.starts_with("models/") {
        trimmed[7..].to_string()
    } else {
        trimmed
    };
    if !clean.contains('/') && !clean_cat.is_empty() {
        clean = format!("{}/{}", clean_cat, clean);
    }
    let clean_lower = clean.to_ascii_lowercase();
    if clean_lower.ends_with("_brush") || clean_lower.contains("door") {
        clean
    } else {
        format!("{}_brush", clean)
    }
}

fn vec_to_movedir_str(v: [f64; 3], face: &str, facing: &str) -> String {
    let [vx, vy, vz] = v;
    if vx.abs() > 1e-4 || vy.abs() > 1e-4 || vz.abs() > 1e-4 {
        if vz.abs() >= vx.abs().max(vy.abs()) && vz.abs() > 1e-4 {
            return if vz > 0.0 { "-90 0 0" } else { "90 0 0" }.to_string();
        }
        if vx.abs() >= vy.abs() {
            return if vx >= 0.0 { "0 0 0" } else { "0 180 0" }.to_string();
        }
        return if vy >= 0.0 { "0 90 0" } else { "0 270 0" }.to_string();
    }
    match face.to_ascii_lowercase().as_str() {
        "ceiling" => "-90 0 0".to_string(),
        "floor" => "90 0 0".to_string(),
        _ => match facing.to_ascii_lowercase().as_str() {
            "north" => "0 270 0".to_string(),
            "south" => "0 90 0".to_string(),
            "east" => "0 180 0".to_string(),
            "west" => "0 0 0".to_string(),
            _ => "0 0 0".to_string(),
        },
    }
}

pub(crate) struct VmfValidator {
    pub(crate) solid_count: usize,
    pub(crate) entity_count: usize,
    pub(crate) total_sides: usize,
    pub(crate) targetnames: HashSet<String>,
    pub(crate) io_targets: Vec<(String, String)>,
}

impl VmfValidator {
    pub(crate) fn new() -> Self {
        Self {
            solid_count: 0,
            entity_count: 0,
            total_sides: 0,
            targetnames: HashSet::new(),
            io_targets: Vec::new(),
        }
    }

    pub(crate) fn record_solid(&mut self, sides: usize) {
        self.solid_count += 1;
        self.total_sides += sides;
    }

    fn record_entity(
        &mut self,
        classname: &str,
        kv: &[(&str, String)],
        connections: &[(&str, String)],
    ) {
        self.entity_count += 1;
        for (k, v) in kv {
            if *k == "targetname" && !v.is_empty() {
                self.targetnames.insert(v.clone());
            }
        }
        for (_, cmd_val) in connections {
            if let Some(target) = cmd_val.split(',').next() {
                let t = target.trim();
                if !t.is_empty() {
                    self.io_targets.push((classname.to_string(), t.to_string()));
                }
            }
        }
    }

    pub(crate) fn into_problems(self) -> Vec<String> {
        let mut problems = Vec::new();
        for (src_cls, tgt) in self.io_targets {
            if !tgt.starts_with('!') && !self.targetnames.contains(&tgt) {
                problems.push(format!(
                    "Entity '{}' has I/O output to missing targetname '{}'",
                    src_cls, tgt
                ));
            }
        }
        if self.solid_count > 8192 {
            problems.push(format!(
                "Total brushes ({}) exceed Source Engine MAX_MAP_BRUSHES (8192)",
                self.solid_count
            ));
        }
        if self.entity_count > 8192 {
            problems.push(format!(
                "Total entities ({}) exceed Source Engine MAX_MAP_ENTITIES (8192)",
                self.entity_count
            ));
        }
        if self.total_sides > 65536 {
            problems.push(format!(
                "Total brush sides ({}) exceed Source Engine MAX_MAP_BRUSHSIDES (65536)",
                self.total_sides
            ));
        }
        problems
    }
}

#[cfg(test)]
pub fn build_vmf(params: &VmfBuildParams<'_>) -> Result<(), String> {
    build_vmf_checked(params).map(|_| ())
}

pub fn build_vmf_checked(params: &VmfBuildParams<'_>) -> Result<Vec<String>, String> {
    let eff_scale = if params.scale_factor > 0.0 {
        params.scale_factor
    } else {
        39.37
    };
    let s_ratio = eff_scale / 39.37;
    let preset = get_lighting_preset(params.lighting_style);
    let _pal = palette_for_style(params.lighting_style);
    let clean_cat = params.category.trim_matches(|c| c == '/' || c == '\\');

    let margin = 800.0;
    let thick = 64.0;
    // Stay strictly inside Source VBSP HashVec's (-16384, +16384) valid coordinate range
    let max_inner = 16256.0 - thick;

    let x_min = (params.bounds_min[0] - margin).floor().max(-max_inner);
    let x_max = (params.bounds_max[0] + margin).ceil().min(max_inner);
    let y_min = (params.bounds_min[1] - margin).floor().max(-max_inner);
    let y_max = (params.bounds_max[1] + margin).ceil().min(max_inner);
    let z_min = (params.bounds_min[2] - 200.0).floor().max(-max_inner);
    let z_max = (params.bounds_max[2] + 1500.0).ceil().min(max_inner);

    let sky_bounds = [x_min, x_max, y_min, y_max, z_min, z_max];
    let sky_inner_clamp = [x_min + 2.0, x_max - 2.0, y_min + 2.0, y_max - 2.0, z_min + 2.0, z_max - 2.0];

    let mut brush_id = 2usize;
    let mut side_id = 20usize;
    let mut ent_id = 200usize;
    let mut validator = VmfValidator::new();

    if let Some(parent) = params.output_vmf.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed creating VMF dir: {}", e))?;
    }
    let out_file = std::fs::File::create(params.output_vmf)
        .map_err(|e| format!("Failed creating VMF {}: {}", params.output_vmf.display(), e))?;
    let mut writer = BufWriter::with_capacity(256 * 1024, out_file);
    let io_err = |e: std::io::Error| format!("Failed writing VMF {}: {}", params.output_vmf.display(), e);

    writer
        .write_all(
            br#"versioninfo
{
	"editorversion" "400"
	"editorbuild" "8864"
	"mapversion" "1"
	"formatversion" "100"
	"prefab" "0"
}
visgroups
{
	visgroup
	{
		"name" "Skybox"
		"visgroupid" "1"
		"color" "0 180 255"
	}
	visgroup
	{
		"name" "Collision"
		"visgroupid" "2"
		"color" "255 0 255"
	}
	visgroup
	{
		"name" "Ladders"
		"visgroupid" "3"
		"color" "255 128 0"
	}
	visgroup
	{
		"name" "Interactables"
		"visgroupid" "4"
		"color" "0 220 100"
	}
	visgroup
	{
		"name" "Water"
		"visgroupid" "5"
		"color" "0 128 255"
	}
	visgroup
	{
		"name" "VVIS"
		"visgroupid" "6"
		"color" "255 200 0"
	}
	visgroup
	{
		"name" "Hazards"
		"visgroupid" "7"
		"color" "255 60 0"
	}
}
viewsettings
{
	"bShowGrid" "1"
	"nGridSpacing" "64"
	"bShowLogicalGrid" "0"
	"nLogicalGridSpacing" "64"
	"bShow3DGrid" "0"
}
"#,
        )
        .map_err(io_err)?;
    writeln!(
        writer,
        "world\n{{\n\t\"id\" \"1\"\n\t\"mapversion\" \"1\"\n\t\"classname\" \"worldspawn\"\n\t\"detailmaterial\" \"detail/detailsprites\"\n\t\"detailvbsp\" \"detail.vbsp\"\n\t\"maxrange\" \"32768\"\n\t\"_minlight\" \"{}\"\n\t\"skyname\" \"{}\"",
        preset.min_light, preset.sky_name
    )
    .map_err(io_err)?;

    let mut next_brush = || {
        let id = brush_id;
        brush_id += 1;
        id
    };
    let mut next_ent = || {
        let id = ent_id;
        ent_id += 1;
        id
    };
    let mut solid_buf: Vec<u8> = Vec::with_capacity(64 * 1024);

    // 1. Sealed skybox enclosure (ceiling + nodraw floor + split perimeter walls)
    let mut world_brush_count = 0usize;
    let sky_lo = z_min + 1.0;
    let sky_hi = (z_max - 1.0).max(sky_lo);
    let sky_floor_z = params.bounds_min[2].clamp(sky_lo, sky_hi);
    let mut sky_brushes: Vec<([f64; 6], &[&str; 6])> = Vec::with_capacity(10);
    // Ceiling
    sky_brushes.push((
        [x_min - thick, y_min - thick, z_max, x_max + thick, y_max + thick, z_max + thick],
        &["TOOLS/TOOLSSKYBOX"; 6],
    ));
    // Floor (nodraw so VRAD++ never shines upward sky ambient through the bottom of dark rooms)
    sky_brushes.push((
        [x_min - thick, y_min - thick, z_min - thick, x_max + thick, y_max + thick, z_min],
        &["TOOLS/TOOLSNODRAW"; 6],
    ));
    // 4 perimeter walls split vertically at sky_floor_z: nodraw skirt below, skybox above
    let perimeter_xy = [
        [x_min - thick, y_min - thick, x_min, y_max + thick], // west
        [x_max, y_min - thick, x_max + thick, y_max + thick], // east
        [x_min, y_min - thick, x_max, y_min],                 // south
        [x_min, y_max, x_max, y_max + thick],                 // north
    ];
    for [wx1, wy1, wx2, wy2] in perimeter_xy {
        sky_brushes.push(([wx1, wy1, z_min, wx2, wy2, sky_floor_z], &["TOOLS/TOOLSNODRAW"; 6]));
        sky_brushes.push(([wx1, wy1, sky_floor_z, wx2, wy2, z_max], &["TOOLS/TOOLSSKYBOX"; 6]));
    }
    for (w, mats) in sky_brushes {
        write_box_brush(
            &mut writer,
            next_brush(),
            w[0],
            w[1],
            w[2],
            w[3],
            w[4],
            w[5],
            mats,
            &mut side_id,
            Some(1),
            "0 180 255",
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;
        world_brush_count += 1;
    }

    // 2. Water worldspawn brushes (animated Minecraft LightmappedGeneric %compilewater)
    const MAX_VMF_WATER_BRUSHES: usize = 1024;
    const MAX_VMF_CORE_OCCLUDERS: usize = 48;
    let aabb_volume = |b: &[f64; 6]| -> f64 {
        (b[3] - b[0]).abs() * (b[4] - b[1]).abs() * (b[5] - b[2]).abs()
    };

    let water_top_mat = format!("{}/mc_water", clean_cat);
    let water_uv_scale = eff_scale / 64.0;
    let mut water_brush_count = 0usize;
    let mut max_water_z: Option<f64> = None;
    let mut water_indices: Vec<usize> = (0..params.water_pools.len()).collect();
    if water_indices.len() > MAX_VMF_WATER_BRUSHES {
        water_indices.sort_by(|&ia, &ib| {
            let a = &params.water_pools[ia];
            let b = &params.water_pools[ib];
            aabb_volume(b)
                .total_cmp(&aabb_volume(a))
                .then_with(|| b[2].max(b[5]).total_cmp(&a[2].max(a[5])))
                .then_with(|| ia.cmp(&ib))
        });
        water_indices.truncate(MAX_VMF_WATER_BRUSHES);
    }
    for idx in water_indices {
        let wp = params.water_pools[idx];
        if let Some(wc) = clamp_box_coords(wp, sky_inner_clamp, 1.0) {
            write_box_brush_with_uv(
                &mut writer,
                next_brush(),
                wc[0],
                wc[1],
                wc[2],
                wc[3],
                wc[4],
                wc[5],
                &[
                    water_top_mat.as_str(),
                    "TOOLS/TOOLSNODRAW",
                    "TOOLS/TOOLSNODRAW",
                    "TOOLS/TOOLSNODRAW",
                    "TOOLS/TOOLSNODRAW",
                    "TOOLS/TOOLSNODRAW",
                ],
                BrushUvMode::WorldAligned(water_uv_scale),
                &mut side_id,
                Some(5),
                "0 128 255",
                &mut validator,
            )
            .map_err(io_err)?;
            writer.write_all(b"\n").map_err(io_err)?;
            world_brush_count += 1;
            water_brush_count += 1;
            max_water_z = Some(max_water_z.map(|v| v.max(wc[5])).unwrap_or(wc[5]));
        }
    }

    // 3. Buried core occluders (largest volumes first, capped to avoid VVIS portal bloat)
    let mut occ_indices: Vec<usize> = (0..params.core_occluders.len()).collect();
    occ_indices.sort_by(|&ia, &ib| {
        aabb_volume(&params.core_occluders[ib])
            .total_cmp(&aabb_volume(&params.core_occluders[ia]))
            .then_with(|| ia.cmp(&ib))
    });
    occ_indices.truncate(MAX_VMF_CORE_OCCLUDERS);
    for idx in occ_indices {
        let oc = params.core_occluders[idx];
        if let Some(occ) = clamp_box_coords(oc, sky_inner_clamp, 4.0) {
            write_box_brush(
                &mut writer,
                next_brush(),
                occ[0],
                occ[1],
                occ[2],
                occ[3],
                occ[4],
                occ[5],
                &["TOOLS/TOOLSNODRAW"; 6],
                &mut side_id,
                Some(6),
                "180 180 0",
                &mut validator,
            )
            .map_err(io_err)?;
            writer.write_all(b"\n").map_err(io_err)?;
            world_brush_count += 1;
        }
    }

    // 4. VVIS horizontal sky separation HINT/SKIP + vertical chunk-grid HINT/SKIP planes
    let surface_top_z = params
        .bounds_max[2]
        .max(max_water_z.unwrap_or(params.bounds_max[2]));
    let sky_vis_z_min = (z_min + 32.0).max(surface_top_z + 128.0);
    let sky_vis_z_max = z_max - 16.0;
    let mut hint_z_ceil = z_max - 16.0;

    if (sky_vis_z_max - sky_vis_z_min) >= 32.0
        && (x_max - x_min) >= 64.0
        && (y_max - y_min) >= 64.0
        && (sky_vis_z_min - 8.0) > (z_min + 8.0)
    {
        write_box_brush(
            &mut writer,
            next_brush(),
            x_min + 8.0,
            y_min + 8.0,
            sky_vis_z_min - 8.0,
            x_max - 8.0,
            y_max - 8.0,
            sky_vis_z_min - 4.0,
            &[
                "TOOLS/TOOLSHINT",
                "TOOLS/TOOLSSKIP",
                "TOOLS/TOOLSSKIP",
                "TOOLS/TOOLSSKIP",
                "TOOLS/TOOLSSKIP",
                "TOOLS/TOOLSSKIP",
            ],
            &mut side_id,
            Some(6),
            "255 200 0",
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;
        world_brush_count += 1;
        hint_z_ceil = sky_vis_z_min - 8.0;
    }

    let world_dx_span = (params.bounds_max[0] - params.bounds_min[0]).max(0.0);
    let world_dy_span = (params.bounds_max[1] - params.bounds_min[1]).max(0.0);
    if (hint_z_ceil - (z_min + 8.0)) >= 32.0 && (y_max - y_min) >= 64.0 && (x_max - x_min) >= 64.0 {
        let base_hint_step = (32.0 * eff_scale).max(1024.0);
        if world_dx_span >= 1536.0 {
            let raw_cuts = (world_dx_span / base_hint_step).floor() as isize - 1;
            let x_cuts_count = raw_cuts.clamp(1, 5) as usize;
            let x_cut_step = world_dx_span / (x_cuts_count + 1) as f64;
            for idx in 1..=x_cuts_count {
                let cx = params.bounds_min[0] + (idx as f64) * x_cut_step;
                if cx > (x_min + 32.0) && cx < (x_max - 32.0) {
                    write_box_brush(
                        &mut writer,
                        next_brush(),
                        cx - 2.0,
                        y_min + 8.0,
                        z_min + 8.0,
                        cx + 2.0,
                        y_max - 8.0,
                        hint_z_ceil,
                        &[
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSHINT", // right (+X)
                        ],
                        &mut side_id,
                        Some(6),
                        "255 200 0",
                        &mut validator,
                    )
                    .map_err(io_err)?;
                    writer.write_all(b"\n").map_err(io_err)?;
                    world_brush_count += 1;
                }
            }
        }
        if world_dy_span >= 1536.0 {
            let raw_cuts = (world_dy_span / base_hint_step).floor() as isize - 1;
            let y_cuts_count = raw_cuts.clamp(1, 5) as usize;
            let y_cut_step = world_dy_span / (y_cuts_count + 1) as f64;
            for idx in 1..=y_cuts_count {
                let cy = params.bounds_min[1] + (idx as f64) * y_cut_step;
                if cy > (y_min + 32.0) && cy < (y_max - 32.0) {
                    write_box_brush(
                        &mut writer,
                        next_brush(),
                        x_min + 8.0,
                        cy - 2.0,
                        z_min + 8.0,
                        x_max - 8.0,
                        cy + 2.0,
                        hint_z_ceil,
                        &[
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSHINT", // back (+Y)
                            "TOOLS/TOOLSSKIP",
                            "TOOLS/TOOLSSKIP",
                        ],
                        &mut side_id,
                        Some(6),
                        "255 200 0",
                        &mut validator,
                    )
                    .map_err(io_err)?;
                    writer.write_all(b"\n").map_err(io_err)?;
                    world_brush_count += 1;
                }
            }
        }
    }

    writer.write_all(b"}\n").map_err(io_err)?;

    // 5. Hazard trigger_hurt volumes
    let mut hazard_count = 0usize;
    for hb in params.hazard_boxes.iter().take(512) {
        if let Some(hc) = clamp_box_coords(hb.box_coords, sky_inner_clamp, 0.5) {
            solid_buf.clear();
            write_box_brush(
                &mut solid_buf,
                next_brush(),
                hc[0],
                hc[1],
                hc[2],
                hc[3],
                hc[4],
                hc[5],
                &["TOOLS/TOOLSTRIGGER"; 6],
                &mut side_id,
                Some(7),
                "255 60 0",
                &mut validator,
            )
            .map_err(io_err)?;
            write_entity_bytes(
                &mut writer,
                "trigger_hurt",
                &[
                    ("id", next_ent().to_string()),
                    ("damage", hb.damage.to_string()),
                    ("damagecap", hb.damage.to_string()),
                    ("damagemodel", "0".to_string()),
                    ("damagetype", hb.damage_type.to_string()),
                    ("nodmgforce", "1".to_string()),
                    ("spawnflags", "1".to_string()),
                    ("StartDisabled", "0".to_string()),
                    (
                        "origin",
                        format!(
                            "{:.1} {:.1} {:.1}",
                            (hc[0] + hc[3]) * 0.5,
                            (hc[1] + hc[4]) * 0.5,
                            (hc[2] + hc[5]) * 0.5
                        ),
                    ),
                ],
                &[],
                &solid_buf,
                &mut validator,
            )
            .map_err(io_err)?;
            writer.write_all(b"\n").map_err(io_err)?;
            hazard_count += 1;
        }
    }

    // 6. func_detail batches for collision brushes (overflow + fallback chunks + forced VMF brushes) & ladders
    // Note: info_node entities are intentionally omitted so GMod does not trigger
    // "Node Graph out of Date. Rebuilding..." on load without a prebuilt .ain.
    let other_brushes = world_brush_count
        + hazard_count
        + params.ladder_boxes.len()
        + params.doors.len()
        + params.buttons.len()
        + 8;
    let max_collision_brushes = 8192usize.saturating_sub(other_brushes);

    let mut batch_count = 0usize;
    let flush_func_detail_batch = |writer: &mut BufWriter<std::fs::File>,
                                   solid_buf: &mut Vec<u8>,
                                   batch_count: &mut usize,
                                   next_ent: &mut dyn FnMut() -> usize,
                                   validator: &mut VmfValidator|
     -> Result<(), String> {
        if *batch_count > 0 {
            write_entity_bytes(
                writer,
                "func_detail",
                &[("id", next_ent().to_string())],
                &[],
                solid_buf,
                validator,
            )
            .map_err(io_err)?;
            writer.write_all(b"\n").map_err(io_err)?;
            solid_buf.clear();
            *batch_count = 0;
        }
        Ok(())
    };

    let clean_cat = params.category.trim_matches(|c| c == '/' || c == '\\');
    if params.force_vmf_brushes || params.chunks.is_empty() {
        let mut sorted_boxes: Vec<&CollisionBox> = params.all_solid_boxes.iter().collect();
        if sorted_boxes.len() > max_collision_brushes {
            sorted_boxes.sort_by(|a, b| {
                let za = a.box_coords[2].max(a.box_coords[5]);
                let zb = b.box_coords[2].max(b.box_coords[5]);
                zb.total_cmp(&za)
            });
            sorted_boxes.truncate(max_collision_brushes);
        }
        solid_buf.clear();
        for cb in sorted_boxes {
            if let Some(c) = clamp_box_coords(cb.box_coords, sky_inner_clamp, 0.5) {
                let col_mat = format!("{}/col_{}", clean_cat, cb.surface_prop.as_source_str());
                write_box_brush(
                    &mut solid_buf,
                    next_brush(),
                    c[0],
                    c[1],
                    c[2],
                    c[3],
                    c[4],
                    c[5],
                    &[col_mat.as_str(); 6],
                    &mut side_id,
                    Some(2),
                    "255 0 255",
                    &mut validator,
                )
                .map_err(io_err)?;
                solid_buf.push(b'\n');
                batch_count += 1;
                if batch_count >= 300 {
                    flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;
                }
            }
        }
        flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;
    } else {
        // Batch overflow/fallback boxes per chunk for spatial locality, enforcing budget
        let total_overflow: usize = params.chunks.iter().map(|ch| ch.overflow_boxes.len()).sum();
        if total_overflow <= max_collision_brushes {
            for ch in params.chunks {
                if ch.overflow_boxes.is_empty() {
                    continue;
                }
                solid_buf.clear();
                batch_count = 0;
                for cb in &ch.overflow_boxes {
                    if let Some(c) = clamp_box_coords(cb.box_coords, sky_inner_clamp, 0.5) {
                        let col_mat = format!("{}/col_{}", clean_cat, cb.surface_prop.as_source_str());
                        write_box_brush(
                            &mut solid_buf,
                            next_brush(),
                            c[0],
                            c[1],
                            c[2],
                            c[3],
                            c[4],
                            c[5],
                            &[col_mat.as_str(); 6],
                            &mut side_id,
                            Some(2),
                            "255 0 255",
                            &mut validator,
                        )
                        .map_err(io_err)?;
                        solid_buf.push(b'\n');
                        batch_count += 1;
                        if batch_count >= 300 {
                            flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;
                        }
                    }
                }
                flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;
            }
        } else {
            let mut prioritized: Vec<(usize, &CollisionBox)> = params
                .chunks
                .iter()
                .enumerate()
                .flat_map(|(c_idx, ch)| ch.overflow_boxes.iter().map(move |cb| (c_idx, cb)))
                .collect();
            prioritized.sort_by(|(_, a), (_, b)| {
                let score = |cb: &CollisionBox| -> (u8, i64, i64) {
                    let walk = if cb.is_walkable { 1 } else { 0 };
                    let dx = (cb.box_coords[3] - cb.box_coords[0]).abs();
                    let dy = (cb.box_coords[4] - cb.box_coords[1]).abs();
                    let area = ((dx * dy) * 10.0) as i64;
                    let z_top = (cb.box_coords[2].max(cb.box_coords[5]) * 10.0) as i64;
                    (walk, area, z_top)
                };
                score(b).cmp(&score(a))
            });
            prioritized.truncate(max_collision_brushes);
            prioritized.sort_by_key(|(c_idx, _)| *c_idx);

            let mut cur_chunk: Option<usize> = None;
            solid_buf.clear();
            batch_count = 0;
            for (c_idx, cb) in prioritized {
                if cur_chunk != Some(c_idx) {
                    flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;
                    cur_chunk = Some(c_idx);
                }
                if let Some(c) = clamp_box_coords(cb.box_coords, sky_inner_clamp, 0.5) {
                    let col_mat = format!("{}/col_{}", clean_cat, cb.surface_prop.as_source_str());
                    write_box_brush(
                        &mut solid_buf,
                        next_brush(),
                        c[0],
                        c[1],
                        c[2],
                        c[3],
                        c[4],
                        c[5],
                        &[col_mat.as_str(); 6],
                        &mut side_id,
                        Some(2),
                        "255 0 255",
                        &mut validator,
                    )
                    .map_err(io_err)?;
                    solid_buf.push(b'\n');
                    batch_count += 1;
                    if batch_count >= 300 {
                        flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;
                    }
                }
            }
            flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;
        }
    }

    solid_buf.clear();
    batch_count = 0;
    for lb in params.ladder_boxes {
        if let Some(c) = clamp_box_coords(*lb, sky_inner_clamp, 0.5) {
            write_box_brush(
                &mut solid_buf,
                next_brush(),
                c[0],
                c[1],
                c[2],
                c[3],
                c[4],
                c[5],
                &["TOOLS/TOOLSINVISIBLELADDER"; 6],
                &mut side_id,
                Some(3),
                "255 128 0",
                &mut validator,
            )
            .map_err(io_err)?;
            solid_buf.push(b'\n');
            batch_count += 1;
            if batch_count >= 300 {
                flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;
            }
        }
    }
    flush_func_detail_batch(&mut writer, &mut solid_buf, &mut batch_count, &mut next_ent, &mut validator)?;

    // 7. prop_static chunks
    let dx = (params.bounds_max[0] - params.bounds_min[0]).max(0.0);
    let dy = (params.bounds_max[1] - params.bounds_min[1]).max(0.0);
    let dz = (params.bounds_max[2] - params.bounds_min[2]).max(0.0);
    let diag = (dx * dx + dy * dy + dz * dz).sqrt();
    let fog_end_i32 = preset.fog_end.parse::<i32>().unwrap_or(8500);
    let max_farz = (fog_end_i32 + 512).max(6000);
    let farz = ((diag * 1.1).round() as i32).clamp(6000, max_farz);
    let use_chunk_fade = dx.max(dy) > fog_end_i32 as f64;
    let prop_fade_min = if use_chunk_fade {
        (fog_end_i32 + (1400.0 * s_ratio).round() as i32).to_string()
    } else {
        "-1".to_string()
    };
    let prop_fade_max = if use_chunk_fade {
        (fog_end_i32 + (2200.0 * s_ratio).round() as i32).to_string()
    } else {
        "0".to_string()
    };

    let self_shadow = if params.hq_light {
        "0"
    } else {
        preset.prop_disableselfshadowing
    };
    let fade_min = ((2500.0 * s_ratio).round() as i32).max(1800).to_string();
    let fade_max = ((3600.0 * s_ratio).round() as i32).max(2600).to_string();

    for ch in params.chunks {
        if ch.model_rel_path.is_empty() {
            continue;
        }
        let c_solid = if params.force_vmf_brushes || ch.fallback_vmf_brushes {
            "0"
        } else {
            "6"
        };
        let orig_str = format!(
            "{:.2} {:.2} {:.2}",
            ch.origin_source[0], ch.origin_source[1], ch.origin_source[2]
        );
        write_entity_bytes(
            &mut writer,
            "prop_static",
            &[
                ("id", next_ent().to_string()),
                ("angles", "0 0 0".to_string()),
                ("disableselfshadowing", self_shadow.to_string()),
                ("disableshadows", "0".to_string()),
                ("disablevertexlighting", "0".to_string()),
                ("fademaxdist", prop_fade_max.clone()),
                ("fademindist", prop_fade_min.clone()),
                ("fadescale", "1".to_string()),
                ("ignorenormals", "0".to_string()),
                ("_minlight", preset.min_light.to_string()),
                ("model", ch.model_rel_path.clone()),
                ("origin", orig_str.clone()),
                ("renderamt", "255".to_string()),
                ("rendercolor", "255 255 255".to_string()),
                ("screenspacefade", "0".to_string()),
                ("skin", "0".to_string()),
                ("solid", c_solid.to_string()),
            ],
            &[],
            &[],
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;

        if let Some(ref t_mdl) = ch.translucent_model_rel_path {
            write_entity_bytes(
                &mut writer,
                "prop_static",
                &[
                    ("id", next_ent().to_string()),
                    ("angles", "0 0 0".to_string()),
                    ("disableselfshadowing", "1".to_string()),
                    ("disableshadows", "1".to_string()),
                    ("disablevertexlighting", "0".to_string()),
                    ("fademaxdist", fade_max.clone()),
                    ("fademindist", fade_min.clone()),
                    ("fadescale", "1".to_string()),
                    ("ignorenormals", "0".to_string()),
                    ("_minlight", preset.min_light.to_string()),
                    ("model", t_mdl.clone()),
                    ("origin", orig_str.clone()),
                    ("renderamt", "255".to_string()),
                    ("rendercolor", "255 255 255".to_string()),
                    ("screenspacefade", "0".to_string()),
                    ("skin", "0".to_string()),
                    ("solid", "0".to_string()),
                ],
                &[],
                &[],
                &mut validator,
            )
            .map_err(io_err)?;
            writer.write_all(b"\n").map_err(io_err)?;
        }

        if !params.force_vmf_brushes {
            for (_, col_mdl) in &ch.extra_collision_models {
                if col_mdl.is_empty() {
                    continue;
                }
                write_entity_bytes(
                    &mut writer,
                    "prop_static",
                    &[
                        ("id", next_ent().to_string()),
                        ("angles", "0 0 0".to_string()),
                        ("disableselfshadowing", "1".to_string()),
                        ("disableshadows", "1".to_string()),
                        ("disablevertexlighting", "1".to_string()),
                        ("fademaxdist", "0".to_string()),
                        ("fademindist", "-1".to_string()),
                        ("fadescale", "1".to_string()),
                        ("ignorenormals", "1".to_string()),
                        ("model", col_mdl.clone()),
                        ("origin", orig_str.clone()),
                        ("renderamt", "0".to_string()),
                        ("rendermode", "10".to_string()),
                        ("rendercolor", "255 255 255".to_string()),
                        ("screenspacefade", "0".to_string()),
                        ("skin", "0".to_string()),
                        ("solid", "6".to_string()),
                    ],
                    &[],
                    &[],
                    &mut validator,
                )
                .map_err(io_err)?;
                writer.write_all(b"\n").map_err(io_err)?;
            }
        }
    }

    // 8. Interactive Doors (func_door_rotating), Fence Gates, Buttons & Levers with VMF I/O connections
    let uv_scale = eff_scale / 16.0; // 2.460625 at 39.37
    let interact_minlight = match params.lighting_style.to_ascii_lowercase().as_str() {
        "night" => "0.08",
        "minecraft" => "0.30",
        _ => "0.18",
    };

    for (idx, door) in params.doors.iter().enumerate() {
        let raw_mat = if door.mat_name.contains('/')
            || door.mat_name.to_ascii_uppercase().starts_with("TOOLS/")
            || clean_cat.is_empty()
        {
            door.mat_name.clone()
        } else {
            format!("{}/{}", clean_cat, door.mat_name)
        };
        let door_mat = sanitize_brush_material(&raw_mat, clean_cat, "CONCRETE/CONCRETEFLOOR001A");

        let x_min_d = door.box_coords[0].min(door.box_coords[3]);
        let x_max_d = door.box_coords[0].max(door.box_coords[3]);
        let y_min_d = door.box_coords[1].min(door.box_coords[4]);
        let y_max_d = door.box_coords[1].max(door.box_coords[4]);
        let z_min_d = door.box_coords[2].min(door.box_coords[5]);
        let z_max_d = door.box_coords[2].max(door.box_coords[5]);

        let span_x = x_max_d - x_min_d;
        let span_y = y_max_d - y_min_d;
        let clearance = 0.25;
        let (dx_min, dx_max, dy_min, dy_max) = if span_x >= span_y {
            (
                if span_x > 1.0 { x_min_d + clearance } else { x_min_d },
                if span_x > 1.0 { x_max_d - clearance } else { x_max_d },
                y_min_d,
                y_max_d,
            )
        } else {
            (
                x_min_d,
                x_max_d,
                if span_y > 1.0 { y_min_d + clearance } else { y_min_d },
                if span_y > 1.0 { y_max_d - clearance } else { y_max_d },
            )
        };
        let dz_min = if (z_max_d - z_min_d) > 1.0 { z_min_d + clearance } else { z_min_d };
        let dz_max = if (z_max_d - z_min_d) > 1.0 { z_max_d - clearance } else { z_max_d };

        let short_lower = door.short_type.to_ascii_lowercase();
        let mat_lower = door.mat_name.to_ascii_lowercase();
        solid_buf.clear();
        if short_lower.contains("fence_gate") {
            write_box_brush_scaled(
                &mut solid_buf,
                next_brush(),
                dx_min,
                dy_min,
                dz_min,
                dx_max,
                dy_max,
                dz_max,
                &[door_mat.as_str(); 6],
                uv_scale,
                &mut side_id,
                Some(4),
                "0 220 100",
                &mut validator,
            )
            .map_err(io_err)?;
        } else {
            write_door_brush(
                &mut solid_buf,
                next_brush(),
                [dx_min, dy_min, dz_min, dx_max, dy_max, dz_max],
                [x_min_d, y_min_d, z_min_d, x_max_d, y_max_d, z_max_d],
                &door_mat,
                uv_scale,
                &mut side_id,
                &mut validator,
            )
            .map_err(io_err)?;
        }
        solid_buf.push(b'\n');

        let is_iron = short_lower.contains("iron") || mat_lower.contains("iron");
        let spawnflags = if is_iron {
            door.spawnflags & !256
        } else {
            door.spawnflags
        };
        let (noise1, noise2) = if is_iron {
            (
                "doors/door_metal_medium_open1.wav",
                "doors/door_metal_medium_close1.wav",
            )
        } else {
            ("doors/door_wood_move1.wav", "doors/door_wood_close1.wav")
        };

        let targetname = format!("mc_door_{}_{}", idx, door.short_type);
        write_entity_bytes(
            &mut writer,
            "func_door_rotating",
            &[
                ("id", next_ent().to_string()),
                ("targetname", targetname),
                ("angles", "0 0 0".to_string()),
                ("speed", "350".to_string()),
                ("distance", "90".to_string()),
                ("wait", "-1".to_string()),
                ("spawnflags", spawnflags.to_string()),
                (
                    "origin",
                    format!("{:.2} {:.2} {:.2}", door.origin[0], door.origin[1], door.origin[2]),
                ),
                ("noise1", noise1.to_string()),
                ("noise2", noise2.to_string()),
                ("_minlight", interact_minlight.to_string()),
                ("disableshadows", "1".to_string()),
                ("vrad_brush_cast_shadows", "0".to_string()),
            ],
            &[],
            &solid_buf,
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;
    }

    for (idx, btn) in params.buttons.iter().enumerate() {
        let bx0 = btn.box_coords[0].min(btn.box_coords[3]);
        let bx1 = btn.box_coords[0].max(btn.box_coords[3]);
        let by0 = btn.box_coords[1].min(btn.box_coords[4]);
        let by1 = btn.box_coords[1].max(btn.box_coords[4]);
        let bz0 = btn.box_coords[2].min(btn.box_coords[5]);
        let bz1 = btn.box_coords[2].max(btn.box_coords[5]);

        let c = 0.25;
        let ibx0 = if (bx1 - bx0) > 1.0 { bx0 + c } else { bx0 };
        let ibx1 = if (bx1 - bx0) > 1.0 { bx1 - c } else { bx1 };
        let iby0 = if (by1 - by0) > 1.0 { by0 + c } else { by0 };
        let iby1 = if (by1 - by0) > 1.0 { by1 - c } else { by1 };
        let ibz0 = if (bz1 - bz0) > 1.0 { bz0 + c } else { bz0 };
        let ibz1 = if (bz1 - bz0) > 1.0 { bz1 - c } else { bz1 };

        let b_mat = sanitize_brush_material(&btn.material, clean_cat, "CONCRETE/CONCRETEFLOOR001A");
        solid_buf.clear();
        write_box_brush_scaled(
            &mut solid_buf,
            next_brush(),
            ibx0,
            iby0,
            ibz0,
            ibx1,
            iby1,
            ibz1,
            &[b_mat.as_str(); 6],
            uv_scale,
            &mut side_id,
            Some(4),
            "0 220 100",
            &mut validator,
        )
        .map_err(io_err)?;
        solid_buf.push(b'\n');
        let orig = format!("{:.2} {:.2} {:.2}", btn.origin[0], btn.origin[1], btn.origin[2]);
        if btn.kind == "lever" {
            let mut conns = Vec::with_capacity(btn.target_ids.len() * 2);
            for t in &btn.target_ids {
                conns.push(("OnIn", format!("{},Open,,0.0,-1", t)));
                conns.push(("OnOut", format!("{},Close,,0.0,-1", t)));
            }
            write_entity_bytes(
                &mut writer,
                "func_rot_button",
                &[
                    ("id", next_ent().to_string()),
                    ("targetname", format!("mc_lever_{}", idx)),
                    ("speed", "200".to_string()),
                    ("distance", "60".to_string()),
                    ("wait", "-1".to_string()),
                    ("spawnflags", "1057".to_string()),
                    ("sounds", "Buttons.snd1".to_string()),
                    ("origin", orig),
                    ("_minlight", interact_minlight.to_string()),
                    ("disableshadows", "1".to_string()),
                    ("vrad_brush_cast_shadows", "0".to_string()),
                ],
                &conns,
                &solid_buf,
                &mut validator,
            )
            .map_err(io_err)?;
            writer.write_all(b"\n").map_err(io_err)?;
        } else {
            let mut conns = Vec::with_capacity(btn.target_ids.len() * 2);
            for t in &btn.target_ids {
                conns.push(("OnPressed", format!("{},Toggle,,0.0,-1", t)));
                conns.push(("OnOut", format!("{},Toggle,,0.0,-1", t)));
            }
            let movedir = vec_to_movedir_str(btn.move_dir, &btn.face, &btn.facing);
            write_entity_bytes(
                &mut writer,
                "func_button",
                &[
                    ("id", next_ent().to_string()),
                    ("targetname", format!("mc_btn_{}", idx)),
                    ("movedir", movedir),
                    ("speed", "100".to_string()),
                    ("lip", "2".to_string()),
                    ("wait", "1.0".to_string()),
                    ("spawnflags", "1024".to_string()),
                    ("sounds", "Buttons.snd1".to_string()),
                    ("origin", orig),
                    ("_minlight", interact_minlight.to_string()),
                    ("disableshadows", "1".to_string()),
                    ("vrad_brush_cast_shadows", "0".to_string()),
                ],
                &conns,
                &solid_buf,
                &mut validator,
            )
            .map_err(io_err)?;
            writer.write_all(b"\n").map_err(io_err)?;
        }
    }

    // 9. Resolve safe spawn points above solid walkable surfaces
    let spawns = resolve_spawn_points(
        params.spawn_points_source,
        params.bounds_min,
        params.bounds_max,
        sky_bounds,
        params.all_solid_boxes,
        params.water_pools,
        params.hazard_boxes,
        eff_scale,
    );
    for (i, sp) in spawns.iter().enumerate() {
        let eid = if i == 0 { 100 } else { next_ent() };
        write_entity_bytes(
            &mut writer,
            "info_player_start",
            &[
                ("id", eid.to_string()),
                ("angles", "0 90 0".to_string()),
                ("origin", format!("{:.1} {:.1} {:.1}", sp[0], sp[1], sp[2])),
            ],
            &[],
            &[],
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;
    }

    // 10. Lighting + env_fog_controller + HDR tonemapper + water_lod_control + ambient_generic + env_cubemap + cavity fill lights
    let sun_z = z_max - 200.0;
    write_entity_bytes(
        &mut writer,
        "light_environment",
        &[
            ("id", "102".to_string()),
            ("_ambient", preset.sun_ambient.to_string()),
            ("_ambientHDR", "-1 -1 -1 1".to_string()),
            ("_AmbientScaleHDR", "1".to_string()),
            ("_light", preset.sun_light.to_string()),
            ("_lightHDR", "-1 -1 -1 1".to_string()),
            ("_lightscaleHDR", "1".to_string()),
            ("angles", preset.sun_angles.to_string()),
            ("pitch", preset.sun_pitch.to_string()),
            ("SunSpreadAngle", "5".to_string()),
            ("origin", format!("0 0 {:.1}", sun_z)),
        ],
        &[],
        &[],
        &mut validator,
    )
    .map_err(io_err)?;
    writer.write_all(b"\n").map_err(io_err)?;
    if !params.lighting_style.eq_ignore_ascii_case("night") {
        write_entity_bytes(
            &mut writer,
            "env_sun",
            &[
                ("id", "103".to_string()),
                ("angles", preset.sun_angles.to_string()),
                ("pitch", preset.sun_pitch.to_string()),
                ("rendercolor", preset.sun_color.to_string()),
                ("size", "16".to_string()),
                ("use_angles", "1".to_string()),
                ("origin", format!("0 0 {:.1}", sun_z)),
            ],
            &[],
            &[],
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;
    }

    write_entity_bytes(
        &mut writer,
        "env_fog_controller",
        &[
            ("id", next_ent().to_string()),
            ("angles", "0 0 0".to_string()),
            ("farz", farz.to_string()),
            ("fogcolor", preset.fog_color.to_string()),
            ("fogcolor2", preset.fog_color.to_string()),
            ("fogdir", "1 0 0".to_string()),
            ("fogenable", "1".to_string()),
            ("fogend", preset.fog_end.to_string()),
            ("fogmaxdensity", "1".to_string()),
            ("fogstart", preset.fog_start.to_string()),
            ("origin", format!("0 0 {:.1}", sun_z)),
            ("spawnflags", "0".to_string()),
            ("use_angles", "0".to_string()),
        ],
        &[],
        &[],
        &mut validator,
    )
    .map_err(io_err)?;
    writer.write_all(b"\n").map_err(io_err)?;

    // HDR Tonemap Controller + logic_auto exposure clamping
    write_entity_bytes(
        &mut writer,
        "env_tonemap_controller",
        &[
            ("id", next_ent().to_string()),
            ("targetname", "tonemap_ctrl".to_string()),
            ("origin", format!("0 0 {:.1}", sun_z)),
        ],
        &[],
        &[],
        &mut validator,
    )
    .map_err(io_err)?;
    writer.write_all(b"\n").map_err(io_err)?;
    write_entity_bytes(
        &mut writer,
        "logic_auto",
        &[
            ("id", next_ent().to_string()),
            ("spawnflags", "1".to_string()),
            ("origin", format!("0 0 {:.1}", sun_z)),
        ],
        &[
            ("OnMapSpawn", "tonemap_ctrl,SetAutoExposureMin,0.75,0,-1".to_string()),
            ("OnMapSpawn", "tonemap_ctrl,SetAutoExposureMax,1.05,0,-1".to_string()),
            ("OnMapSpawn", "tonemap_ctrl,SetBloomScale,0.12,0,-1".to_string()),
        ],
        &[],
        &mut validator,
    )
    .map_err(io_err)?;
    writer.write_all(b"\n").map_err(io_err)?;

    // Water LOD Control (when water brushes exist)
    if water_brush_count > 0 {
        let mid_wx = (params.bounds_min[0] + params.bounds_max[0]) * 0.5;
        let mid_wy = (params.bounds_min[1] + params.bounds_max[1]) * 0.5;
        let lod_z = (z_max - 32.0).min(params.bounds_max[2] + 64.0);
        let cheap_start = ((1500.0 * s_ratio).round() as i32).max(1000);
        let cheap_end = ((3500.0 * s_ratio).round() as i32).max(2000);
        write_entity_bytes(
            &mut writer,
            "water_lod_control",
            &[
                ("id", next_ent().to_string()),
                ("cheapwaterstartdistance", cheap_start.to_string()),
                ("cheapwaterenddistance", cheap_end.to_string()),
                ("origin", format!("{:.1} {:.1} {:.1}", mid_wx, mid_wy, lod_z)),
            ],
            &[],
            &[],
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;
    }

    // Ambient outdoor wind soundscape
    write_entity_bytes(
        &mut writer,
        "ambient_generic",
        &[
            ("id", next_ent().to_string()),
            ("targetname", "ambient_wind".to_string()),
            ("message", "ambient/atmosphere/outdoor_wind.wav".to_string()),
            ("health", "3".to_string()),
            ("pitch", "100".to_string()),
            ("pitchstart", "100".to_string()),
            ("radius", "1250".to_string()),
            ("spawnflags", "1".to_string()),
            ("origin", format!("0 0 {:.1}", sun_z)),
        ],
        &[],
        &[],
        &mut validator,
    )
    .map_err(io_err)?;
    writer.write_all(b"\n").map_err(io_err)?;

    // Environment cubemaps for reflective materials (water pools, center, quadrants, max 16, 128 HU dedup)
    let mut cubemap_candidates: Vec<[f64; 3]> = Vec::new();
    for wp in params.water_pools {
        let pcx = (wp[0] + wp[3]) * 0.5;
        let pcy = (wp[1] + wp[4]) * 0.5;
        let pcz = wp[2].max(wp[5]) + 32.0;
        cubemap_candidates.push([pcx, pcy, pcz]);
    }
    let mid_cx = (params.bounds_min[0] + params.bounds_max[0]) * 0.5;
    let mid_cy = (params.bounds_min[1] + params.bounds_max[1]) * 0.5;
    let center_cz = spawns
        .first()
        .map(|s| s[2] + 64.0)
        .unwrap_or((params.bounds_min[2] + params.bounds_max[2]) * 0.5);
    cubemap_candidates.push([mid_cx, mid_cy, center_cz]);

    if dx > 800.0 || dy > 800.0 {
        for fx in [0.25, 0.75] {
            for fy in [0.25, 0.75] {
                let qx = params.bounds_min[0] + fx * dx;
                let qy = params.bounds_min[1] + fy * dy;
                cubemap_candidates.push([qx, qy, center_cz]);
            }
        }
    }

    let mut placed_cubemaps: Vec<[f64; 3]> = Vec::new();
    for [cx, cy, cz] in cubemap_candidates {
        if placed_cubemaps.len() >= 16 {
            break;
        }
        let too_close = placed_cubemaps.iter().any(|[ex, ey, ez]| {
            let ddx = cx - ex;
            let ddy = cy - ey;
            let ddz = cz - ez;
            ddx * ddx + ddy * ddy + ddz * ddz < (128.0 * 128.0)
        });
        if too_close {
            continue;
        }
        placed_cubemaps.push([cx, cy, cz]);
        write_entity_bytes(
            &mut writer,
            "env_cubemap",
            &[
                ("id", next_ent().to_string()),
                ("cubemapsize", "0".to_string()),
                ("origin", format!("{:.1} {:.1} {:.1}", cx, cy, cz)),
            ],
            &[],
            &[],
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;
    }

    // Emissive block lights
    let emitted_lights_count = params.lights.len().min(1500);
    for lt in params.lights.iter().take(1500) {
        let light_val = format!("{} {}", lt.color, lt.brightness);
        write_entity_bytes(
            &mut writer,
            "light",
            &[
                ("id", next_ent().to_string()),
                ("_light", light_val.clone()),
                ("_lightHDR", light_val),
                ("_lightscaleHDR", "1".to_string()),
                ("_fifty_percent_distance", lt.dist50.to_string()),
                ("_zero_percent_distance", lt.dist0.to_string()),
                ("_constant_attn", "0".to_string()),
                ("_linear_attn", "0".to_string()),
                ("_quadratic_attn", "1".to_string()),
                ("origin", format!("{:.1} {:.1} {:.1}", lt.pos[0], lt.pos[1], lt.pos[2])),
            ],
            &[],
            &[],
            &mut validator,
        )
        .map_err(io_err)?;
        writer.write_all(b"\n").map_err(io_err)?;
    }

    // Interior cavity ambient fill lights under solid opaque ceilings (disabled for gmod preset)
    if preset.ambient_fill && preset.ambient_fill_light != "0 0 0 0" {
        let max_fill_lights = 1600usize.saturating_sub(emitted_lights_count).clamp(16, 128);
        let fill_lights = compute_ambient_cavity_lights(
            params.bounds_min,
            params.bounds_max,
            params.all_solid_boxes,
            &params.lights[..emitted_lights_count],
            eff_scale,
            max_fill_lights,
        );
        let fill_dist50 = ((280.0 * s_ratio).round() as i32).max(160).to_string();
        let fill_dist0 = ((600.0 * s_ratio).round() as i32).max(320).to_string();
        for [fx, fy, fz] in fill_lights {
            write_entity_bytes(
                &mut writer,
                "light",
                &[
                    ("id", next_ent().to_string()),
                    ("_light", preset.ambient_fill_light.to_string()),
                    ("_lightHDR", preset.ambient_fill_light.to_string()),
                    ("_lightscaleHDR", "1".to_string()),
                    ("_fifty_percent_distance", fill_dist50.clone()),
                    ("_zero_percent_distance", fill_dist0.clone()),
                    ("_constant_attn", "0".to_string()),
                    ("_linear_attn", "1".to_string()),
                    ("_quadratic_attn", "0".to_string()),
                    ("origin", format!("{:.1} {:.1} {:.1}", fx, fy, fz)),
                ],
                &[],
                &[],
                &mut validator,
            )
            .map_err(io_err)?;
            writer.write_all(b"\n").map_err(io_err)?;
        }
    }

    writer.flush().map_err(io_err)?;
    let mut problems = validator.into_problems();
    // Surface silent content truncation so users understand what is missing on huge maps
    if params.water_pools.len() > 1024 {
        problems.push(format!(
            "Water pools truncated: emitted 1024 of {} brushes; remaining water is missing from the map",
            params.water_pools.len()
        ));
    }
    if params.hazard_boxes.len() > 512 {
        problems.push(format!(
            "Hazard triggers truncated: emitted 512 of {} (lava/fire/cactus/magma damage zones)",
            params.hazard_boxes.len()
        ));
    }
    if params.lights.len() > 1500 {
        problems.push(format!(
            "Block lights truncated: emitted 1500 of {} light entities",
            params.lights.len()
        ));
    }
    Ok(problems)
}

#[allow(clippy::too_many_arguments)]
fn write_box_brush<W: Write>(
    out: &mut W,
    brush_id: usize,
    x1: f64,
    y1: f64,
    z1: f64,
    x2: f64,
    y2: f64,
    z2: f64,
    face_materials: &[&str; 6],
    side_id: &mut usize,
    visgroup_id: Option<u32>,
    editor_color: &str,
    validator: &mut VmfValidator,
) -> std::io::Result<()> {
    write_box_brush_with_uv(
        out,
        brush_id,
        x1,
        y1,
        z1,
        x2,
        y2,
        z2,
        face_materials,
        BrushUvMode::Default,
        side_id,
        visgroup_id,
        editor_color,
        validator,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_box_brush_scaled<W: Write>(
    out: &mut W,
    brush_id: usize,
    x1: f64,
    y1: f64,
    z1: f64,
    x2: f64,
    y2: f64,
    z2: f64,
    face_materials: &[&str; 6],
    uv_scale: f64,
    side_id: &mut usize,
    visgroup_id: Option<u32>,
    editor_color: &str,
    validator: &mut VmfValidator,
) -> std::io::Result<()> {
    write_box_brush_with_uv(
        out,
        brush_id,
        x1,
        y1,
        z1,
        x2,
        y2,
        z2,
        face_materials,
        BrushUvMode::LocalTile(uv_scale),
        side_id,
        visgroup_id,
        editor_color,
        validator,
    )
}

#[derive(Clone, Copy)]
enum BrushUvMode {
    Default,
    LocalTile(f64),
    WorldAligned(f64),
}

#[allow(clippy::too_many_arguments)]
fn write_box_brush_with_uv<W: Write>(
    out: &mut W,
    brush_id: usize,
    x1: f64,
    y1: f64,
    z1: f64,
    x2: f64,
    y2: f64,
    z2: f64,
    face_materials: &[&str; 6],
    uv_mode: BrushUvMode,
    side_id: &mut usize,
    visgroup_id: Option<u32>,
    editor_color: &str,
    validator: &mut VmfValidator,
) -> std::io::Result<()> {
    let x_min = x1.min(x2);
    let mut x_max = x1.max(x2);
    let y_min = y1.min(y2);
    let mut y_max = y1.max(y2);
    let z_min = z1.min(z2);
    let mut z_max = z1.max(z2);
    if (x_max - x_min).abs() < 1.0 {
        x_max = x_min + 1.0;
    }
    if (y_max - y_min).abs() < 1.0 {
        y_max = y_min + 1.0;
    }
    if (z_max - z_min).abs() < 1.0 {
        z_max = z_min + 1.0;
    }

    validator.record_solid(6);
    writeln!(out, "\tsolid\n\t{{\n\t\t\"id\" \"{}\"", brush_id)?;
    let uv_axes: [(String, String); 6] = match uv_mode {
        BrushUvMode::LocalTile(uv_scale) => {
            let s = if uv_scale > 0.0 { uv_scale } else { 39.37 / 16.0 };
            let u_xp = (-x_min / s).rem_euclid(16.0);
            let u_xn = (x_max / s).rem_euclid(16.0);
            let u_yp = (-y_min / s).rem_euclid(16.0);
            let u_yn = (y_max / s).rem_euclid(16.0);
            let v_cap = (y_max / s).rem_euclid(16.0);
            let v_side = (z_max / s).rem_euclid(16.0);
            [
                (
                    format!("[1 0 0 {:.6}] {:.6}", u_xp, s),
                    format!("[0 -1 0 {:.6}] {:.6}", v_cap, s),
                ),
                (
                    format!("[1 0 0 {:.6}] {:.6}", u_xp, s),
                    format!("[0 -1 0 {:.6}] {:.6}", v_cap, s),
                ),
                (
                    format!("[1 0 0 {:.6}] {:.6}", u_xp, s),
                    format!("[0 0 -1 {:.6}] {:.6}", v_side, s),
                ),
                (
                    format!("[-1 0 0 {:.6}] {:.6}", u_xn, s),
                    format!("[0 0 -1 {:.6}] {:.6}", v_side, s),
                ),
                (
                    format!("[0 1 0 {:.6}] {:.6}", u_yp, s),
                    format!("[0 0 -1 {:.6}] {:.6}", v_side, s),
                ),
                (
                    format!("[0 -1 0 {:.6}] {:.6}", u_yn, s),
                    format!("[0 0 -1 {:.6}] {:.6}", v_side, s),
                ),
            ]
        }
        BrushUvMode::WorldAligned(uv_scale) => {
            let s = if uv_scale > 0.0 { uv_scale } else { 39.37 / 64.0 };
            [
                (format!("[1 0 0 0] {:.6}", s), format!("[0 -1 0 0] {:.6}", s)),
                (format!("[1 0 0 0] {:.6}", s), format!("[0 -1 0 0] {:.6}", s)),
                (format!("[1 0 0 0] {:.6}", s), format!("[0 0 -1 0] {:.6}", s)),
                (format!("[-1 0 0 0] {:.6}", s), format!("[0 0 -1 0] {:.6}", s)),
                (format!("[0 1 0 0] {:.6}", s), format!("[0 0 -1 0] {:.6}", s)),
                (format!("[0 -1 0 0] {:.6}", s), format!("[0 0 -1 0] {:.6}", s)),
            ]
        }
        BrushUvMode::Default => [
            ("[1 0 0 0] 0.25".to_string(), "[0 -1 0 0] 0.25".to_string()),
            ("[1 0 0 0] 0.25".to_string(), "[0 -1 0 0] 0.25".to_string()),
            ("[1 0 0 0] 0.25".to_string(), "[0 0 -1 0] 0.25".to_string()),
            ("[-1 0 0 0] 0.25".to_string(), "[0 0 -1 0] 0.25".to_string()),
            ("[0 1 0 0] 0.25".to_string(), "[0 0 -1 0] 0.25".to_string()),
            ("[0 -1 0 0] 0.25".to_string(), "[0 0 -1 0] 0.25".to_string()),
        ],
    };
    let planes: [[f64; 9]; 6] = [
        [x_min, y_max, z_max, x_max, y_max, z_max, x_max, y_min, z_max],
        [x_min, y_min, z_min, x_max, y_min, z_min, x_max, y_max, z_min],
        [x_min, y_min, z_min, x_min, y_min, z_max, x_max, y_min, z_max],
        [x_max, y_max, z_min, x_max, y_max, z_max, x_min, y_max, z_max],
        [x_min, y_max, z_min, x_min, y_max, z_max, x_min, y_min, z_max],
        [x_max, y_min, z_min, x_max, y_min, z_max, x_max, y_max, z_max],
    ];
    for (idx, (p, (uaxis, vaxis))) in planes.iter().zip(uv_axes.iter()).enumerate() {
        let sid = *side_id;
        *side_id += 1;
        writeln!(
            out,
            "\t\tside\n\t\t{{\n\t\t\t\"id\" \"{}\"\n\t\t\t\"plane\" \"({:.2} {:.2} {:.2}) ({:.2} {:.2} {:.2}) ({:.2} {:.2} {:.2})\"\n\t\t\t\"material\" \"{}\"\n\t\t\t\"uaxis\" \"{}\"\n\t\t\t\"vaxis\" \"{}\"\n\t\t\t\"rotation\" \"0\"\n\t\t\t\"lightmapscale\" \"16\"\n\t\t\t\"smoothing_groups\" \"0\"\n\t\t}}",
            sid, p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8], face_materials[idx], uaxis, vaxis
        )?;
    }
    if let Some(vg) = visgroup_id {
        writeln!(
            out,
            "\t\teditor\n\t\t{{\n\t\t\t\"color\" \"{}\"\n\t\t\t\"visgroupid\" \"{}\"\n\t\t\t\"visgroupshown\" \"1\"\n\t\t\t\"visgroupautoshown\" \"1\"\n\t\t}}",
            editor_color, vg
        )?;
    }
    write!(out, "\t}}")
}

#[allow(clippy::too_many_arguments)]
fn write_door_brush<W: Write>(
    out: &mut W,
    brush_id: usize,
    inset_bounds: [f64; 6],
    full_bounds: [f64; 6],
    door_mat: &str,
    uv_scale: f64,
    side_id: &mut usize,
    validator: &mut VmfValidator,
) -> std::io::Result<()> {
    let x_min = inset_bounds[0].min(inset_bounds[3]);
    let mut x_max = inset_bounds[0].max(inset_bounds[3]);
    let y_min = inset_bounds[1].min(inset_bounds[4]);
    let mut y_max = inset_bounds[1].max(inset_bounds[4]);
    let z_min = inset_bounds[2].min(inset_bounds[5]);
    let mut z_max = inset_bounds[2].max(inset_bounds[5]);

    if (x_max - x_min).abs() < 1.0 {
        x_max = x_min + 1.0;
    }
    if (y_max - y_min).abs() < 1.0 {
        y_max = y_min + 1.0;
    }
    if (z_max - z_min).abs() < 1.0 {
        z_max = z_min + 1.0;
    }

    let fx_min = full_bounds[0].min(full_bounds[3]);
    let fx_max = full_bounds[0].max(full_bounds[3]);
    let fy_min = full_bounds[1].min(full_bounds[4]);
    let fy_max = full_bounds[1].max(full_bounds[4]);
    let fz_max = full_bounds[2].max(full_bounds[5]);

    let s = if uv_scale > 0.0 { uv_scale } else { 39.37 / 16.0 };

    let u_shift_x_pos = (-fx_min / s).rem_euclid(16.0);
    let u_shift_x_neg = (fx_max / s).rem_euclid(16.0);
    let u_shift_y_pos = (-fy_min / s).rem_euclid(16.0);
    let u_shift_y_neg = (fy_max / s).rem_euclid(16.0);
    let v_shift_z = (fz_max / s).rem_euclid(32.0);
    let v_shift_cap_y = (fy_max / s).rem_euclid(32.0);
    let v_shift_cap_x = (fx_max / s).rem_euclid(32.0);

    let span_x = fx_max - fx_min;
    let span_y = fy_max - fy_min;
    let (cap_uaxis, cap_vaxis) = if span_x >= span_y {
        (
            format!("[1 0 0 {:.6}] {:.6}", u_shift_x_pos, s),
            format!("[0 -1 0 {:.6}] {:.6}", v_shift_cap_y, s),
        )
    } else {
        (
            format!("[0 1 0 {:.6}] {:.6}", u_shift_y_pos, s),
            format!("[1 0 0 {:.6}] {:.6}", v_shift_cap_x, s),
        )
    };

    let vz = format!("[0 0 -1 {:.6}] {:.6}", v_shift_z, s);
    let u_xp = format!("[1 0 0 {:.6}] {:.6}", u_shift_x_pos, s);
    let u_xn = format!("[-1 0 0 {:.6}] {:.6}", u_shift_x_neg, s);
    let u_yp = format!("[0 1 0 {:.6}] {:.6}", u_shift_y_pos, s);
    let u_yn = format!("[0 -1 0 {:.6}] {:.6}", u_shift_y_neg, s);

    let faces: [([f64; 9], &str, &str); 6] = [
        ([x_min, y_max, z_max, x_max, y_max, z_max, x_max, y_min, z_max], &cap_uaxis, &cap_vaxis),
        ([x_min, y_min, z_min, x_max, y_min, z_min, x_max, y_max, z_min], &cap_uaxis, &cap_vaxis),
        ([x_min, y_min, z_min, x_min, y_min, z_max, x_max, y_min, z_max], &u_xp, &vz),
        ([x_max, y_max, z_min, x_max, y_max, z_max, x_min, y_max, z_max], &u_xn, &vz),
        ([x_min, y_max, z_min, x_min, y_max, z_max, x_min, y_min, z_max], &u_yp, &vz),
        ([x_max, y_min, z_min, x_max, y_min, z_max, x_max, y_max, z_max], &u_yn, &vz),
    ];

    validator.record_solid(6);
    writeln!(out, "\tsolid\n\t{{\n\t\t\"id\" \"{}\"", brush_id)?;
    for (p, uaxis, vaxis) in &faces {
        let sid = *side_id;
        *side_id += 1;
        writeln!(
            out,
            "\t\tside\n\t\t{{\n\t\t\t\"id\" \"{}\"\n\t\t\t\"plane\" \"({:.2} {:.2} {:.2}) ({:.2} {:.2} {:.2}) ({:.2} {:.2} {:.2})\"\n\t\t\t\"material\" \"{}\"\n\t\t\t\"uaxis\" \"{}\"\n\t\t\t\"vaxis\" \"{}\"\n\t\t\t\"rotation\" \"0\"\n\t\t\t\"lightmapscale\" \"16\"\n\t\t\t\"smoothing_groups\" \"0\"\n\t\t}}",
            sid, p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8], door_mat, uaxis, vaxis
        )?;
    }
    write!(
        out,
        "\t\teditor\n\t\t{{\n\t\t\t\"color\" \"0 220 100\"\n\t\t\t\"visgroupid\" \"4\"\n\t\t\t\"visgroupshown\" \"1\"\n\t\t\t\"visgroupautoshown\" \"1\"\n\t\t}}\n\t}}"
    )
}

fn write_entity_bytes<W: Write>(
    out: &mut W,
    classname: &str,
    kv: &[(&str, String)],
    connections: &[(&str, String)],
    solids_bytes: &[u8],
    validator: &mut VmfValidator,
) -> std::io::Result<()> {
    validator.record_entity(classname, kv, connections);
    out.write_all(b"entity\n{\n")?;
    for (k, v) in kv {
        if *k == "id" {
            writeln!(out, "\t\"id\" \"{}\"", v)?;
        }
    }
    writeln!(out, "\t\"classname\" \"{}\"", classname)?;
    for (k, v) in kv {
        if *k != "id" {
            writeln!(out, "\t\"{}\" \"{}\"", k, v)?;
        }
    }
    if !connections.is_empty() {
        out.write_all(b"\tconnections\n\t{\n")?;
        for (out_name, cmd_val) in connections {
            writeln!(out, "\t\t\"{}\" \"{}\"", out_name, cmd_val)?;
        }
        out.write_all(b"\t}\n")?;
    }
    if !solids_bytes.is_empty() {
        out.write_all(solids_bytes)?;
    }
    out.write_all(b"}")
}

fn clamp_box_coords(b: [f64; 6], bounds: [f64; 6], min_span: f64) -> Option<[f64; 6]> {
    let x0 = b[0].min(b[3]).max(bounds[0]);
    let x1 = b[0].max(b[3]).min(bounds[1]);
    let y0 = b[1].min(b[4]).max(bounds[2]);
    let y1 = b[1].max(b[4]).min(bounds[3]);
    let z0 = b[2].min(b[5]).max(bounds[4]);
    let z1 = b[2].max(b[5]).min(bounds[5]);
    if (x1 - x0) < min_span || (y1 - y0) < min_span || (z1 - z0) < min_span {
        None
    } else {
        Some([x0, y0, z0, x1, y1, z1])
    }
}

fn is_hull_blocked(
    qx: f64,
    qy: f64,
    foot_z: f64,
    hull_half: f64,
    hull_height: f64,
    boxes: &[&CollisionBox],
) -> bool {
    boxes.iter().any(|b| {
        let c = b.box_coords;
        let x0 = c[0].min(c[3]);
        let x1 = c[0].max(c[3]);
        let y0 = c[1].min(c[4]);
        let y1 = c[1].max(c[4]);
        let z0 = c[2].min(c[5]);
        let z1 = c[2].max(c[5]);
        x0 < qx + hull_half
            && x1 > qx - hull_half
            && y0 < qy + hull_half
            && y1 > qy - hull_half
            && z0 < foot_z + hull_height
            && z1 > foot_z
    })
}

fn find_top_surface_at(
    qx: f64,
    qy: f64,
    boxes: &[&CollisionBox],
    z_max_limit: f64,
    preferred_z: Option<f64>,
    hull_half: f64,
    hull_height: f64,
) -> Option<f64> {
    let mut support_tops: Vec<f64> = boxes
        .iter()
        .filter_map(|b| {
            let c = b.box_coords;
            let x0 = c[0].min(c[3]);
            let x1 = c[0].max(c[3]);
            let y0 = c[1].min(c[4]);
            let y1 = c[1].max(c[4]);
            let z1 = c[2].max(c[5]);
            if qx >= x0 && qx <= x1 && qy >= y0 && qy <= y1 && z1 <= z_max_limit {
                Some(z1)
            } else {
                None
            }
        })
        .collect();

    support_tops.sort_by(|a, b| {
        if let Some(pz) = preferred_z {
            let da = (a - pz).abs();
            let db = (b - pz).abs();
            da.total_cmp(&db).then_with(|| b.total_cmp(a))
        } else {
            b.total_cmp(a)
        }
    });
    support_tops.dedup_by(|a, b| (*a - *b).abs() < 1e-3);

    for top_z in support_tops {
        let foot_z = top_z + 2.0;
        if !is_hull_blocked(qx, qy, foot_z, hull_half, hull_height, boxes) {
            return Some(foot_z);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn resolve_spawn_points(
    preferred: &[[f64; 3]],
    bmin: [f64; 3],
    bmax: [f64; 3],
    sky_bounds: [f64; 6],
    solids: &[CollisionBox],
    waters: &[[f64; 6]],
    hazards: &[HazardBox],
    eff_scale: f64,
) -> Vec<[f64; 3]> {
    let min_spawns = 4usize;
    let max_spawns = 8usize;
    let min_spacing = 48.0f64;

    let ix_min = sky_bounds[0] + 24.0;
    let ix_max = (ix_min + 4.0).max(sky_bounds[1] - 24.0);
    let iy_min = sky_bounds[2] + 24.0;
    let iy_max = (iy_min + 4.0).max(sky_bounds[3] - 24.0);
    let iz_min = sky_bounds[4] + 4.0;
    let iz_max = (iz_min + 4.0).max(sky_bounds[5] - 80.0);

    let s_ratio = eff_scale / 39.37;
    let hull_half = (11.5 * s_ratio).max(2.0);
    let hull_height = (70.0 * s_ratio).max(24.0);

    let in_skybox_xy = |px: f64, py: f64| -> bool {
        px >= ix_min && px <= ix_max && py >= iy_min && py <= iy_max
    };
    let clamp_z = |pz: f64| -> f64 { pz.clamp(iz_min, iz_max) };
    let is_far_enough = |px: f64, py: f64, pz: f64, existing: &[[f64; 3]]| -> bool {
        existing.iter().all(|&[qx, qy, qz]| {
            ((px - qx).powi(2) + (py - qy).powi(2)).sqrt() >= min_spacing || (pz - qz).abs() >= 80.0
        })
    };
    let is_hazard_or_water = |px: f64, py: f64, pz: f64| -> bool {
        let in_haz = hazards.iter().any(|h| {
            let c = h.box_coords;
            px + hull_half > c[0].min(c[3])
                && px - hull_half < c[0].max(c[3])
                && py + hull_half > c[1].min(c[4])
                && py - hull_half < c[1].max(c[4])
                && pz < c[2].max(c[5]) + 4.0
                && pz + hull_height > c[2].min(c[5]) - 4.0
        });
        let deep_water = waters.iter().any(|w| {
            px + hull_half > w[0].min(w[3])
                && px - hull_half < w[0].max(w[3])
                && py + hull_half > w[1].min(w[4])
                && py - hull_half < w[1].max(w[4])
                && (w[2].max(w[5]) - pz) > 40.0 * s_ratio
        });
        in_haz || deep_water
    };

    let all_solid_refs: Vec<&CollisionBox> = solids.iter().collect();
    let mut accepted: Vec<[f64; 3]> = Vec::new();

    for &[raw_x, raw_y, raw_z] in preferred {
        if accepted.len() >= max_spawns {
            break;
        }
        let px = raw_x.clamp(ix_min, ix_max);
        let py = raw_y.clamp(iy_min, iy_max);
        let pz = clamp_z(raw_z);

        let chosen_z = if all_solid_refs.is_empty()
            || (!is_hull_blocked(px, py, pz, hull_half, hull_height, &all_solid_refs)
                && !is_hazard_or_water(px, py, pz))
        {
            Some(pz)
        } else {
            find_top_surface_at(px, py, &all_solid_refs, iz_max, Some(pz), hull_half, hull_height)
                .map(&clamp_z)
                .filter(|&fz| !is_hazard_or_water(px, py, fz))
        };

        if let Some(cz) = chosen_z {
            if in_skybox_xy(px, py) && is_far_enough(px, py, cz, &accepted) {
                accepted.push([px, py, cz]);
            }
        }
    }

    let mid_x = ((bmin[0] + bmax[0]) * 0.5).clamp(ix_min, ix_max);
    let mid_y = ((bmin[1] + bmax[1]) * 0.5).clamp(iy_min, iy_max);
    let (ax, ay) = if let Some(first) = accepted.first() {
        (first[0], first[1])
    } else {
        (mid_x, mid_y)
    };
    let mut pref_z = accepted.first().map(|p| p[2]);

    if accepted.len() < min_spawns && !solids.is_empty() {
        let filtered: Vec<&CollisionBox> = solids
            .iter()
            .filter(|b| {
                let c = b.box_coords;
                c[3] >= ax - 600.0 && c[0] <= ax + 600.0 && c[4] >= ay - 600.0 && c[1] <= ay + 600.0
            })
            .collect();
        let local_boxes: &[&CollisionBox] = if filtered.is_empty() {
            &all_solid_refs
        } else {
            &filtered
        };

        let mut sample_coords: Vec<[f64; 2]> = Vec::new();
        if accepted.is_empty() {
            sample_coords.push([ax, ay]);
        }
        let ring_radii = [56.0, 88.0, 128.0, 180.0, 256.0, 384.0];
        let angles_deg = [0.0f64, 90.0, 180.0, 270.0, 45.0, 135.0, 225.0, 315.0];
        for rad in ring_radii {
            for deg in angles_deg {
                let ang = deg.to_radians();
                sample_coords.push([ax + rad * ang.cos(), ay + rad * ang.sin()]);
            }
        }

        for (hh, hgt) in [
            (hull_half, hull_height),
            (hull_half.min(2.0), hull_height.min(60.0)),
        ] {
            for &[qx, qy] in &sample_coords {
                if accepted.len() >= min_spawns {
                    break;
                }
                if !in_skybox_xy(qx, qy) {
                    continue;
                }
                if let Some(surf_z) = find_top_surface_at(qx, qy, local_boxes, iz_max, pref_z, hh, hgt) {
                    let cz = clamp_z(surf_z);
                    if !is_hazard_or_water(qx, qy, cz) && is_far_enough(qx, qy, cz, &accepted) {
                        accepted.push([qx, qy, cz]);
                        if pref_z.is_none() {
                            pref_z = Some(cz);
                        }
                    }
                }
            }
            if !accepted.is_empty() && ((hgt - hull_height).abs() < 1e-6 && accepted.len() >= min_spawns || (hgt - hull_height).abs() > 1e-6) {
                break;
            }
        }

        if accepted.is_empty() && !local_boxes.is_empty() {
            if let Some(nearest) = local_boxes.iter().min_by(|a, b| {
                let da = (((a.box_coords[0] + a.box_coords[3]) * 0.5) - ax).hypot(((a.box_coords[1] + a.box_coords[4]) * 0.5) - ay);
                let db = (((b.box_coords[0] + b.box_coords[3]) * 0.5) - ax).hypot(((b.box_coords[1] + b.box_coords[4]) * 0.5) - ay);
                da.total_cmp(&db)
            }) {
                let bx = ((nearest.box_coords[0] + nearest.box_coords[3]) * 0.5).clamp(ix_min, ix_max);
                let by = ((nearest.box_coords[1] + nearest.box_coords[4]) * 0.5).clamp(iy_min, iy_max);
                let bz = clamp_z(nearest.box_coords[2].max(nearest.box_coords[5]) + 4.0);
                accepted.push([bx, by, bz]);
            }
        }
    }

    if accepted.is_empty() {
        accepted.push([mid_x, mid_y, clamp_z(bmax[2] + 4.0)]);
    }

    if accepted.len() < min_spawns {
        let [base_x, base_y, base_z] = accepted[0];
        let bw = bmax[0] - bmin[0];
        let bh = bmax[1] - bmin[1];
        let px_min = if bw >= 140.0 { (bmin[0] + 12.0).max(ix_min) } else { ix_min };
        let px_max = if bw >= 140.0 { (bmax[0] - 12.0).min(ix_max) } else { ix_max };
        let py_min = if bh >= 140.0 { (bmin[1] + 12.0).max(iy_min) } else { iy_min };
        let py_max = if bh >= 140.0 { (bmax[1] - 12.0).min(iy_max) } else { iy_max };
        let pad_offsets = [
            (56.0, 0.0),
            (-56.0, 0.0),
            (0.0, 56.0),
            (0.0, -56.0),
            (56.0, 56.0),
            (-56.0, 56.0),
            (56.0, -56.0),
            (-56.0, -56.0),
        ];
        for (ox, oy) in pad_offsets {
            if accepted.len() >= min_spawns {
                break;
            }
            let qx = base_x + ox;
            let qy = base_y + oy;
            if !in_skybox_xy(qx, qy) || qx < px_min || qx > px_max || qy < py_min || qy > py_max {
                continue;
            }
            let cand_z = clamp_z(base_z);
            if !all_solid_refs.is_empty()
                && is_hull_blocked(qx, qy, cand_z + 1.0, hull_half, hull_height, &all_solid_refs)
            {
                continue;
            }
            if is_hazard_or_water(qx, qy, cand_z) {
                continue;
            }
            if is_far_enough(qx, qy, cand_z, &accepted) {
                accepted.push([qx, qy, cand_z]);
            }
        }
    }

    accepted.truncate(max_spawns);
    accepted
}

fn merge_z_intervals(boxes: &[&CollisionBox]) -> Vec<(f64, f64)> {
    let mut raw: Vec<(f64, f64)> = boxes
        .iter()
        .map(|b| (b.box_coords[2].min(b.box_coords[5]), b.box_coords[2].max(b.box_coords[5])))
        .collect();
    raw.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(raw.len());
    for (zs, ze) in raw {
        if let Some(last) = merged.last_mut() {
            if zs <= last.1 {
                last.1 = last.1.max(ze);
                continue;
            }
        }
        merged.push((zs, ze));
    }
    merged
}

fn compute_ambient_cavity_lights(
    bmin: [f64; 3],
    bmax: [f64; 3],
    solids: &[CollisionBox],
    existing_lights: &[BlockLight],
    scale: f64,
    max_fill_lights: usize,
) -> Vec<[f64; 3]> {
    if max_fill_lights == 0 || !solids.iter().any(|b| b.is_opaque_roof) {
        return Vec::new();
    }

    let s_ratio = scale / 39.37;
    let dx = (bmax[0] - bmin[0]).max(100.0);
    let dy = (bmax[1] - bmin[1]).max(100.0);
    let dz = (bmax[2] - bmin[2]).max(100.0);

    let step_xy = (320.0 * s_ratio).clamp(256.0, 384.0);
    let step_z = (220.0 * s_ratio).clamp(160.0, 320.0);
    let nx = ((dx / step_xy).ceil() as usize).clamp(1, 32);
    let ny = ((dy / step_xy).ceil() as usize).clamp(1, 32);
    let nz = ((dz / step_z).ceil() as usize).clamp(1, 8);

    let cell_w = dx / nx as f64;
    let cell_h = dy / ny as f64;
    let x_pts: Vec<f64> = (0..nx).map(|i| bmin[0] + (i as f64 + 0.5) * cell_w).collect();
    let y_pts: Vec<f64> = (0..ny).map(|j| bmin[1] + (j as f64 + 0.5) * cell_h).collect();
    let z_pts: Vec<f64> = (0..nz)
        .map(|k| bmin[2] + (64.0 * s_ratio).max((k as f64 + 0.5) * (dz / nz as f64)))
        .collect();

    let col_margin = 18.0 * s_ratio;
    let min_cavity_h = 60.0 * s_ratio;
    let min_light_sep_sq = (128.0 * s_ratio).powi(2);

    // Populate 2D (nx x ny) spatial bucket grid in one O(N) pass over solids.
    let mut roof_grid: Vec<Vec<&CollisionBox>> = vec![Vec::new(); nx * ny];
    let mut floor_grid: Vec<Vec<&CollisionBox>> = vec![Vec::new(); nx * ny];
    for b in solids {
        let c = b.box_coords;
        let bx0 = c[0].min(c[3]) - col_margin;
        let bx1 = c[0].max(c[3]) + col_margin;
        let by0 = c[1].min(c[4]) - col_margin;
        let by1 = c[1].max(c[4]) + col_margin;
        let i0 = (((bx0 - bmin[0]) / cell_w - 0.5).ceil() as isize).max(0) as usize;
        let i1 = (((bx1 - bmin[0]) / cell_w - 0.5).floor() as isize).min(nx as isize - 1);
        let j0 = (((by0 - bmin[1]) / cell_h - 0.5).ceil() as isize).max(0) as usize;
        let j1 = (((by1 - bmin[1]) / cell_h - 0.5).floor() as isize).min(ny as isize - 1);
        if i0 as isize > i1 || j0 as isize > j1 {
            continue;
        }
        for i in i0..=(i1 as usize) {
            let row_off = i * ny;
            for j in j0..=(j1 as usize) {
                let idx = row_off + j;
                floor_grid[idx].push(b);
                if b.is_opaque_roof {
                    roof_grid[idx].push(b);
                }
            }
        }
    }

    let mut fill = Vec::new();
    for (i, &gx) in x_pts.iter().enumerate() {
        if fill.len() >= max_fill_lights {
            break;
        }
        let row_off = i * ny;
        for (j, &gy) in y_pts.iter().enumerate() {
            if fill.len() >= max_fill_lights {
                break;
            }
            let cell_idx = row_off + j;
            let col_roofs = &roof_grid[cell_idx];
            if col_roofs.is_empty() {
                continue;
            }
            let merged_roof = merge_z_intervals(col_roofs);
            let roof_top = merged_roof.last().unwrap().1;

            let col_floors = &floor_grid[cell_idx];
            let merged_floor = if col_floors.is_empty() {
                merged_roof.clone()
            } else {
                merge_z_intervals(col_floors)
            };
            let floor_bottom = merged_floor[0].0;

            for &gz in &z_pts {
                if fill.len() >= max_fill_lights {
                    break;
                }
                if gz + 20.0 >= roof_top || gz <= floor_bottom {
                    continue;
                }

                let inside_solid = merged_roof.iter().any(|&(zs, ze)| zs <= gz && gz <= ze)
                    || merged_floor.iter().any(|&(zs, ze)| zs <= gz && gz <= ze);

                let target_z = if inside_solid {
                    let mut found = None;
                    for idx in 0..merged_roof.len().saturating_sub(1) {
                        let floor_top = merged_roof[idx].1;
                        let roof_bottom = merged_roof[idx + 1].0;
                        if roof_bottom - floor_top >= min_cavity_h {
                            let mid_z = (floor_top + roof_bottom) * 0.5;
                            if !merged_floor.iter().any(|&(zs, ze)| zs <= mid_z && mid_z <= ze) {
                                found = Some(mid_z);
                                break;
                            }
                        }
                    }
                    match found {
                        Some(z) => z,
                        None => continue,
                    }
                } else {
                    let roof_z = merged_roof
                        .iter()
                        .filter(|&&(zs, _)| zs > gz)
                        .map(|&(zs, _)| zs)
                        .reduce(f64::min);
                    let floor_z = merged_floor
                        .iter()
                        .filter(|&&(_, ze)| ze < gz)
                        .map(|&(_, ze)| ze)
                        .reduce(f64::max);
                    match (roof_z, floor_z) {
                        (Some(rz), Some(fz)) if (rz - fz) >= min_cavity_h => {
                            let mid_z = fz + (rz - fz) * 0.68;
                            if merged_floor.iter().any(|&(zs, ze)| zs <= mid_z && mid_z <= ze) {
                                gz
                            } else {
                                mid_z
                            }
                        }
                        _ => continue,
                    }
                };

                if merged_floor.iter().any(|&(zs, ze)| zs <= target_z && target_z <= ze) {
                    continue;
                }

                let has_nearby_light = existing_lights.iter().any(|l| {
                    let d2 = (l.pos[0] - gx).powi(2) + (l.pos[1] - gy).powi(2) + (l.pos[2] - target_z).powi(2);
                    d2 < min_light_sep_sq
                }) || fill.iter().any(|f: &[f64; 3]| {
                    let d2 = (f[0] - gx).powi(2) + (f[1] - gy).powi(2) + (f[2] - target_z).powi(2);
                    d2 < min_light_sep_sq
                });

                if !has_nearby_light {
                    fill.push([gx, gy, target_z]);
                }
            }
        }
    }
    fill
}

/// Helper used by tests / diagnostics
#[cfg(test)]
pub fn count_vmf_entities(vmf_text: &str, classname: &str) -> usize {
    let pat = format!("\"classname\" \"{}\"", classname);
    vmf_text.matches(&pat).count()
}

#[cfg(test)]
pub fn vmf_contains_kv(vmf_text: &str, k: &str, v: &str) -> bool {
    let pat = format!("\"{}\" \"{}\"", k, v);
    vmf_text.contains(&pat)
}
