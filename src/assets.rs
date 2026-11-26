use crate::anvil::BlockState;
use image::{ImageBuffer, Rgba, RgbaImage};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaceDir {
    Up,
    Down,
    North,
    South,
    East,
    West,
}

impl FaceDir {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "up" | "top" => Some(Self::Up),
            "down" | "bottom" => Some(Self::Down),
            "north" => Some(Self::North),
            "south" => Some(Self::South),
            "east" => Some(Self::East),
            "west" => Some(Self::West),
            _ => None,
        }
    }


    pub fn rotate_xyz(self, rot_x: i32, rot_y: i32, rot_z: i32) -> Self {
        let mut dir = self;
        let steps_x = rot_x.rem_euclid(360) / 90;
        for _ in 0..steps_x {
            dir = match dir {
                Self::Up => Self::North,
                Self::North => Self::Down,
                Self::Down => Self::South,
                Self::South => Self::Up,
                other => other,
            };
        }
        let steps_y = rot_y.rem_euclid(360) / 90;
        for _ in 0..steps_y {
            dir = match dir {
                Self::North => Self::East,
                Self::East => Self::South,
                Self::South => Self::West,
                Self::West => Self::North,
                other => other,
            };
        }
        let steps_z = rot_z.rem_euclid(360) / 90;
        for _ in 0..steps_z {
            dir = match dir {
                Self::Up => Self::East,
                Self::East => Self::Down,
                Self::Down => Self::West,
                Self::West => Self::Up,
                other => other,
            };
        }
        dir
    }

    pub fn offset(self) -> [i32; 3] {
        match self {
            Self::Up => [0, 1, 0],
            Self::Down => [0, -1, 0],
            Self::North => [0, 0, -1],
            Self::South => [0, 0, 1],
            Self::East => [1, 0, 0],
            Self::West => [-1, 0, 0],
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelFace {
    pub uv: Option<[f32; 4]>, // [u1, v1, u2, v2] in 0..16
    pub texture: String,      // canonical "ns:path"
    pub rotation: i32,
    pub tint_index: Option<i32>,
    pub cullface: Option<FaceDir>,
}

#[derive(Debug, Clone)]
pub struct ModelRotation {
    pub origin: [f32; 3], // 0..16
    pub axis: char,       // 'x', 'y', 'z'
    pub angle: f32,
    pub rescale: bool,
    pub euler_xyz: Option<[f32; 3]>, // MC 1.21.11+ multi-axis rotation { x, y, z }
}

#[derive(Debug, Clone)]
pub struct ModelElement {
    pub from: [f32; 3], // 0..16
    pub to: [f32; 3],   // 0..16
    pub rotation: Option<ModelRotation>,
    pub faces: HashMap<FaceDir, ModelFace>,
    pub rot_x: i32,
    pub rot_y: i32,
    pub rot_z: i32,
    pub uvlock: bool,
}

#[derive(Debug, Clone)]
pub struct ObjTri {
    pub positions: [[f32; 3]; 3], // MC 0..1 block local space
    pub uvs: [[f32; 2]; 3],
    pub normals: [[f32; 3]; 3],
    pub texture: String, // canonical "ns:path"
}

#[derive(Debug, Clone)]
pub struct AnimatedStrip {
    pub frames: Vec<RgbaImage>,
    pub fps: f32,
}

#[derive(Debug, Clone)]
pub enum ResolvedBlockModel {
    Elements(Vec<ModelElement>),
    Obj(Vec<ObjTri>),
    Empty,
}

#[derive(Debug, Deserialize)]
struct BlockstateJson {
    #[serde(default)]
    variants: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    multipart: Option<Vec<MultipartCase>>,
}

#[derive(Debug, Deserialize)]
struct MultipartCase {
    #[serde(default)]
    when: Option<serde_json::Value>,
    apply: serde_json::Value,
}

#[derive(Debug, Deserialize, Default)]
struct ModelJson {
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    textures: HashMap<String, String>,
    #[serde(default)]
    elements: Option<Vec<RawElement>>,
    #[serde(default)]
    model: Option<String>, // forge:obj
}

#[derive(Debug, Deserialize)]
struct RawElement {
    from: [f32; 3],
    to: [f32; 3],
    #[serde(default)]
    rotation: Option<RawRotation>,
    #[serde(default)]
    faces: HashMap<String, RawFace>,
}

#[derive(Debug, Deserialize)]
struct RawRotation {
    origin: [f32; 3],
    #[serde(default)]
    axis: Option<String>,
    #[serde(default)]
    angle: f32,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    #[serde(default)]
    z: f32,
    #[serde(default)]
    rescale: bool,
}

#[derive(Debug, Deserialize)]
struct RawFace {
    #[serde(default)]
    uv: Option<[f32; 4]>,
    texture: String,
    #[serde(default)]
    rotation: i32,
    #[serde(default)]
    tintindex: Option<i32>,
    #[serde(default)]
    cullface: Option<String>,
}

enum AssetLoc {
    #[cfg(test)]
    Inline(Arc<[u8]>),
    Zip { source_idx: usize, entry_idx: usize },
    File(PathBuf),
}

pub struct AssetIndex {
    zip_sources: Vec<Mutex<ZipArchive<BufReader<File>>>>,
    entries: HashMap<String, AssetLoc>, // normalized lowercase paths "assets/<ns>/..."
    bytes_cache: RwLock<HashMap<String, Arc<[u8]>>>,
    blockstate_cache: RwLock<HashMap<String, Option<Arc<BlockstateJson>>>>,
    model_cache: RwLock<HashMap<String, Option<Arc<ModelJson>>>>,
    font_ascii: Option<RgbaImage>,
    font_nonlatin: Option<RgbaImage>,
    font_accented: Option<RgbaImage>,
}

impl AssetIndex {
    pub fn discover_and_load(
        vanilla_jar: Option<&Path>,
        world_dir: Option<&Path>,
        extra_mod_dirs: &[PathBuf],
    ) -> Self {
        let mut archive_paths: Vec<PathBuf> = Vec::new();
        let world_ver = world_dir.and_then(crate::anvil::read_level_version);
        let world_ver_name = world_ver.as_ref().and_then(|v| v.name.as_deref());
        let target_dv = world_ver.as_ref().and_then(|v| v.data_version);
        let target_tuple = world_ver_name.and_then(parse_mc_version_tuple);

        // 1. Vanilla JAR first (lowest base priority)
        if let Some(vj) = vanilla_jar {
            if vj.is_file() && is_valid_vanilla_jar(vj) {
                let jar_dv = read_jar_world_version(vj);
                let needs_newer = match (target_dv, jar_dv) {
                    (Some(tdv), Some(jdv)) => jdv < tdv,
                    _ => {
                        let jar_tuple = parse_mc_version_tuple(&vj.to_string_lossy());
                        matches!((target_tuple, jar_tuple), (Some(t), Some(j)) if j < t)
                    }
                };
                if needs_newer {
                    if let Some(better_vj) =
                        find_vanilla_jar_for_world_with_dir(world_ver.as_ref(), world_dir)
                    {
                        let better_ok = match (target_dv, read_jar_world_version(&better_vj)) {
                            (Some(tdv), Some(bdv)) => bdv >= tdv,
                            _ => {
                                let better_tuple = parse_mc_version_tuple(&better_vj.to_string_lossy());
                                target_tuple.is_none_or(|t| better_tuple >= Some(t))
                            }
                        };
                        if better_ok {
                            println!(
                                "[Assets] World version is {} (DataVersion={:?}); using matching vanilla JAR: {}",
                                world_ver_name.unwrap_or("unknown"),
                                target_dv,
                                better_vj.display()
                            );
                            archive_paths.push(better_vj);
                        } else {
                            archive_paths.push(vj.to_path_buf());
                        }
                    } else {
                        archive_paths.push(vj.to_path_buf());
                    }
                } else {
                    archive_paths.push(vj.to_path_buf());
                }
            } else if let Some(default_vj) =
                find_vanilla_jar_for_world_with_dir(world_ver.as_ref(), world_dir)
            {
                archive_paths.push(default_vj);
            } else if vj.is_file() {
                archive_paths.push(vj.to_path_buf());
            }
        } else if let Some(vj) = find_vanilla_jar_for_world_with_dir(world_ver.as_ref(), world_dir) {
            archive_paths.push(vj);
        }

        // 2. Discover mod directories and resource packs (Fabric / PrismLauncher / .minecraft)
        let mut mod_dirs: Vec<PathBuf> = Vec::new();
        let mut rp_zips: Vec<PathBuf> = Vec::new();
        let mut rp_dirs: Vec<PathBuf> = Vec::new();
        let mut world_rps: Vec<PathBuf> = Vec::new();

        if let Some(wd) = world_dir {
            for ancestor_idx in 1..=3 {
                let mut cur = wd.to_path_buf();
                let mut ok = true;
                for _ in 0..ancestor_idx {
                    if let Some(p) = cur.parent() {
                        cur = p.to_path_buf();
                    } else {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    let cand = cur.join("mods");
                    if cand.is_dir() && !mod_dirs.contains(&cand) {
                        mod_dirs.push(cand);
                    }
                    let rp_dir = cur.join("resourcepacks");
                    if rp_dir.is_dir() {
                        if let Ok(rd) = std::fs::read_dir(&rp_dir) {
                            for ent in rd.flatten() {
                                let p = ent.path();
                                if p.is_file()
                                    && p.extension()
                                        .and_then(|s| s.to_str())
                                        .map(|s| s.eq_ignore_ascii_case("zip"))
                                        .unwrap_or(false)
                                {
                                    if !rp_zips.contains(&p) {
                                        rp_zips.push(p);
                                    }
                                } else if p.is_dir() && p.join("assets").is_dir() && !rp_dirs.contains(&p) {
                                    rp_dirs.push(p);
                                }
                            }
                        }
                    }
                }
            }
            let world_rp = wd.join("resources.zip");
            if world_rp.is_file() {
                world_rps.push(world_rp);
            }
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let mc_mods = PathBuf::from(appdata).join(".minecraft").join("mods");
            if mc_mods.is_dir() && !mod_dirs.contains(&mc_mods) {
                mod_dirs.push(mc_mods);
            }
        }
        for d in extra_mod_dirs {
            if d.is_dir() && !mod_dirs.contains(d) {
                mod_dirs.push(d.clone());
            }
        }

        let mut mod_jars: Vec<PathBuf> = Vec::new();
        for dir in mod_dirs {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if p.is_file() {
                        if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                            if ext.eq_ignore_ascii_case("jar") || ext.eq_ignore_ascii_case("zip") {
                                mod_jars.push(p);
                            }
                        }
                    }
                }
            }
        }

        // Priority order (last-writer-wins):
        // 1. vanilla_jar -> 2. mod_jars -> 3. resourcepacks/*.zip -> 4. unpacked resourcepacks -> 5. world/resources.zip
        mod_jars.sort();
        rp_zips.sort();
        rp_dirs.sort();
        world_rps.sort();

        archive_paths.extend(mod_jars);
        archive_paths.extend(rp_zips);

        enum AssetSource {
            Zip(PathBuf),
            Dir(PathBuf),
        }
        let mut sources: Vec<AssetSource> = archive_paths.into_iter().map(AssetSource::Zip).collect();
        sources.extend(rp_dirs.into_iter().map(AssetSource::Dir));
        sources.extend(world_rps.into_iter().map(AssetSource::Zip));

        // Index all JARs/ZIPs/unpacked resourcepacks in parallel (central directory scan only — zero upfront decompression!)
        enum IndexedSource {
            Zip(Option<Mutex<ZipArchive<BufReader<File>>>>, Vec<(String, usize)>),
            Dir(Vec<(String, PathBuf)>),
        }

        let indexed: Vec<IndexedSource> = sources
            .par_iter()
            .map(|src| match src {
                AssetSource::Zip(p) => {
                    let (arc_opt, list) = index_relevant_zip_entries(p);
                    IndexedSource::Zip(arc_opt.map(Mutex::new), list)
                }
                AssetSource::Dir(d) => IndexedSource::Dir(index_relevant_dir_entries(d)),
            })
            .collect();

        let mut zip_sources = Vec::new();
        let mut entries = HashMap::new();
        for src in indexed {
            match src {
                IndexedSource::Zip(Some(arc_mtx), list) => {
                    let s_idx = zip_sources.len();
                    zip_sources.push(arc_mtx);
                    for (k, entry_idx) in list {
                        entries.insert(k, AssetLoc::Zip { source_idx: s_idx, entry_idx });
                    }
                }
                IndexedSource::Zip(None, _) => {}
                IndexedSource::Dir(list) => {
                    for (k, p) in list {
                        entries.insert(k, AssetLoc::File(p));
                    }
                }
            }
        }

        let mut index = Self {
            zip_sources,
            entries,
            bytes_cache: RwLock::new(HashMap::new()),
            blockstate_cache: RwLock::new(HashMap::new()),
            model_cache: RwLock::new(HashMap::new()),
            font_ascii: None,
            font_nonlatin: None,
            font_accented: None,
        };
        index.font_ascii = index.decode_png("assets/minecraft/textures/font/ascii.png");
        index.font_nonlatin = index.decode_png("assets/minecraft/textures/font/nonlatin_european.png");
        index.font_accented = index.decode_png("assets/minecraft/textures/font/accented.png");
        index
    }

    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            zip_sources: Vec::new(),
            entries: HashMap::new(),
            bytes_cache: RwLock::new(HashMap::new()),
            blockstate_cache: RwLock::new(HashMap::new()),
            model_cache: RwLock::new(HashMap::new()),
            font_ascii: None,
            font_nonlatin: None,
            font_accented: None,
        }
    }

    #[cfg(test)]
    pub fn insert_entry(&mut self, path: &str, data: Vec<u8>) {
        self.entries.insert(
            path.replace('\\', "/").to_lowercase(),
            AssetLoc::Inline(Arc::from(data)),
        );
    }

    fn decode_png(&self, path: &str) -> Option<RgbaImage> {
        let bytes = self.get_bytes(path)?;
        image::load_from_memory(&bytes).ok().map(|im| im.to_rgba8())
    }

    #[inline]
    pub fn has_asset(&self, key: &str) -> bool {
        let lower = if key.bytes().any(|b| b.is_ascii_uppercase()) {
            std::borrow::Cow::Owned(key.to_lowercase())
        } else {
            std::borrow::Cow::Borrowed(key)
        };
        self.entries.contains_key(lower.as_ref())
    }

    pub fn get_bytes(&self, key: &str) -> Option<Arc<[u8]>> {
        let lower = if key.bytes().any(|b| b.is_ascii_uppercase()) {
            std::borrow::Cow::Owned(key.to_lowercase())
        } else {
            std::borrow::Cow::Borrowed(key)
        };
        let loc = self.entries.get(lower.as_ref())?;
        #[cfg(test)]
        if let AssetLoc::Inline(arc) = loc {
            return Some(Arc::clone(arc));
        }
        if let Ok(cache) = self.bytes_cache.read() {
            if let Some(hit) = cache.get(lower.as_ref()) {
                return Some(Arc::clone(hit));
            }
        }
        let loaded: Arc<[u8]> = match loc {
            #[cfg(test)]
            AssetLoc::Inline(arc) => Arc::clone(arc),
            AssetLoc::File(p) => {
                const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
                if std::fs::metadata(p).ok()?.len() > MAX_ASSET_BYTES as u64 {
                    return None;
                }
                let data = std::fs::read(p).ok()?;
                Arc::from(data)
            }
            AssetLoc::Zip { source_idx, entry_idx } => {
                const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
                let mut guard = self.zip_sources.get(*source_idx)?.lock().ok()?;
                let entry = guard.by_index(*entry_idx).ok()?;
                let cap = (entry.size().min(4 * 1024 * 1024)) as usize;
                let mut buf = Vec::with_capacity(cap);
                let mut limited = entry.take((MAX_ASSET_BYTES as u64) + 1);
                limited.read_to_end(&mut buf).ok()?;
                if buf.len() > MAX_ASSET_BYTES {
                    return None;
                }
                Arc::from(buf)
            }
        };
        if let Ok(mut cache) = self.bytes_cache.write() {
            cache.insert(lower.into_owned(), Arc::clone(&loaded));
        }
        Some(loaded)
    }

    fn resolve_texture_bytes(&self, tex_ref: &str) -> Option<(String, Arc<[u8]>)> {
        let (ns, rel) = split_namespaced(tex_ref);
        let rel_trimmed = rel.trim_start_matches('/');
        let rel_no_tex = rel_trimmed.strip_prefix("textures/").unwrap_or(rel_trimmed);
        let rel_clean = rel_no_tex.strip_suffix(".png").unwrap_or(rel_no_tex);

        let base_name = rel_clean.rsplit('/').next().unwrap_or(rel_clean);
        let sub_after_first = rel_clean.split_once('/').map(|(_, r)| r);
        let mut candidate_subs = vec![rel_clean.to_string()];
        for folder in ["block", "blocks", "item", "items", "entity"] {
            let cand = format!("{}/{}", folder, base_name);
            if !candidate_subs.contains(&cand) {
                candidate_subs.push(cand);
            }
            if let Some(after) = sub_after_first {
                let cand2 = format!("{}/{}", folder, after);
                if !candidate_subs.contains(&cand2) {
                    candidate_subs.push(cand2);
                }
            }
        }

        for c_sub in &candidate_subs {
            let path = format!("assets/{}/textures/{}.png", ns, c_sub);
            if let Some(bytes) = self.get_bytes(&path) {
                return Some((path, bytes));
            }
            if ns != "minecraft" {
                let mc_path = format!("assets/minecraft/textures/{}.png", c_sub);
                if let Some(bytes) = self.get_bytes(&mc_path) {
                    return Some((mc_path, bytes));
                }
            }
        }
        None
    }

    pub fn load_animated_texture_frames(&self, tex_ref: &str) -> Option<AnimatedStrip> {
        let (png_path, bytes) = self.resolve_texture_bytes(tex_ref)?;
        let img = image::load_from_memory(&bytes).ok()?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        if w == 0 || h <= w || h % w != 0 {
            return None;
        }
        let strip_frames = (h / w) as usize;
        if strip_frames < 2 {
            return None;
        }

        let mcmeta_path = format!("{}.mcmeta", png_path);
        let mcmeta_val: Option<serde_json::Value> = self
            .get_bytes(&mcmeta_path)
            .and_then(|b| serde_json::from_slice(&b).ok());
        let anim_obj = mcmeta_val.as_ref().and_then(|v| v.get("animation"));

        let is_block_or_item = png_path.contains("/textures/block/")
            || png_path.contains("/textures/blocks/")
            || png_path.contains("/textures/item/")
            || png_path.contains("/textures/items/");
        if anim_obj.is_none() && !is_block_or_item {
            return None;
        }

        let default_ft = anim_obj
            .and_then(|a| a.get("frametime"))
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
            .max(1.0);
        let fps = (20.0 / default_ft as f32).clamp(1.0, 20.0);

        let mut frames = Vec::new();
        if let Some(arr) = anim_obj.and_then(|a| a.get("frames")).and_then(|v| v.as_array()) {
            for item in arr {
                let (idx, time_mul) = if let Some(n) = item.as_u64() {
                    (n as usize, 1usize)
                } else if let Some(obj) = item.as_object() {
                    let idx = obj.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    let t = obj.get("time").and_then(|v| v.as_u64()).unwrap_or(default_ft.round() as u64);
                    let mul = ((t as f64 / default_ft).round() as usize).clamp(1, 4);
                    (idx, mul)
                } else {
                    continue;
                };
                if idx < strip_frames {
                    let frame_img = image::imageops::crop_imm(&rgba, 0, (idx as u32) * w, w, w).to_image();
                    for _ in 0..time_mul {
                        if frames.len() >= 32 {
                            break;
                        }
                        frames.push(frame_img.clone());
                    }
                }
                if frames.len() >= 32 {
                    break;
                }
            }
        }
        if frames.is_empty() {
            for idx in 0..strip_frames.min(32) {
                frames.push(image::imageops::crop_imm(&rgba, 0, (idx as u32) * w, w, w).to_image());
            }
        }
        if frames.len() < 2 {
            return None;
        }
        Some(AnimatedStrip { frames, fps })
    }

    pub fn load_texture_image(&self, tex_ref: &str) -> Option<RgbaImage> {
        let (png_path, bytes) = self.resolve_texture_bytes(tex_ref)?;
        let img = image::load_from_memory(&bytes).ok()?;
        let mut rgba = img.to_rgba8();

        // Crop animated block/item sprite strips down to first square frame (W x W),
        // restricting non-mcmeta vertical strip cropping to textures/block/ and textures/item/.
        let mcmeta_path = format!("{}.mcmeta", png_path);
        let has_anim_meta = self
            .get_bytes(&mcmeta_path)
            .and_then(|b| std::str::from_utf8(&b).ok().map(|s| s.contains("animation")))
            .unwrap_or(false);
        let is_block_or_item = png_path.contains("/textures/block/")
            || png_path.contains("/textures/blocks/")
            || png_path.contains("/textures/item/")
            || png_path.contains("/textures/items/");
        let (w, h) = rgba.dimensions();
        if w > 0 && h > w && (has_anim_meta || (is_block_or_item && h % w == 0 && (h / w) >= 2)) {
            rgba = image::imageops::crop_imm(&rgba, 0, 0, w, w).to_image();
        }
        Some(rgba)
    }

    pub fn load_sign_entity_texture(&self, block_name: &str) -> Option<RgbaImage> {
        let (ns, short) = split_namespaced(block_name);
        let is_hanging = short.contains("hanging_sign");
        let wood = ["_wall_hanging_sign", "_hanging_sign", "_wall_sign", "_sign"]
            .iter()
            .find_map(|sfx| short.strip_suffix(sfx))
            .unwrap_or("oak");

        let mut paths = Vec::new();
        if is_hanging {
            paths.push(format!("assets/{}/textures/entity/signs/hanging/{}.png", ns, wood));
            if ns != "minecraft" {
                paths.push(format!("assets/minecraft/textures/entity/signs/hanging/{}.png", wood));
            }
        }
        paths.push(format!("assets/{}/textures/entity/signs/{}.png", ns, wood));
        if ns != "minecraft" {
            paths.push(format!("assets/minecraft/textures/entity/signs/{}.png", wood));
        }

        for p in &paths {
            if let Some(bytes) = self.get_bytes(p) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    return Some(img.to_rgba8());
                }
            }
        }
        None
    }

    pub fn find_door_textures(&self, block_name: &str) -> (Option<RgbaImage>, Option<RgbaImage>) {
        let (ns, short) = split_namespaced(block_name);
        let mut candidates_top = vec![
            format!("{}:block/{}_top", ns, short),
            format!("{}:blocks/{}_top", ns, short),
            format!("{}:block/{}_upper", ns, short),
            format!("{}:block/{}", ns, short),
        ];
        let mut candidates_bot = vec![
            format!("{}:block/{}_bottom", ns, short),
            format!("{}:blocks/{}_bottom", ns, short),
            format!("{}:block/{}_lower", ns, short),
            format!("{}:block/{}", ns, short),
        ];
        if ns != "minecraft" {
            candidates_top.push(format!("minecraft:block/{}_top", short));
            candidates_top.push(format!("minecraft:block/{}_upper", short));
            candidates_bot.push(format!("minecraft:block/{}_bottom", short));
            candidates_bot.push(format!("minecraft:block/{}_lower", short));
        }

        let mut top = None;
        for c in &candidates_top {
            if let Some(im) = self.load_texture_image(c) {
                top = Some(im);
                break;
            }
        }
        let mut bot = None;
        for c in &candidates_bot {
            if let Some(im) = self.load_texture_image(c) {
                bot = Some(im);
                break;
            }
        }
        (top, bot)
    }

    pub fn font_images(
        &self,
    ) -> (
        Option<&RgbaImage>,
        Option<&RgbaImage>,
        Option<&RgbaImage>,
    ) {
        (
            self.font_ascii.as_ref(),
            self.font_nonlatin.as_ref(),
            self.font_accented.as_ref(),
        )
    }

    pub fn resolve_block_model(&self, state: &BlockState) -> ResolvedBlockModel {
        let sname = state.short_name();
        if state.is_air {
            return ResolvedBlockModel::Empty;
        }
        if state.is_water_tag || sname == "lava" || sname == "flowing_water" || sname == "flowing_lava" {
            return build_fluid_model(state);
        }
        if sname == "snow" {
            let layers: f32 = state
                .prop("layers")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(1.0_f32)
                .clamp(1.0, 8.0);
            let mut faces = HashMap::new();
            let full_uv = Some([0.0, 0.0, 16.0, 16.0]);
            let side_uv = Some([0.0, 16.0 - layers * 2.0, 16.0, 16.0]);
            faces.insert(
                FaceDir::Up,
                ModelFace {
                    uv: full_uv,
                    texture: "minecraft:block/snow".to_string(),
                    rotation: 0,
                    tint_index: None,
                    cullface: if layers >= 8.0 { Some(FaceDir::Up) } else { None },
                },
            );
            faces.insert(
                FaceDir::Down,
                ModelFace {
                    uv: full_uv,
                    texture: "minecraft:block/snow".to_string(),
                    rotation: 0,
                    tint_index: None,
                    cullface: Some(FaceDir::Down),
                },
            );
            for dir in [FaceDir::North, FaceDir::South, FaceDir::East, FaceDir::West] {
                faces.insert(
                    dir,
                    ModelFace {
                        uv: side_uv,
                        texture: "minecraft:block/snow".to_string(),
                        rotation: 0,
                        tint_index: None,
                        cullface: Some(dir),
                    },
                );
            }
            return ResolvedBlockModel::Elements(vec![ModelElement {
                from: [0.0, 0.0, 0.0],
                to: [16.0, layers * 2.0, 16.0],
                rotation: None,
                faces,
                rot_x: 0,
                rot_y: 0,
                rot_z: 0,
                uvlock: false,
            }]);
        }

        let (ns, short) = split_namespaced(&state.name);
        let bs_key = format!("assets/{}/blockstates/{}.json", ns, short);

        let bs_opt: Option<Arc<BlockstateJson>> = {
            if let Ok(cache) = self.blockstate_cache.read() {
                if let Some(hit) = cache.get(&bs_key) {
                    hit.clone()
                } else {
                    drop(cache);
                    let parsed = self
                        .get_bytes(&bs_key)
                        .and_then(|bytes| serde_json::from_slice::<BlockstateJson>(&bytes).ok())
                        .map(Arc::new);
                    if let Ok(mut cache) = self.blockstate_cache.write() {
                        cache.insert(bs_key.clone(), parsed.clone());
                    }
                    parsed
                }
            } else {
                None
            }
        };

        let mut applies: Vec<(String, i32, i32, i32, bool)> = Vec::new();
        if let Some(bs) = bs_opt {
            if let Some(variants) = &bs.variants {
                if let Some(val) = match_best_variant(variants, &state.props) {
                    collect_variant_apply(val, &mut applies);
                }
            } else if let Some(cases) = &bs.multipart {
                for case in cases {
                    let ok = match &case.when {
                        Some(w) => eval_multipart_when(w, &state.props),
                        None => true,
                    };
                    if ok {
                        collect_variant_apply(&case.apply, &mut applies);
                    }
                }
            }
        } else {
            // Fallback try direct block model
            applies.push((format!("{}:block/{}", ns, short), 0, 0, 0, false));
        }

        if applies.is_empty() {
            return fallback_model_for_special(state, self);
        }

        let mut all_elements: Vec<ModelElement> = Vec::new();
        let mut all_obj_tris: Vec<ObjTri> = Vec::new();

        for (model_ref, rx, ry, rz, uvlock) in applies {
            if model_ref.ends_with(".obj") {
                self.load_obj_model(&model_ref, rx, ry, rz, &mut all_obj_tris);
                continue;
            }
            let resolved = self.resolve_model_json_chain(&model_ref, rx, ry, rz, uvlock);
            match resolved {
                ResolvedBlockModel::Elements(els) => all_elements.extend(els),
                ResolvedBlockModel::Obj(tris) => all_obj_tris.extend(tris),
                ResolvedBlockModel::Empty => {}
            }
        }

        if sname == "bell" {
            all_elements.extend(build_bell_body_elements(self));
        }

        if !all_obj_tris.is_empty() {
            ResolvedBlockModel::Obj(all_obj_tris)
        } else if !all_elements.is_empty() {
            ResolvedBlockModel::Elements(all_elements)
        } else {
            fallback_model_for_special(state, self)
        }
    }

    fn get_cached_model(&self, path: &str) -> Option<Arc<ModelJson>> {
        if let Ok(cache) = self.model_cache.read() {
            if let Some(hit) = cache.get(path) {
                return hit.clone();
            }
        }
        let parsed = self
            .get_bytes(path)
            .and_then(|bytes| serde_json::from_slice::<ModelJson>(&bytes).ok())
            .map(Arc::new);
        if let Ok(mut cache) = self.model_cache.write() {
            cache.insert(path.to_string(), parsed.clone());
        }
        parsed
    }

    fn resolve_model_json_chain(
        &self,
        model_ref: &str,
        rot_x: i32,
        rot_y: i32,
        rot_z: i32,
        uvlock: bool,
    ) -> ResolvedBlockModel {
        let mut chain: Vec<Arc<ModelJson>> = Vec::new();
        let mut cur_ref = Some(model_ref.to_string());
        let mut depth = 0;

        while let Some(r) = cur_ref {
            if depth >= 16 {
                break;
            }
            depth += 1;
            let (ns, rel) = split_namespaced(&r);
            let rel_clean = rel.trim_start_matches('/').trim_end_matches(".json");
            let path = if rel_clean.starts_with("models/") {
                format!("assets/{}/{}.json", ns, rel_clean)
            } else {
                format!("assets/{}/models/{}.json", ns, rel_clean)
            };
            let Some(mj) = self.get_cached_model(&path) else {
                break;
            };
            cur_ref = mj.parent.clone();
            chain.push(mj);
        }

        if chain.is_empty() {
            return ResolvedBlockModel::Empty;
        }

        // Check if leaf/parent references a Forge/NeoForge OBJ loader
        for mj in &chain {
            if let Some(obj_path) = &mj.model {
                if obj_path.to_lowercase().ends_with(".obj") {
                    let mut tris = Vec::new();
                    self.load_obj_model(obj_path, rot_x, rot_y, rot_z, &mut tris);
                    if !tris.is_empty() {
                        return ResolvedBlockModel::Obj(tris);
                    }
                }
            }
        }

        // Merge textures from root (last) down to leaf (first)
        let mut textures: HashMap<String, String> = HashMap::new();
        let mut raw_elements: Option<&Vec<RawElement>> = None;
        for mj in chain.iter().rev() {
            for (k, v) in &mj.textures {
                textures.insert(k.clone(), v.clone());
            }
            if let Some(els) = &mj.elements {
                raw_elements = Some(els);
            }
        }

        let Some(raw_els) = raw_elements else {
            return ResolvedBlockModel::Empty;
        };

        let mut elements = Vec::with_capacity(raw_els.len());
        for re in raw_els {
            let mut faces = HashMap::new();
            for (dir_str, rf) in &re.faces {
                let Some(fdir) = FaceDir::from_str(dir_str) else {
                    continue;
                };
                let resolved_tex = resolve_texture_var(&rf.texture, &textures);
                let cull = rf.cullface.as_deref().and_then(FaceDir::from_str);
                faces.insert(
                    fdir,
                    ModelFace {
                        uv: rf.uv,
                        texture: resolved_tex,
                        rotation: rf.rotation,
                        tint_index: rf.tintindex,
                        cullface: cull,
                    },
                );
            }
            let rotation = re.rotation.as_ref().map(|rr| {
                let euler_xyz = if rr.axis.is_none() && (rr.x != 0.0 || rr.y != 0.0 || rr.z != 0.0) {
                    Some([rr.x, rr.y, rr.z])
                } else {
                    None
                };
                ModelRotation {
                    origin: rr.origin,
                    axis: rr
                        .axis
                        .as_deref()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('y'),
                    angle: rr.angle,
                    rescale: rr.rescale,
                    euler_xyz,
                }
            });
            elements.push(ModelElement {
                from: re.from,
                to: re.to,
                rotation,
                faces,
                rot_x,
                rot_y,
                rot_z,
                uvlock,
            });
        }
        ResolvedBlockModel::Elements(elements)
    }

    fn load_obj_model(
        &self,
        obj_ref: &str,
        rot_x: i32,
        rot_y: i32,
        rot_z: i32,
        out: &mut Vec<ObjTri>,
    ) {
        let (ns, rel) = split_namespaced(obj_ref);
        let rel_clean = rel.trim_start_matches('/');
        let candidates = [
            format!("assets/{}/{}", ns, rel_clean),
            format!("assets/{}/models/{}", ns, rel_clean),
            format!("assets/{}/models/block/{}", ns, rel_clean),
        ];
        let mut obj_bytes = None;
        let mut obj_dir = format!("assets/{}/models/block", ns);
        for c in &candidates {
            if let Some(b) = self.get_bytes(c) {
                obj_bytes = Some(b);
                if let Some((dir, _)) = c.rsplit_once('/') {
                    obj_dir = dir.to_string();
                }
                break;
            }
        }
        let Some(bytes) = obj_bytes else {
            return;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return;
        };

        let mut mtl_map: HashMap<String, String> = HashMap::new();
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut normals: Vec<[f32; 3]> = Vec::new();
        let mut cur_tex = "minecraft:block/iron_block".to_string();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("mtllib") => {
                    if let Some(mtl_file) = parts.next() {
                        let mtl_key = format!("{}/{}", obj_dir, mtl_file);
                        if let Some(mb) = self.get_bytes(&mtl_key) {
                            if let Ok(mtext) = std::str::from_utf8(&mb) {
                                parse_mtl(mtext, &ns, &mut mtl_map);
                            }
                        }
                    }
                }
                Some("usemtl") => {
                    if let Some(mat) = parts.next() {
                        if let Some(t) = mtl_map.get(mat) {
                            cur_tex = t.clone();
                        } else if mat.contains(':') {
                            cur_tex = mat.to_string();
                        }
                    }
                }
                Some("v") => {
                    let x = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    let y = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    let z = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    positions.push([x, y, z]);
                }
                Some("vt") => {
                    let u = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    let v = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    uvs.push([u, v]);
                }
                Some("vn") => {
                    let x = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    let y = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0);
                    let z = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    normals.push([x, y, z]);
                }
                Some("f") => {
                    let verts: Vec<(usize, usize, usize)> = parts
                        .filter_map(|tok| parse_obj_face_vertex(tok, positions.len(), uvs.len(), normals.len()))
                        .collect();
                    if verts.len() >= 3 {
                        for k in 1..(verts.len() - 1) {
                            let tri = [verts[0], verts[k], verts[k + 1]];
                            let p0 = rotate_block_point(positions[tri[0].0], rot_x, rot_y, rot_z);
                            let p1 = rotate_block_point(positions[tri[1].0], rot_x, rot_y, rot_z);
                            let p2 = rotate_block_point(positions[tri[2].0], rot_x, rot_y, rot_z);
                            let uv0 = uvs.get(tri[0].1).copied().unwrap_or([0.0, 0.0]);
                            let uv1 = uvs.get(tri[1].1).copied().unwrap_or([0.0, 0.0]);
                            let uv2 = uvs.get(tri[2].1).copied().unwrap_or([0.0, 0.0]);
                            let n_fallback = compute_tri_normal(p0, p1, p2);
                            let n0 = normals
                                .get(tri[0].2)
                                .map(|&n| rotate_block_normal(n, rot_x, rot_y, rot_z))
                                .unwrap_or(n_fallback);
                            let n1 = normals
                                .get(tri[1].2)
                                .map(|&n| rotate_block_normal(n, rot_x, rot_y, rot_z))
                                .unwrap_or(n_fallback);
                            let n2 = normals
                                .get(tri[2].2)
                                .map(|&n| rotate_block_normal(n, rot_x, rot_y, rot_z))
                                .unwrap_or(n_fallback);
                            out.push(ObjTri {
                                positions: [p0, p1, p2],
                                uvs: [uv0, uv1, uv2],
                                normals: [n0, n1, n2],
                                texture: cur_tex.clone(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn parse_mtl(text: &str, default_ns: &str, out: &mut HashMap<String, String>) {
    let mut cur_mat = String::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("newmtl") => {
                cur_mat = parts.next().unwrap_or("").to_string();
            }
            Some("map_Kd") if !cur_mat.is_empty() => {
                if let Some(tex) = parts.next() {
                    let clean = tex.strip_prefix('#').unwrap_or(tex);
                    let canon = if clean.contains(':') {
                        clean.to_string()
                    } else {
                        format!("{}:{}", default_ns, clean)
                    };
                    out.insert(cur_mat.clone(), canon);
                }
            }
            _ => {}
        }
    }
}

fn parse_obj_face_vertex(
    tok: &str,
    v_len: usize,
    vt_len: usize,
    vn_len: usize,
) -> Option<(usize, usize, usize)> {
    let mut parts = tok.split('/');
    let vi = parse_obj_idx(parts.next()?, v_len)?;
    let vti = parts.next().and_then(|s| parse_obj_idx(s, vt_len)).unwrap_or(usize::MAX);
    let vni = parts.next().and_then(|s| parse_obj_idx(s, vn_len)).unwrap_or(usize::MAX);
    Some((vi, vti, vni))
}

fn parse_obj_idx(s: &str, len: usize) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let idx: isize = s.parse().ok()?;
    if idx > 0 {
        let u = (idx - 1) as usize;
        (u < len).then_some(u)
    } else if idx < 0 {
        let u = (len as isize + idx) as usize;
        (u < len).then_some(u)
    } else {
        None
    }
}

fn rotate_block_point(p: [f32; 3], rot_x: i32, rot_y: i32, rot_z: i32) -> [f32; 3] {
    let mut x = p[0] - 0.5;
    let mut y = p[1] - 0.5;
    let mut z = p[2] - 0.5;
    let rx_steps = rot_x.rem_euclid(360) / 90;
    for _ in 0..rx_steps {
        let (ny, nz) = (z, -y);
        y = ny;
        z = nz;
    }
    let ry_steps = rot_y.rem_euclid(360) / 90;
    for _ in 0..ry_steps {
        let (nx, nz) = (-z, x);
        x = nx;
        z = nz;
    }
    let rz_steps = rot_z.rem_euclid(360) / 90;
    for _ in 0..rz_steps {
        let (nx, ny) = (y, -x);
        x = nx;
        y = ny;
    }
    [x + 0.5, y + 0.5, z + 0.5]
}

fn rotate_block_normal(n: [f32; 3], rot_x: i32, rot_y: i32, rot_z: i32) -> [f32; 3] {
    let mut x = n[0];
    let mut y = n[1];
    let mut z = n[2];
    let rx_steps = rot_x.rem_euclid(360) / 90;
    for _ in 0..rx_steps {
        let (ny, nz) = (z, -y);
        y = ny;
        z = nz;
    }
    let ry_steps = rot_y.rem_euclid(360) / 90;
    for _ in 0..ry_steps {
        let (nx, nz) = (-z, x);
        x = nx;
        z = nz;
    }
    let rz_steps = rot_z.rem_euclid(360) / 90;
    for _ in 0..rz_steps {
        let (nx, ny) = (y, -x);
        x = nx;
        y = ny;
    }
    [x, y, z]
}

fn compute_tri_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let nx = u[1] * v[2] - u[2] * v[1];
    let ny = u[2] * v[0] - u[0] * v[2];
    let nz = u[0] * v[1] - u[1] * v[0];
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
    [nx / len, ny / len, nz / len]
}

pub fn split_namespaced(r: &str) -> (String, String) {
    if let Some((ns, p)) = r.split_once(':') {
        (ns.to_lowercase(), p.to_string())
    } else {
        ("minecraft".to_string(), r.to_string())
    }
}

fn resolve_texture_var(raw: &str, textures: &HashMap<String, String>) -> String {
    let mut cur = raw.trim().to_string();
    for _ in 0..8 {
        if let Some(key) = cur.strip_prefix('#') {
            if let Some(next_val) = textures.get(key) {
                cur = next_val.clone();
            } else if key != "all" && textures.contains_key("all") {
                cur = textures["all"].clone();
            } else if key != "particle" && textures.contains_key("particle") {
                cur = textures["particle"].clone();
            } else {
                cur = key.to_string();
                break;
            }
        } else {
            break;
        }
    }
    let clean = cur.trim_start_matches('#');
    let (ns, p) = split_namespaced(clean);
    format!("{}:{}", ns, p)
}

fn canonical_prop_default_score(pk: &str, pv: &str) -> isize {
    match (pk, pv) {
        ("facing", "north")
        | ("half", "bottom" | "lower")
        | ("shape", "straight" | "north_south")
        | ("axis", "y")
        | ("face", "wall")
        | ("type", "bottom" | "single")
        | ("hinge", "left")
        | ("attachment", "floor")
        | ("vertical_direction", "up")
        | ("thickness", "tip")
        | ("mode", "compare")
        | ("up", "true")
        | ("bottom", "true")
        | ("side_chain", "unconnected")
        | ("lit", "false")
        | ("open", "false")
        | ("powered", "false")
        | ("waterlogged", "false")
        | ("hanging", "false")
        | ("in_wall", "false")
        | ("extended", "false")
        | ("triggered", "false")
        | ("locked", "false")
        | ("has_book", "false")
        | ("occupied", "false")
        | ("part", "foot")
        | ("attached", "false")
        | ("cracked", "false")
        | ("delay", "1")
        | ("flower_amount", "1")
        | ("segment_amount", "1")
        | ("age", "0") => 2,
        (_, "none" | "false" | "0" | "1") => 1,
        _ => 0,
    }
}

fn match_best_variant<'a>(
    variants: &'a HashMap<String, serde_json::Value>,
    props: &BTreeMap<String, String>,
) -> Option<&'a serde_json::Value> {
    use std::cmp::Reverse;

    if variants.len() == 1 {
        if let Some(v) = variants.values().next() {
            return Some(v);
        }
    }

    variants
        .iter()
        .map(|(k, val)| {
            let mut mismatches = 0usize;
            let mut matched = 0usize;
            let mut default_score = 0isize;
            if !k.is_empty() {
                for pair in k.split(',') {
                    let Some((pk, pv)) = pair.split_once('=') else {
                        continue;
                    };
                    let pk = pk.trim();
                    let pv = pv.trim();
                    default_score += canonical_prop_default_score(pk, pv);
                    if let Some(actual) = props.get(pk) {
                        if actual == pv {
                            matched += 1;
                        } else {
                            mismatches += 1;
                        }
                    }
                }
            }
            ((mismatches, Reverse(matched), Reverse(default_score), k.as_str()), val)
        })
        .min_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, v)| v)
}

fn eval_multipart_when(when: &serde_json::Value, props: &BTreeMap<String, String>) -> bool {
    let Some(obj) = when.as_object() else {
        return true;
    };
    if let Some(or_arr) = obj.get("OR").and_then(|v| v.as_array()) {
        return or_arr.iter().any(|sub| eval_multipart_when(sub, props));
    }
    if let Some(and_arr) = obj.get("AND").and_then(|v| v.as_array()) {
        return and_arr.iter().all(|sub| eval_multipart_when(sub, props));
    }
    for (k, v) in obj {
        let expected_str = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => continue,
        };
        let actual = props.get(k).map(|s| s.as_str()).unwrap_or("");
        if !expected_str.split('|').any(|opt| opt == actual) {
            return false;
        }
    }
    true
}

fn collect_variant_apply(val: &serde_json::Value, out: &mut Vec<(String, i32, i32, i32, bool)>) {
    let item = match val {
        serde_json::Value::Array(arr) => arr.first(),
        other => Some(other),
    };
    let Some(obj) = item.and_then(|v| v.as_object()) else {
        return;
    };
    let Some(model) = obj.get("model").and_then(|v| v.as_str()) else {
        return;
    };
    let rx = obj.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let ry = obj.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let rz = obj.get("z").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let uvlock = obj.get("uvlock").and_then(|v| v.as_bool()).unwrap_or(false);
    out.push((model.to_string(), rx, ry, rz, uvlock));
}

fn build_fluid_model(state: &BlockState) -> ResolvedBlockModel {
    let sname = state.short_name();
    let is_water = state.is_water_tag || sname.contains("water");
    let still_tex = if is_water {
        "minecraft:block/water_still"
    } else {
        "minecraft:block/lava_still"
    };
    let flow_tex = if is_water {
        "minecraft:block/water_flow"
    } else {
        "minecraft:block/lava_flow"
    };
    let level: i32 = state
        .prop("level")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let top_px = if level >= 8 {
        16.0
    } else {
        (14.2 - (level as f32 * 1.7)).clamp(2.0, 14.5)
    };
    let tint = if is_water { Some(0) } else { None };
    let mut faces = HashMap::new();
    for dir in [
        FaceDir::Up,
        FaceDir::Down,
        FaceDir::North,
        FaceDir::South,
        FaceDir::East,
        FaceDir::West,
    ] {
        let (tex, uv) = match dir {
            FaceDir::Up | FaceDir::Down => (still_tex, [0.0, 0.0, 16.0, 16.0]),
            _ => (flow_tex, [0.0, 16.0 - top_px, 16.0, 16.0]),
        };
        faces.insert(
            dir,
            ModelFace {
                uv: Some(uv),
                texture: tex.to_string(),
                rotation: 0,
                tint_index: tint,
                cullface: Some(dir),
            },
        );
    }
    ResolvedBlockModel::Elements(vec![ModelElement {
        from: [0.0, 0.0, 0.0],
        to: [16.0, top_px, 16.0],
        rotation: None,
        faces,
        rot_x: 0,
        rot_y: 0,
        rot_z: 0,
        uvlock: false,
    }])
}

fn fallback_model_for_special(state: &BlockState, assets: &AssetIndex) -> ResolvedBlockModel {
    let s = state.short_name();
    if s.contains("chest") || s.contains("shulker") {
        return single_box_model([1.0, 0.0, 1.0], [15.0, 14.0, 15.0], "minecraft:block/oak_planks");
    }
    if s.contains("bed") {
        return single_box_model([0.0, 0.0, 0.0], [16.0, 9.0, 16.0], "minecraft:block/red_wool");
    }
    if s.contains("banner") {
        return single_box_model([1.0, 0.0, 7.0], [15.0, 16.0, 9.0], "minecraft:block/white_wool");
    }
    // Check if there's an eponymous texture in block/ or blocks/
    let (ns, short) = split_namespaced(&state.name);
    let cand_tex = format!("{}:block/{}", ns, short);
    if assets.has_asset(&format!("assets/{}/textures/block/{}.png", ns, short))
        || assets.has_asset(&format!("assets/{}/textures/blocks/{}.png", ns, short))
    {
        return single_box_model([0.0, 0.0, 0.0], [16.0, 16.0, 16.0], &cand_tex);
    }
    ResolvedBlockModel::Empty
}

fn single_box_model(from: [f32; 3], to: [f32; 3], tex: &str) -> ResolvedBlockModel {
    let mut faces = HashMap::new();
    for dir in [
        FaceDir::Up,
        FaceDir::Down,
        FaceDir::North,
        FaceDir::South,
        FaceDir::East,
        FaceDir::West,
    ] {
        faces.insert(
            dir,
            ModelFace {
                uv: Some([0.0, 0.0, 16.0, 16.0]),
                texture: tex.to_string(),
                rotation: 0,
                tint_index: None,
                cullface: None,
            },
        );
    }
    ResolvedBlockModel::Elements(vec![ModelElement {
        from,
        to,
        rotation: None,
        faces,
        rot_x: 0,
        rot_y: 0,
        rot_z: 0,
        uvlock: false,
    }])
}

/// Builds the hanging golden bell body & flared rim rendered by Minecraft's `BellRenderer`
/// (absent from the blockstate JSON which only defines the stone posts and wooden bar).
fn build_bell_body_elements(assets: &AssetIndex) -> Vec<ModelElement> {
    let pick_tex = |rel: &str| -> String {
        let path = format!("assets/minecraft/textures/block/{}.png", rel);
        let path_legacy = format!("assets/minecraft/textures/blocks/{}.png", rel);
        let tex_ref = format!("minecraft:block/{}", rel);
        if assets.has_asset(&path) || assets.has_asset(&path_legacy) {
            tex_ref
        } else {
            "minecraft:block/gold_block".to_string()
        }
    };
    let top_tex = pick_tex("bell_top");
    let side_tex = pick_tex("bell_side");
    let bottom_tex = pick_tex("bell_bottom");

    let make_box = |from: [f32; 3], to: [f32; 3]| -> ModelElement {
        let mut faces = HashMap::new();
        for dir in [
            FaceDir::Up,
            FaceDir::Down,
            FaceDir::North,
            FaceDir::South,
            FaceDir::East,
            FaceDir::West,
        ] {
            let tex = match dir {
                FaceDir::Up => &top_tex,
                FaceDir::Down => &bottom_tex,
                _ => &side_tex,
            };
            faces.insert(
                dir,
                ModelFace {
                    uv: Some([0.0, 0.0, 16.0, 16.0]),
                    texture: tex.clone(),
                    rotation: 0,
                    tint_index: None,
                    cullface: None,
                },
            );
        }
        ModelElement {
            from,
            to,
            rotation: None,
            faces,
            rot_x: 0,
            rot_y: 0,
            rot_z: 0,
            uvlock: false,
        }
    };

    vec![
        // Upper bell body (6x7x6 px hanging directly beneath the bar at y=13)
        make_box([5.0, 6.0, 5.0], [11.0, 13.0, 11.0]),
        // Flared lower rim (8x2x8 px)
        make_box([4.0, 4.0, 4.0], [12.0, 6.0, 12.0]),
    ]
}

fn is_relevant_asset_path(name: &str) -> bool {
    if !name.starts_with("assets/") {
        return false;
    }
    let valid_ext = name.ends_with(".json")
        || name.ends_with(".png")
        || name.ends_with(".mcmeta")
        || name.ends_with(".obj")
        || name.ends_with(".mtl");
    if !valid_ext {
        return false;
    }
    if name.contains("/blockstates/") {
        return true;
    }
    if name.contains("/models/") {
        return !name.contains("/models/item/");
    }
    if name.contains("/font/") {
        return true;
    }
    if name.contains("/textures/") {
        if name.contains("/textures/gui/")
            || name.contains("/textures/particle/")
            || name.contains("/textures/mob_effect/")
            || name.contains("/textures/painting/")
            || name.contains("/textures/models/armor/")
            || name.contains("/textures/colormap/")
            || name.contains("/textures/misc/")
            || name.contains("/textures/effect/")
        {
            return false;
        }
        if name.contains("/textures/entity/") && !name.contains("/textures/entity/signs/") {
            return false;
        }
        return true;
    }
    false
}

type IndexedZipEntries = (Option<ZipArchive<BufReader<File>>>, Vec<(String, usize)>);

fn index_relevant_zip_entries(path: &Path) -> IndexedZipEntries {
    let Ok(file) = File::open(path) else {
        return (None, Vec::new());
    };
    let Ok(archive) = ZipArchive::new(BufReader::new(file)) else {
        return (None, Vec::new());
    };
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let Some(raw_name) = archive.name_for_index(i) else {
            continue;
        };
        if raw_name.ends_with('/') || raw_name.ends_with('\\') {
            continue;
        }
        let name = raw_name.replace('\\', "/").to_lowercase();
        if is_relevant_asset_path(&name) {
            out.push((name, i));
        }
    }
    if out.is_empty() {
        (None, out)
    } else {
        (Some(archive), out)
    }
}

fn index_relevant_dir_entries(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let assets_root = root.join("assets");
    if !assets_root.is_dir() {
        return out;
    }
    let mut stack = vec![assets_root];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        children.sort();
        for p in children {
            if p.is_dir() {
                stack.push(p);
            } else if p.is_file() {
                let Ok(rel) = p.strip_prefix(root) else {
                    continue;
                };
                let name = rel.to_string_lossy().replace('\\', "/").to_lowercase();
                if is_relevant_asset_path(&name) {
                    out.push((name, p));
                }
            }
        }
    }
    out
}

pub fn is_valid_vanilla_jar(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(mut archive) = ZipArchive::new(BufReader::new(file)) else {
        return false;
    };
    if archive
        .by_name("assets/minecraft/textures/block/stone.png")
        .is_ok()
    {
        return true;
    }
    let has_legacy = archive
        .by_name("assets/minecraft/textures/blocks/stone.png")
        .is_ok();
    has_legacy
}

pub fn parse_mc_version_tuple(s: &str) -> Option<(u32, u32, u32)> {
    let mut best: Option<(u32, u32, u32)> = None;
    for raw_tok in s.split(['/', '\\', '-', '_', ' ']) {
        let tok = raw_tok
            .trim()
            .trim_end_matches(".jar")
            .trim_end_matches(".JAR");
        if tok.is_empty() {
            continue;
        }
        let parts: Vec<&str> = tok.split('.').collect();
        if !(parts.len() == 2 || parts.len() == 3) {
            continue;
        }
        if !parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let Ok(maj) = parts[0].parse::<u32>() else {
            continue;
        };
        let Ok(min) = parts[1].parse::<u32>() else {
            continue;
        };
        let pat = if parts.len() == 3 {
            let Ok(v) = parts[2].parse::<u32>() else {
                continue;
            };
            v
        } else {
            0
        };
        let is_mc = (maj == 1 && (13..=99).contains(&min) && pat <= 99)
            || ((26..=99).contains(&maj) && min <= 99 && pat <= 99);
        if is_mc {
            best = Some((maj, min, pat));
        }
    }
    best
}

pub fn read_jar_world_version(path: &Path) -> Option<i32> {
    const MAX_VERSION_JSON_BYTES: usize = 1024 * 1024;
    let file = File::open(path).ok()?;
    let mut archive = ZipArchive::new(BufReader::new(file)).ok()?;
    let entry = archive.by_name("version.json").ok()?;
    let mut buf = Vec::with_capacity((entry.size().min(64 * 1024)) as usize);
    let mut limited = std::io::Read::take(entry, (MAX_VERSION_JSON_BYTES as u64) + 1);
    std::io::Read::read_to_end(&mut limited, &mut buf).ok()?;
    if buf.len() > MAX_VERSION_JSON_BYTES {
        return None;
    }
    let val: serde_json::Value = serde_json::from_slice(&buf).ok()?;
    val.get("world_version")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
}

pub fn select_best_vanilla_jar(
    candidates: Vec<(String, PathBuf)>,
    world_ver: Option<&crate::anvil::WorldVersionInfo>,
) -> Option<PathBuf> {
    let target_dv = world_ver.and_then(|v| v.data_version);
    let target_ver = world_ver
        .and_then(|v| v.name.as_deref())
        .and_then(parse_mc_version_tuple);

    type JarScoreKey = (u8, u8, i64, (u32, u32, u32));
    // Pre-filter to valid vanilla JARs so we only read `version.json` on real JARs.
    let mut scored: Vec<(JarScoreKey, String, PathBuf)> = candidates
        .into_iter()
        .filter(|(_, p)| is_valid_vanilla_jar(p))
        .map(|(name, path)| {
            let lower = name.to_lowercase();
            let full_lower = path.to_string_lossy().to_lowercase();
            let is_loader = ["fabric", "forge", "neoforge", "quilt", "optifine"]
                .iter()
                .any(|k| lower.contains(k) || full_lower.contains(k));
            let non_loader_tier = if is_loader { 0 } else { 1 };
            let ver = parse_mc_version_tuple(&name)
                .or_else(|| parse_mc_version_tuple(&path.to_string_lossy()))
                .unwrap_or((0, 0, 0));

            let key = if let Some(tdv) = target_dv {
                if let Some(jdv) = read_jar_world_version(&path) {
                    let (match_tier, tie_bias) = if jdv == tdv {
                        (4u8, 0i64)
                    } else if jdv > tdv {
                        (3u8, -((jdv - tdv) as i64))
                    } else {
                        (1u8, jdv as i64)
                    };
                    (non_loader_tier, match_tier, tie_bias, ver)
                } else {
                    let ver_score = (ver.0 as i64 * 10000) + (ver.1 as i64 * 100) + ver.2 as i64;
                    (non_loader_tier, 2u8, ver_score, ver)
                }
            } else if let Some(target) = target_ver {
                let (match_tier, tie_bias) = if ver == target {
                    (4u8, 0i64)
                } else if ver.0 == target.0 && ver.1 == target.1 && ver.2 >= target.2 {
                    (3u8, -((ver.2 - target.2) as i64))
                } else if ver > target {
                    (2u8, -((ver.0 as i64 * 10000) + (ver.1 as i64 * 100) + ver.2 as i64))
                } else if ver.0 == target.0 && ver.1 == target.1 {
                    (1u8, ver.2 as i64)
                } else {
                    (0u8, (ver.0 as i64 * 10000) + (ver.1 as i64 * 100) + ver.2 as i64)
                };
                (non_loader_tier, match_tier, tie_bias, ver)
            } else {
                let ver_score = (ver.0 as i64 * 10000) + (ver.1 as i64 * 100) + ver.2 as i64;
                (non_loader_tier, 0u8, ver_score, ver)
            };
            (key, name, path)
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    scored.into_iter().map(|(_, _, p)| p).next()
}

pub fn find_vanilla_jar_for_world_with_dir(
    world_ver: Option<&crate::anvil::WorldVersionInfo>,
    world_dir: Option<&Path>,
) -> Option<PathBuf> {
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();

    let mut version_roots: Vec<PathBuf> = Vec::new();
    let mut lib_roots: Vec<PathBuf> = crate::instances::discover_launcher_library_roots(world_dir);

    if let Some(appdata) = std::env::var_os("APPDATA") {
        let appdata_dir = PathBuf::from(&appdata);
        version_roots.push(appdata_dir.join(".minecraft").join("versions"));
        version_roots.push(appdata_dir.join("ModrinthApp").join("meta").join("versions"));
        lib_roots.push(
            appdata_dir
                .join(".minecraft")
                .join("libraries")
                .join("com")
                .join("mojang")
                .join("minecraft"),
        );
        lib_roots.push(
            appdata_dir
                .join("PrismLauncher")
                .join("libraries")
                .join("com")
                .join("mojang")
                .join("minecraft"),
        );
        lib_roots.push(
            appdata_dir
                .join("MultiMC")
                .join("libraries")
                .join("com")
                .join("mojang")
                .join("minecraft"),
        );
    }
    if let Some(userprofile) = std::env::var_os("USERPROFILE") {
        let up_dir = PathBuf::from(&userprofile);
        version_roots.push(
            up_dir
                .join("curseforge")
                .join("minecraft")
                .join("Install")
                .join("versions"),
        );
    }

    for vroot in version_roots {
        if !vroot.is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&vroot) {
            for ent in rd.flatten() {
                let dir = ent.path();
                if dir.is_dir() {
                    if let Some(name) = dir.file_name().and_then(|s| s.to_str()) {
                        let jar = dir.join(format!("{}.jar", name));
                        if jar.is_file() {
                            candidates.push((name.to_string(), jar));
                        } else {
                            let client_jar = dir.join("client.jar");
                            if client_jar.is_file() {
                                candidates.push((name.to_string(), client_jar));
                            }
                        }
                    }
                }
            }
        }
    }

    for lroot in lib_roots {
        if !lroot.is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&lroot) {
            for ent in rd.flatten() {
                let dir = ent.path();
                if dir.is_dir() {
                    let ver_name = dir
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    if let Ok(sub_rd) = std::fs::read_dir(&dir) {
                        for sub_ent in sub_rd.flatten() {
                            let p = sub_ent.path();
                            if p.is_file()
                                && p.extension()
                                    .and_then(|s| s.to_str())
                                    .map(|s| s.eq_ignore_ascii_case("jar"))
                                    .unwrap_or(false)
                            {
                                candidates.push((ver_name.clone(), p));
                            }
                        }
                    }
                }
            }
        }
    }

    select_best_vanilla_jar(candidates, world_ver)
}

pub fn build_missing_texture_image() -> RgbaImage {
    let mut img = ImageBuffer::new(16, 16);
    for y in 0..16 {
        for x in 0..16 {
            let magenta = ((x / 8) + (y / 8)) % 2 == 0;
            let px = if magenta {
                Rgba([248, 0, 248, 255])
            } else {
                Rgba([16, 16, 16, 255])
            };
            img.put_pixel(x, y, px);
        }
    }
    img
}


pub fn is_grayscale_image(img: &RgbaImage) -> bool {
    let mut checked = 0usize;
    let mut colorful = 0usize;
    for p in img.pixels() {
        if p[3] < 32 {
            continue;
        }
        checked += 1;
        let max_c = p[0].max(p[1]).max(p[2]) as i32;
        let min_c = p[0].min(p[1]).min(p[2]) as i32;
        if max_c - min_c > 14 {
            colorful += 1;
        }
    }
    if checked == 0 {
        return true;
    }
    (colorful * 100 / checked) <= 8
}

pub fn apply_biome_tint(img: &RgbaImage, tint_rgb: [u8; 3]) -> RgbaImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        if p[3] == 0 {
            continue;
        }
        p[0] = ((p[0] as u16 * tint_rgb[0] as u16) / 255) as u8;
        p[1] = ((p[1] as u16 * tint_rgb[1] as u16) / 255) as u8;
        p[2] = ((p[2] as u16 * tint_rgb[2] as u16) / 255) as u8;
    }
    out
}

pub fn collect_known_biome_tints(biome_id: &str) -> ([u8; 3], [u8; 3], [u8; 3]) {
    // Returns (grass_rgb, foliage_rgb, water_rgb)
    let b = biome_id.to_lowercase();
    if b.contains("pale_garden") {
        ([119, 130, 114], [135, 141, 118], [118, 136, 157])
    } else if b.contains("swamp") || b.contains("mangrove") {
        ([106, 112, 57], [106, 112, 57], [97, 123, 100])
    } else if b.contains("desert") || b.contains("badlands") || b.contains("savanna") {
        ([191, 183, 85], [174, 164, 42], [63, 118, 228])
    } else if b.contains("dark_forest") {
        ([80, 122, 50], [89, 174, 48], [63, 118, 228])
    } else if b.contains("birch") {
        ([136, 187, 103], [128, 167, 85], [63, 118, 228])
    } else if b.contains("taiga") || b.contains("snow") || b.contains("frozen") || b.contains("grove") || b.contains("peaks") {
        ([134, 183, 131], [104, 164, 112], [57, 56, 201])
    } else if b.contains("jungle") {
        ([89, 201, 60], [48, 187, 11], [63, 118, 228])
    } else if b.contains("cherry") {
        ([182, 219, 97], [182, 219, 97], [93, 183, 239])
    } else {
        ([121, 192, 90], [89, 174, 48], [63, 118, 228])
    }
}


