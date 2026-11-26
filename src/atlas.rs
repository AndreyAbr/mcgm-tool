use crate::assets::{
    apply_biome_tint, build_missing_texture_image, collect_known_biome_tints, is_grayscale_image,
    split_namespaced, AssetIndex,
};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ColorType, ImageBuffer, ImageEncoder, Rgba, RgbaImage};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn save_png_fast(img: &RgbaImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = File::create(path).map_err(|e| e.to_string())?;
    let writer = BufWriter::with_capacity(64 * 1024, file);
    let encoder = PngEncoder::new_with_quality(writer, CompressionType::Fast, FilterType::NoFilter);
    encoder
        .write_image(img.as_raw(), img.width(), img.height(), ColorType::Rgba8.into())
        .map_err(|e| e.to_string())
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RenderBucket {
    TerrainOpaque,
    TerrainCutout,
    TerrainCutoutCull,
    TerrainTranslucent,
}

impl RenderBucket {
    pub fn material_name(self) -> &'static str {
        match self {
            Self::TerrainOpaque => "terrain_opaque",
            Self::TerrainCutout => "terrain_cutout",
            Self::TerrainCutoutCull => "terrain_cutout_cull",
            Self::TerrainTranslucent => "terrain_translucent",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UvRect {
    pub bucket: RenderBucket,
    pub u_min: f32,
    pub u_max: f32,
    pub v_top: f32, // corresponds to MC v=0 (top of texture)
    pub v_bot: f32, // corresponds to MC v=16 (bottom of texture)
}

impl UvRect {
    #[inline]
    pub fn map_uv(&self, u_norm: f32, v_norm: f32) -> [f32; 2] {
        let uc = u_norm.clamp(0.0, 1.0);
        let vc = v_norm.clamp(0.0, 1.0);
        let u = (self.u_min + uc * (self.u_max - self.u_min)).clamp(self.u_min, self.u_max);
        let v = (self.v_top - vc * (self.v_top - self.v_bot)).clamp(self.v_bot, self.v_top);
        [u, v]
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct TexRequestKey {
    pub tex_ref: String,
    pub tint_rgb: Option<[u8; 3]>,
}

pub struct TextureAtlas {
    pub image: RgbaImage,
    pub width: u32,
    pub height: u32,
    pub rects: HashMap<TexRequestKey, UvRect>,
    #[cfg(test)]
    pub pad: u32,
}

#[derive(Serialize, Deserialize)]
struct AtlasCacheMeta {
    width: u32,
    height: u32,
    pad: u32,
    rects: Vec<(TexRequestKey, UvRect)>,
}

// Bump when VTF conversion flags change to invalidate cached .vtf files
pub const VTF_CONVERSION_VERSION: u32 = 4;

/// Computes a deterministic 8-hex-char FNV-1a hash of the sorted texture requests,
/// world scale, lighting style, and atlas padding for `--reuse-cache`.
pub fn compute_atlas_cache_hash(
    requests: &HashSet<TexRequestKey>,
    scale: f64,
    lighting_style: &str,
    pad: u32,
) -> String {
    let mut keys: Vec<&TexRequestKey> = requests.iter().collect();
    keys.sort_by(|a, b| a.tex_ref.cmp(&b.tex_ref).then_with(|| a.tint_rgb.cmp(&b.tint_rgb)));

    let mut h: u64 = 0xcbf29ce484222325;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };

    mix(&VTF_CONVERSION_VERSION.to_le_bytes());
    mix(&scale.to_bits().to_le_bytes());
    mix(lighting_style.as_bytes());
    mix(&[0xff]);
    mix(&pad.to_le_bytes());
    for k in keys {
        mix(k.tex_ref.as_bytes());
        match k.tint_rgb {
            Some(rgb) => {
                mix(&[1]);
                mix(&rgb);
            }
            None => mix(&[0]),
        }
    }
    format!("{:08x}", (h ^ (h >> 32)) as u32)
}

pub fn save_atlas_cache_meta(atlas: &TextureAtlas, path: &Path, pad: u32) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let meta = AtlasCacheMeta {
        width: atlas.width,
        height: atlas.height,
        pad,
        rects: atlas.rects.iter().map(|(k, v)| (k.clone(), *v)).collect(),
    };
    let json = serde_json::to_string(&meta).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn load_atlas_cache_meta(path: &Path) -> Result<TextureAtlas, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let meta: AtlasCacheMeta = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if meta.width == 0 || meta.height == 0 || meta.rects.is_empty() {
        return Err("Invalid cached atlas metadata".to_string());
    }
    let mut rects = HashMap::with_capacity(meta.rects.len());
    for (k, v) in meta.rects {
        rects.insert(k, v);
    }
    Ok(TextureAtlas {
        image: RgbaImage::new(1, 1),
        width: meta.width,
        height: meta.height,
        rects,
        #[cfg(test)]
        pad: meta.pad,
    })
}

impl TextureAtlas {
    pub fn get_or_default(&self, key: &TexRequestKey) -> UvRect {
        if let Some(&r) = self.rects.get(key) {
            return r;
        }
        let untinted = TexRequestKey {
            tex_ref: key.tex_ref.clone(),
            tint_rgb: None,
        };
        if let Some(&r) = self.rects.get(&untinted) {
            return r;
        }
        *self
            .rects
            .values()
            .next()
            .expect("Atlas always contains at least missing texture")
    }
}

pub struct SignAtlas {
    pub image: RgbaImage,
    pub rects: HashMap<String, UvRect>, // key: "x,y,z,front", "x,y,z,back", or "base:<sign|hanging>:<wood>"
}

pub fn classify_texture_bucket(tex_ref: &str, img: &RgbaImage) -> RenderBucket {
    let lower = tex_ref.to_lowercase();
    let is_solid_bamboo = lower.contains("bamboo_block")
        || lower.contains("bamboo_planks")
        || lower.contains("bamboo_mosaic")
        || lower.contains("stripped_bamboo_block");
    let is_coral_block = lower.contains("coral_block");

    // packed_ice and blue_ice are completely opaque in Minecraft
    let is_opaque_ice = lower.contains("packed_ice") || lower.contains("blue_ice");
    let translucent_tokens = [
        "water_still",
        "water_flow",
        "water_overlay",
        "stained_glass",
        "slime_block",
        "honey_block",
        "nether_portal",
        "frosted_ice",
        "tinted_glass",
        "bubble",
    ];
    let is_plain_ice = !is_opaque_ice
        && (lower.ends_with("/ice") || lower.ends_with("block/ice") || lower == "ice");
    if is_plain_ice
        || (!is_coral_block && translucent_tokens.iter().any(|t| lower.contains(t)))
    {
        // stained_glass vs glass check: normal glass has binary alpha (cutout)
        if !lower.ends_with("block/glass")
            && !lower.ends_with("block/glass_pane_top")
            && !lower.ends_with("/glass")
        {
            return RenderBucket::TerrainTranslucent;
        }
    }

    // 3D volumetric cutout textures: backface-culled (no $nocull 1)
    let cutout_cull_tokens = [
        "leaves",
        "glass",
        "bars",
        "trapdoor",
        "door",
        "grass_block_side_overlay",
        "hopper",
        "cauldron",
        "beacon",
        "scaffolding",
        "lantern",
        "chain",
        "bell",
        "brewing_stand",
        "flower_pot",
        "azalea",
        "dripleaf",
        "bamboo",
        "chorus_flower",
        "chorus_plant",
        "end_rod",
        "conduit",
        "turtle_egg",
        "sniffer_egg",
        "frogspawn",
    ];
    if cutout_cull_tokens.iter().any(|&t| {
        if t == "bamboo" && is_solid_bamboo {
            return false;
        }
        lower.contains(t)
    }) {
        return RenderBucket::TerrainCutoutCull;
    }

    // Crossed/thin 2D cutout textures ($nocull 1)
    let cutout_tokens = [
        "sapling",
        "propagule",
        "short_grass",
        "tall_grass",
        "fern",
        "poppy",
        "dandelion",
        "tulip",
        "orchid",
        "allium",
        "cornflower",
        "azure_bluet",
        "oxeye_daisy",
        "lily_of_the_valley",
        "wither_rose",
        "sunflower",
        "lilac",
        "rose_bush",
        "peony",
        "torchflower",
        "pitcher_plant",
        "pitcher_crop",
        "lily_pad",
        "vine",
        "ladder",
        "rail",
        "torch",
        "cobweb",
        "sugar_cane",
        "wheat",
        "carrots",
        "potatoes",
        "beetroots",
        "nether_wart",
        "cocoa",
        "_stem",
        "repeater",
        "comparator",
        "redstone_dust",
        "tripwire",
        "kelp",
        "seagrass",
        "coral",
        "campfire",
        "fire",
        "spore_blossom",
        "glow_lichen",
        "sculk_vein",
        "lever",
        "dead_bush",
        "brown_mushroom",
        "red_mushroom",
        "sweet_berry",
        "pink_petals",
        "hanging_roots",
        "mangrove_roots",
        "pointed_dripstone",
        "amethyst_cluster",
        "amethyst_bud",
        "sea_pickle",
        "wildflowers",
        "bush",
        "firefly_bush",
        "cactus_flower",
        "leaf_litter",
        "cogwheel",
        "shaft",
        "pipe",
        "belt",
        "track",
        "fan",
        "sail",
    ];
    if cutout_tokens.iter().any(|&t| {
        if t == "coral" && is_coral_block {
            return false;
        }
        lower.contains(t)
    }) {
        return RenderBucket::TerrainCutout;
    }

    // Inspect pixel alpha distribution directly.
    // Avoid classifying anti-aliased cutout textures (which have alpha==0 holes) as translucent.
    let total = (img.width() * img.height()).max(1);
    let mut semi = 0u32;
    let mut transparent = 0u32;
    for p in img.pixels() {
        if p[3] == 0 {
            transparent += 1;
        } else if p[3] < 245 {
            semi += 1;
        }
    }
    if (transparent == 0 && semi * 100 / total >= 4)
        || (semi * 100 / total >= 15 && semi > transparent)
    {
        RenderBucket::TerrainTranslucent
    } else if transparent > 0 {
        RenderBucket::TerrainCutout
    } else {
        RenderBucket::TerrainOpaque
    }
}

pub fn effective_biome_tint(
    block_name: &str,
    tex_ref: &str,
    tint_index: Option<i32>,
    biome_id: &str,
) -> Option<[u8; 3]> {
    let b_lower = block_name.to_lowercase();
    let t_lower = tex_ref.to_lowercase();

    // Never tint non-foliage tintindex blocks (redstone dust, melon/pumpkin stems, cherry/azalea leaves, plain grass_block_side)
    if b_lower.contains("redstone")
        || t_lower.contains("redstone")
        || b_lower.contains("_stem")
        || t_lower.contains("_stem")
        || t_lower.contains("cherry_leaves")
        || t_lower.contains("azalea_leaves")
        || b_lower.contains("cherry_leaves")
        || b_lower.contains("azalea_leaves")
        || t_lower.ends_with("grass_block_side")
    {
        return None;
    }

    if b_lower.contains("lily_pad") || t_lower.contains("lily_pad") {
        return Some([72, 181, 24]);
    }

    let is_grass_tex = t_lower.contains("grass_block_top")
        || t_lower.contains("grass_block_side_overlay")
        || t_lower.ends_with("short_grass")
        || t_lower.contains("tall_grass")
        || t_lower.ends_with("/grass")
        || t_lower == "grass"
        || t_lower.contains("fern")
        || t_lower.contains("sugar_cane");
    let is_foliage_tex = t_lower.contains("_leaves")
        || t_lower.ends_with("/vine")
        || t_lower.ends_with(":vine")
        || t_lower == "vine";
    let is_water_tex = t_lower.contains("water_still")
        || t_lower.contains("water_flow")
        || t_lower.contains("water_overlay");

    let is_foliage_or_grass_block = b_lower.contains("grass_block")
        || b_lower.ends_with(":grass")
        || b_lower == "grass"
        || b_lower.contains("short_grass")
        || b_lower.contains("tall_grass")
        || b_lower.contains("fern")
        || b_lower.contains("leaves")
        || b_lower.contains("vine")
        || b_lower.contains("sugar_cane")
        || b_lower.contains("water");

    let needs_tint = is_grass_tex
        || is_foliage_tex
        || is_water_tex
        || (tint_index.is_some() && is_foliage_or_grass_block);

    if !needs_tint {
        return None;
    }

    let (grass_rgb, foliage_rgb, water_rgb) = collect_known_biome_tints(biome_id);
    if t_lower.contains("water") || b_lower.contains("water") {
        return Some(quantize_rgb(water_rgb));
    }
    if t_lower.contains("spruce_leaves") || b_lower.contains("spruce_leaves") {
        return Some([96, 152, 96]);
    }
    if t_lower.contains("birch_leaves") || b_lower.contains("birch_leaves") {
        return Some([128, 168, 84]);
    }
    if t_lower.contains("leaves") || t_lower.contains("vine") || b_lower.contains("leaves") {
        return Some(quantize_rgb(foliage_rgb));
    }
    Some(quantize_rgb(grass_rgb))
}

fn quantize_rgb(c: [u8; 3]) -> [u8; 3] {
    [(c[0] & 0xFC), (c[1] & 0xFC), (c[2] & 0xFC)]
}

pub fn build_texture_atlas(
    requests: &HashSet<TexRequestKey>,
    assets: &AssetIndex,
    pad: u32,
) -> TextureAtlas {
    let mut keys: Vec<TexRequestKey> = requests.iter().cloned().collect();
    keys.sort_by(|a, b| a.tex_ref.cmp(&b.tex_ref).then_with(|| a.tint_rgb.cmp(&b.tint_rgb)));

    let missing_key = TexRequestKey {
        tex_ref: "minecraft:block/__missing__".to_string(),
        tint_rgb: None,
    };
    if keys.is_empty() {
        keys.push(missing_key.clone());
    }

    struct PreparedTile {
        key: TexRequestKey,
        img: RgbaImage,
        bucket: RenderBucket,
    }

    // Decode & resize each unique tex_ref ONCE before applying biome tint variants
    let unique_refs: Vec<String> = {
        let mut set: HashSet<&str> = HashSet::with_capacity(keys.len());
        let mut out = Vec::new();
        for k in &keys {
            if set.insert(k.tex_ref.as_str()) {
                out.push(k.tex_ref.clone());
            }
        }
        out
    };

    let tile_upscale: u32 = if pad >= 8 {
        if keys.len() <= 500 {
            4
        } else if keys.len() <= 1600 {
            2
        } else {
            1
        }
    } else {
        1
    };

    let base_map: HashMap<String, (RgbaImage, RenderBucket, bool)> = unique_refs
        .into_par_iter()
        .map(|tex_ref| {
            let mut raw_img = assets
                .load_texture_image(&tex_ref)
                .unwrap_or_else(build_missing_texture_image);

            let (w, h) = raw_img.dimensions();
            if w == 0 || h == 0 {
                raw_img = build_missing_texture_image();
            } else if w > 128 || h > 128 {
                let max_dim = w.max(h) as f32;
                let scale = 128.0 / max_dim;
                let nw = ((w as f32 * scale).round() as u32).max(1);
                let nh = ((h as f32 * scale).round() as u32).max(1);
                raw_img =
                    image::imageops::resize(&raw_img, nw, nh, image::imageops::FilterType::Nearest);
            } else if tile_upscale > 1 && w <= 32 && h <= 32 {
                let nw = (w * tile_upscale).min(128);
                let nh = (h * tile_upscale).min(128);
                raw_img =
                    image::imageops::resize(&raw_img, nw, nh, image::imageops::FilterType::Nearest);
            }

            let bucket = classify_texture_bucket(&tex_ref, &raw_img);
            let is_gray = is_grayscale_image(&raw_img);
            (tex_ref, (raw_img, bucket, is_gray))
        })
        .collect();

    let tiles: Vec<PreparedTile> = keys
        .into_par_iter()
        .map(|key| {
            let (base_img, bucket, is_gray) = &base_map[&key.tex_ref];
            let mut raw_img = if let Some(tint) = key.tint_rgb {
                let is_water = key.tex_ref.to_ascii_lowercase().contains("water");
                if is_water || *is_gray {
                    apply_biome_tint(base_img, tint)
                } else {
                    base_img.clone()
                }
            } else {
                base_img.clone()
            };
            if *bucket == RenderBucket::TerrainOpaque {
                for px in raw_img.as_mut().as_chunks_mut::<4>().0 {
                    px[3] = 255;
                }
            } else {
                // Bleed adjacent visible RGB into alpha==0 pixels to prevent dark halos
                // and alpha-test flickering in mipmaps and DXT5 4x4 compression blocks.
                bleed_transparent_rgb(&mut raw_img);
            }
            PreparedTile {
                key,
                img: raw_img,
                bucket: *bucket,
            }
        })
        .collect();

    // Estimate atlas dimension via shelf packing
    let tile_sizes: Vec<(u32, u32)> = tiles
        .iter()
        .map(|t| (t.img.width() + pad * 2, t.img.height() + pad * 2))
        .collect();
    let total_area: u64 = tile_sizes
        .iter()
        .map(|&(tw, th)| (tw as u64) * (th as u64))
        .sum();
    let mut dim = 128u32;
    while (dim as u64) * (dim as u64) < total_area * 5 / 4 {
        dim *= 2;
        if dim >= 4096 {
            break;
        }
    }

    // Try packing, doubling dim up to 4096 if shelf overflows, then 4096x8192 / 8192x8192,
    // and finally a best-effort placement without panicking.
    let (atlas_w, atlas_h, placements) = loop {
        if let Some(p) = try_shelf_pack(&tile_sizes, dim, dim) {
            break (dim, dim, p.into_iter().map(Some).collect::<Vec<_>>());
        }
        if dim >= 4096 {
            if let Some(p) = try_shelf_pack(&tile_sizes, 4096, 8192) {
                break (4096, 8192, p.into_iter().map(Some).collect::<Vec<_>>());
            }
            if let Some(p) = try_shelf_pack(&tile_sizes, 8192, 8192) {
                break (8192, 8192, p.into_iter().map(Some).collect::<Vec<_>>());
            }
            let p = shelf_pack_best_effort(&tile_sizes, 8192, 8192);
            break (8192, 8192, p);
        }
        dim *= 2;
    };

    let mut atlas_img: RgbaImage = ImageBuffer::new(atlas_w, atlas_h);
    let mut rects = HashMap::with_capacity(tiles.len());

    for (tile, placement) in tiles.into_iter().zip(placements) {
        let Some((ox, oy)) = placement else {
            continue;
        };
        let tw = tile.img.width();
        let th = tile.img.height();
        let px = ox + pad;
        let py = oy + pad;

        if px + tw <= atlas_w && py + th <= atlas_h {
            blit_with_extruded_padding(&tile.img, &mut atlas_img, px, py, pad);
        }

        let aw = atlas_w as f32;
        let ah = atlas_h as f32;
        let half_u = 0.5 / aw;
        let half_v = 0.5 / ah;

        let u_min = ((px as f32) / aw + half_u).clamp(0.0, 1.0);
        let u_max = (((px + tw) as f32) / aw - half_u).clamp(u_min, 1.0);
        let v_top = (1.0 - ((py as f32) / ah + half_v)).clamp(0.0, 1.0);
        let v_bot = (1.0 - (((py + th) as f32) / ah - half_v)).clamp(0.0, v_top);

        rects.insert(
            tile.key,
            UvRect {
                bucket: tile.bucket,
                u_min,
                u_max,
                v_top,
                v_bot,
            },
        );
    }

    TextureAtlas {
        image: atlas_img,
        width: atlas_w,
        height: atlas_h,
        rects,
        #[cfg(test)]
        pad,
    }
}

fn try_shelf_pack(sizes: &[(u32, u32)], max_w: u32, max_h: u32) -> Option<Vec<(u32, u32)>> {
    let mut indexed: Vec<(usize, u32, u32)> = sizes
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| (i, w, h))
        .collect();
    indexed.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));

    let mut out = vec![(0u32, 0u32); sizes.len()];
    let mut cur_x = 0u32;
    let mut cur_y = 0u32;
    let mut row_h = 0u32;

    for (idx, w, h) in indexed {
        if w > max_w || h > max_h {
            return None;
        }
        if cur_x + w > max_w {
            cur_x = 0;
            cur_y += row_h;
            row_h = 0;
        }
        if cur_y + h > max_h {
            return None;
        }
        out[idx] = (cur_x, cur_y);
        cur_x += w;
        row_h = row_h.max(h);
    }
    Some(out)
}

pub(crate) fn shelf_pack_best_effort(sizes: &[(u32, u32)], max_w: u32, max_h: u32) -> Vec<Option<(u32, u32)>> {
    let mut indexed: Vec<(usize, u32, u32)> = sizes
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| (i, w, h))
        .collect();
    indexed.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));

    let mut out = vec![None; sizes.len()];
    let mut cur_x = 0u32;
    let mut cur_y = 0u32;
    let mut row_h = 0u32;

    for (idx, w, h) in indexed {
        if w > max_w || h > max_h {
            out[idx] = None;
            continue;
        }
        if cur_x + w > max_w {
            cur_x = 0;
            cur_y += row_h;
            row_h = 0;
        }
        if cur_y + h > max_h {
            out[idx] = None;
            continue;
        }
        out[idx] = Some((cur_x, cur_y));
        cur_x += w;
        row_h = row_h.max(h);
    }
    out
}

/// Dilates RGB colors from opaque/visible pixels into adjacent `alpha == 0` pixels
/// while keeping `alpha == 0` unchanged, preventing dark halos and mipmap/DXT5 edge flickering.
#[allow(clippy::manual_checked_ops)]
fn bleed_transparent_rgb(img: &mut RgbaImage) {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return;
    }
    let wi = w as i32;
    let hi = h as i32;
    let len = (w * h) as usize;
    let raw = img.as_mut();
    let mut valid = vec![false; len];
    let mut has_transparent = false;
    let mut has_opaque = false;

    for idx in 0..len {
        if raw[idx * 4 + 3] > 0 {
            valid[idx] = true;
            has_opaque = true;
        } else {
            has_transparent = true;
        }
    }
    if !has_transparent || !has_opaque {
        return;
    }

    // Seed initial frontier: transparent pixels adjacent to at least one opaque pixel
    let mut queued = valid.clone();
    let mut frontier: Vec<(i32, i32)> = Vec::new();
    for y in 0..hi {
        for x in 0..wi {
            let idx = (y * wi + x) as usize;
            if valid[idx] {
                continue;
            }
            let mut adj = false;
            for ny in (y - 1).max(0)..=(y + 1).min(hi - 1) {
                for nx in (x - 1).max(0)..=(x + 1).min(wi - 1) {
                    if valid[(ny * wi + nx) as usize] {
                        adj = true;
                        break;
                    }
                }
                if adj {
                    break;
                }
            }
            if adj {
                queued[idx] = true;
                frontier.push((x, y));
            }
        }
    }

    let mut next_frontier: Vec<(i32, i32)> = Vec::new();
    let mut updates: Vec<(usize, [u8; 3])> = Vec::new();
    for _pass in 0..8 {
        if frontier.is_empty() {
            break;
        }
        updates.clear();
        next_frontier.clear();
        for &(x, y) in &frontier {
            let idx = (y * wi + x) as usize;
            let mut sr = 0u32;
            let mut sg = 0u32;
            let mut sb = 0u32;
            let mut cnt = 0u32;
            for ny in (y - 1).max(0)..=(y + 1).min(hi - 1) {
                for nx in (x - 1).max(0)..=(x + 1).min(wi - 1) {
                    if nx == x && ny == y {
                        continue;
                    }
                    let nidx = (ny * wi + nx) as usize;
                    if valid[nidx] {
                        let off = nidx * 4;
                        sr += raw[off] as u32;
                        sg += raw[off + 1] as u32;
                        sb += raw[off + 2] as u32;
                        cnt += 1;
                    } else if !queued[nidx] {
                        queued[nidx] = true;
                        next_frontier.push((nx, ny));
                    }
                }
            }
            if cnt > 0 {
                updates.push((idx, [(sr / cnt) as u8, (sg / cnt) as u8, (sb / cnt) as u8]));
            }
        }
        for &(idx, rgb) in &updates {
            let off = idx * 4;
            raw[off] = rgb[0];
            raw[off + 1] = rgb[1];
            raw[off + 2] = rgb[2];
            valid[idx] = true;
        }
        std::mem::swap(&mut frontier, &mut next_frontier);
    }
}

fn blit_with_extruded_padding(src: &RgbaImage, dst: &mut RgbaImage, px: u32, py: u32, pad: u32) {
    let sw = src.width();
    let sh = src.height();
    let dw = dst.width();
    let dh = dst.height();
    if sw == 0 || sh == 0 || px + sw > dw || py + sh > dh {
        return;
    }

    let src_raw = src.as_raw();
    let dst_raw = dst.as_mut();
    let src_stride = (sw * 4) as usize;
    let dst_stride = (dw * 4) as usize;

    // 1. Copy inner tile rows in bulk
    for sy in 0..sh {
        let s_off = (sy as usize) * src_stride;
        let d_off = ((py + sy) as usize) * dst_stride + (px as usize) * 4;
        dst_raw[d_off..d_off + src_stride].copy_from_slice(&src_raw[s_off..s_off + src_stride]);
    }

    if pad == 0 {
        return;
    }

    let x_start = px.saturating_sub(pad);
    let x_end = (px + sw + pad).min(dw);
    let y_start = py.saturating_sub(pad);
    let y_end = (py + sh + pad).min(dh);

    // 2. Extrude left/right padding on tile rows
    for sy in 0..sh {
        let dy = (py + sy) as usize;
        let row_off = dy * dst_stride;
        let left_px = &src_raw[(sy as usize) * src_stride..(sy as usize) * src_stride + 4];
        for ox in x_start..px {
            let o = row_off + (ox as usize) * 4;
            dst_raw[o..o + 4].copy_from_slice(left_px);
        }
        let right_src_off = (sy as usize) * src_stride + ((sw - 1) as usize) * 4;
        let right_px = &src_raw[right_src_off..right_src_off + 4];
        for ox in (px + sw)..x_end {
            let o = row_off + (ox as usize) * 4;
            dst_raw[o..o + 4].copy_from_slice(right_px);
        }
    }

    // 3. Extrude top/bottom padding by copying the already-padded top/bottom rows
    let span_bytes = ((x_end - x_start) as usize) * 4;
    let top_row_off = (py as usize) * dst_stride + (x_start as usize) * 4;
    let bot_row_off = ((py + sh - 1) as usize) * dst_stride + (x_start as usize) * 4;
    for oy in y_start..py {
        let dst_off = (oy as usize) * dst_stride + (x_start as usize) * 4;
        dst_raw.copy_within(top_row_off..top_row_off + span_bytes, dst_off);
    }
    for oy in (py + sh)..y_end {
        let dst_off = (oy as usize) * dst_stride + (x_start as usize) * 4;
        dst_raw.copy_within(bot_row_off..bot_row_off + span_bytes, dst_off);
    }
}

pub fn export_door_textures(
    door_block_names: &HashSet<String>,
    assets: &AssetIndex,
    out_dir: &Path,
) -> HashMap<String, PathBuf> {
    let _ = std::fs::create_dir_all(out_dir);
    let mut result = HashMap::new();
    for block_name in door_block_names {
        let (ns, short) = split_namespaced(block_name);
        let short_lower = short.to_ascii_lowercase();
        let mat_stem = format!("door_{}", short.replace('/', "_"));
        let is_door = short_lower.ends_with("_door") && !short_lower.ends_with("_trapdoor");

        if is_door {
            let (top_opt, bot_opt) = assets.find_door_textures(block_name);
            let top_raw = top_opt.unwrap_or_else(|| solid_wood_fallback(&short));
            let bot_raw = bot_opt.unwrap_or_else(|| top_raw.clone());

            let top_16 = if top_raw.dimensions() != (16, 16) {
                image::imageops::resize(&top_raw, 16, 16, image::imageops::FilterType::Nearest)
            } else {
                top_raw
            };
            let bot_16 = if bot_raw.dimensions() != (16, 16) {
                image::imageops::resize(&bot_raw, 16, 16, image::imageops::FilterType::Nearest)
            } else {
                bot_raw
            };

            let mut combined: RgbaImage = ImageBuffer::new(16, 32);
            for y in 0..16 {
                for x in 0..16 {
                    combined.put_pixel(x, y, *top_16.get_pixel(x, y));
                    combined.put_pixel(x, y + 16, *bot_16.get_pixel(x, y));
                }
            }
            bleed_transparent_rgb(&mut combined);
            let png_path = out_dir.join(format!("{}.png", mat_stem));
            if save_png_fast(&combined, &png_path).is_ok() {
                result.insert(mat_stem, png_path);
            }
        } else {
            let raw = resolve_non_door_tile(assets, &ns, &short, &short_lower)
                .unwrap_or_else(|| solid_wood_fallback(&short));
            let mut tile_16 = if raw.dimensions() != (16, 16) {
                image::imageops::resize(&raw, 16, 16, image::imageops::FilterType::Nearest)
            } else {
                raw
            };
            bleed_transparent_rgb(&mut tile_16);
            let png_path = out_dir.join(format!("{}.png", mat_stem));
            if save_png_fast(&tile_16, &png_path).is_ok() {
                result.insert(mat_stem, png_path);
            }
        }
    }
    result
}

fn resolve_non_door_tile(
    assets: &AssetIndex,
    ns: &str,
    short: &str,
    short_lower: &str,
) -> Option<RgbaImage> {
    let mut candidates: Vec<String> = Vec::new();
    if short_lower == "stone_button" {
        candidates.push(format!("{}:block/stone", ns));
        if ns != "minecraft" {
            candidates.push("minecraft:block/stone".to_string());
        }
    } else if short_lower == "polished_blackstone_button" {
        candidates.push(format!("{}:block/polished_blackstone", ns));
        if ns != "minecraft" {
            candidates.push("minecraft:block/polished_blackstone".to_string());
        }
    } else if short_lower == "lever" {
        candidates.push(format!("{}:block/cobblestone", ns));
        if ns != "minecraft" {
            candidates.push("minecraft:block/cobblestone".to_string());
        }
    } else if let Some(wood) = short_lower
        .strip_suffix("_fence_gate")
        .or_else(|| short_lower.strip_suffix("_button"))
    {
        candidates.push(format!("{}:block/{}_planks", ns, wood));
        if ns != "minecraft" {
            candidates.push(format!("minecraft:block/{}_planks", wood));
        }
        if wood == "bamboo" {
            candidates.push(format!("{}:block/bamboo_mosaic", ns));
            if ns != "minecraft" {
                candidates.push("minecraft:block/bamboo_mosaic".to_string());
            }
        }
        candidates.push(format!("{}:block/{}", ns, short));
    } else {
        candidates.push(format!("{}:block/{}", ns, short));
        candidates.push(format!("{}:block/{}_planks", ns, short));
        if ns != "minecraft" {
            candidates.push(format!("minecraft:block/{}", short));
            candidates.push(format!("minecraft:block/{}_planks", short));
        }
    }

    candidates
        .iter()
        .find_map(|c| assets.load_texture_image(c))
}

fn solid_wood_fallback(short: &str) -> RgbaImage {
    let s = short.to_lowercase();
    let color = if s.contains("iron") {
        Rgba([210, 210, 215, 255])
    } else if s.contains("oxidized_copper") {
        Rgba([82, 162, 132, 255])
    } else if s.contains("weathered_copper") {
        Rgba([108, 152, 110, 255])
    } else if s.contains("exposed_copper") {
        Rgba([160, 125, 102, 255])
    } else if s.contains("copper") {
        Rgba([192, 107, 79, 255])
    } else if s.contains("dark_oak") {
        Rgba([72, 48, 28, 255])
    } else if s.contains("spruce") {
        Rgba([108, 80, 48, 255])
    } else if s.contains("birch") {
        Rgba([210, 195, 145, 255])
    } else if s.contains("jungle") {
        Rgba([160, 115, 80, 255])
    } else if s.contains("acacia") {
        Rgba([168, 90, 50, 255])
    } else if s.contains("mangrove") {
        Rgba([118, 54, 48, 255])
    } else if s.contains("cherry") {
        Rgba([226, 178, 172, 255])
    } else if s.contains("bamboo") {
        Rgba([196, 175, 76, 255])
    } else if s.contains("crimson") {
        Rgba([101, 48, 70, 255])
    } else if s.contains("warped") {
        Rgba([43, 104, 99, 255])
    } else if s.contains("pale_oak") {
        Rgba([228, 218, 214, 255])
    } else {
        Rgba([162, 130, 78, 255])
    };
    ImageBuffer::from_pixel(16, 16, color)
}

#[derive(Deserialize)]
struct FontFileJson {
    #[serde(default)]
    providers: Vec<FontProviderJson>,
}

#[derive(Deserialize)]
struct FontProviderJson {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    chars: Option<Vec<String>>,
    #[serde(default)]
    height: Option<u32>,
}

pub(crate) struct BitmapGlyph {
    pub(crate) sheet: Arc<RgbaImage>,
    pub(crate) gx: u32,
    pub(crate) gy: u32,
    pub(crate) gw: u32,
    pub(crate) gh: u32,
    pub(crate) content_width: u32,
}

pub struct FontRenderer {
    pub(crate) glyphs: HashMap<char, BitmapGlyph>,
}

impl FontRenderer {
    pub fn from_assets(assets: &AssetIndex) -> Self {
        let mut glyphs = HashMap::new();

        // 1. Parse Minecraft font JSON bitmap providers (default.json & include/default.json)
        let mut sheets_cache: HashMap<String, Arc<RgbaImage>> = HashMap::new();
        for json_path in [
            "assets/minecraft/font/include/default.json",
            "assets/minecraft/font/default.json",
        ] {
            if let Some(bytes) = assets.get_bytes(json_path) {
                if let Ok(ff) = serde_json::from_slice::<FontFileJson>(&bytes) {
                    for prov in ff.providers {
                        if prov.kind != "bitmap" {
                            continue;
                        }
                        let (Some(file_ref), Some(rows)) = (prov.file, prov.chars) else {
                            continue;
                        };
                        let (ns, rel) = split_namespaced(&file_ref);
                        let rel_clean = rel.trim_start_matches('/');
                        let tex_key = if rel_clean.starts_with("textures/") {
                            format!("assets/{}/{}", ns, rel_clean)
                        } else {
                            format!("assets/{}/textures/{}", ns, rel_clean)
                        };
                        let sheet = if let Some(s) = sheets_cache.get(&tex_key) {
                            Arc::clone(s)
                        } else if let Some(b) = assets.get_bytes(&tex_key) {
                            if let Ok(im) = image::load_from_memory(&b) {
                                let rgba = Arc::new(im.to_rgba8());
                                sheets_cache.insert(tex_key.clone(), Arc::clone(&rgba));
                                rgba
                            } else {
                                continue;
                            }
                        } else {
                            continue;
                        };

                        let num_rows = rows.len().max(1) as u32;
                        let gh = sheet.height() / num_rows;
                        if sheet.width() == 0 || sheet.height() == 0 || gh == 0 {
                            continue;
                        }
                        let _ = prov.height;
                        for (row_idx, row_str) in rows.iter().enumerate() {
                            let chars: Vec<char> = row_str.chars().collect();
                            let num_cols = chars.len().max(1) as u32;
                            let gw = sheet.width() / num_cols;
                            if gw == 0 {
                                continue;
                            }
                            for (col_idx, ch) in chars.into_iter().enumerate() {
                                if ch == '\0' || ch == ' ' {
                                    continue;
                                }
                                let gx = (col_idx as u32) * gw;
                                let gy = (row_idx as u32) * gh;
                                if gx.checked_add(gw).is_none_or(|e| e > sheet.width())
                                    || gy.checked_add(gh).is_none_or(|e| e > sheet.height())
                                {
                                    continue;
                                }
                                let cw = measure_glyph_width(&sheet, gx, gy, gw, gh);
                                glyphs.entry(ch).or_insert_with(|| BitmapGlyph {
                                    sheet: Arc::clone(&sheet),
                                    gx,
                                    gy,
                                    gw,
                                    gh,
                                    content_width: cw,
                                });
                            }
                        }
                    }
                }
            }
        }

        // 2. Also inspect ascii.png, nonlatin_european.png, and accented.png directly if JSON providers were absent
        let (ascii_opt, nonlatin_opt, accented_opt) = assets.font_images();
        if let Some(ascii_sheet) = ascii_opt {
            let sheet = Arc::new(ascii_sheet.clone());
            let gw = sheet.width() / 16;
            let gh = sheet.height() / 16;
            if gw > 0 && gh > 0 {
                for code in 33u8..=126u8 {
                    let ch = code as char;
                    if glyphs.contains_key(&ch) {
                        continue;
                    }
                    let gx = ((code % 16) as u32) * gw;
                    let gy = ((code / 16) as u32) * gh;
                    if gx.checked_add(gw).is_none_or(|e| e > sheet.width())
                        || gy.checked_add(gh).is_none_or(|e| e > sheet.height())
                    {
                        continue;
                    }
                    let cw = measure_glyph_width(&sheet, gx, gy, gw, gh);
                    glyphs.insert(
                        ch,
                        BitmapGlyph {
                            sheet: Arc::clone(&sheet),
                            gx,
                            gy,
                            gw,
                            gh,
                            content_width: cw,
                        },
                    );
                }
            }
        }

        if let Some(nonlatin_sheet) = nonlatin_opt {
            if !glyphs.contains_key(&'А') {
                let sheet = Arc::new(nonlatin_sheet.clone());
                let rows: [&str; 6] = [
                    "¡¿ÀÁÂÃÄÅÆÇÈÉÊËÌÍ",
                    "ÎÏÐÑÒÓÔÕÖ×ØÙÚÛÜÝ",
                    "Þßàáâãäåæçèéêëìí",
                    "îïðñòóôõö÷øùúûüý",
                    "АБВГДЕЖЗИЙКЛМНОП",
                    "РСТУФХЦЧШЩЪЫЬЭЮЯ",
                ];
                let num_rows = rows.len() as u32;
                let gw = sheet.width() / 16;
                let gh = sheet.height() / num_rows;
                if gw > 0 && gh > 0 {
                    for (row_idx, row_str) in rows.iter().enumerate() {
                        for (col_idx, ch) in row_str.chars().enumerate() {
                            if glyphs.contains_key(&ch) {
                                continue;
                            }
                            let gx = (col_idx as u32) * gw;
                            let gy = (row_idx as u32) * gh;
                            if gx.checked_add(gw).is_none_or(|e| e > sheet.width())
                                || gy.checked_add(gh).is_none_or(|e| e > sheet.height())
                            {
                                continue;
                            }
                            let cw = measure_glyph_width(&sheet, gx, gy, gw, gh);
                            glyphs.insert(
                                ch,
                                BitmapGlyph {
                                    sheet: Arc::clone(&sheet),
                                    gx,
                                    gy,
                                    gw,
                                    gh,
                                    content_width: cw,
                                },
                            );
                        }
                    }
                }
            }
        }

        if let Some(accented_sheet) = accented_opt {
            if !glyphs.contains_key(&'À') {
                let sheet = Arc::new(accented_sheet.clone());
                let rows: [&str; 4] = [
                    "ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏ",
                    "ÐÑÒÓÔÕÖ×ØÙÚÛÜÝÞß",
                    "àáâãäåæçèéêëìíîï",
                    "ðñòóôõö÷øùúûüýþÿ",
                ];
                let num_rows = rows.len() as u32;
                let gw = sheet.width() / 16;
                let gh = sheet.height() / num_rows;
                if gw > 0 && gh > 0 {
                    for (row_idx, row_str) in rows.iter().enumerate() {
                        for (col_idx, ch) in row_str.chars().enumerate() {
                            if glyphs.contains_key(&ch) {
                                continue;
                            }
                            let gx = (col_idx as u32) * gw;
                            let gy = (row_idx as u32) * gh;
                            if gx.checked_add(gw).is_none_or(|e| e > sheet.width())
                                || gy.checked_add(gh).is_none_or(|e| e > sheet.height())
                            {
                                continue;
                            }
                            let cw = measure_glyph_width(&sheet, gx, gy, gw, gh);
                            glyphs.insert(
                                ch,
                                BitmapGlyph {
                                    sheet: Arc::clone(&sheet),
                                    gx,
                                    gy,
                                    gw,
                                    gh,
                                    content_width: cw,
                                },
                            );
                        }
                    }
                }
            }
        }

        Self { glyphs }
    }

    #[cfg(test)]
    pub fn render_sign_panel(&self, lines: &[String; 4], color_name: &str, wood_tint: [u8; 3]) -> RgbaImage {
        self.render_sign_panel_with_base(lines, color_name, wood_tint, None, false)
    }

    #[cfg(test)]
    pub fn render_sign_panel_with_base(
        &self,
        lines: &[String; 4],
        color_name: &str,
        wood_tint: [u8; 3],
        entity_tex: Option<&RgbaImage>,
        is_hanging: bool,
    ) -> RgbaImage {
        let w = 256u32;
        let h = 128u32;
        let mut img: RgbaImage = if let Some(tex) = entity_tex {
            crop_sign_entity_panel(tex, w, h, is_hanging)
        } else {
            procedural_sign_wood_panel(w, h, wood_tint)
        };

        self.render_sign_text_onto(&mut img, lines, color_name);
        img
    }

    pub(crate) fn render_sign_text_onto(
        &self,
        img: &mut RgbaImage,
        lines: &[String; 4],
        color_name: &str,
    ) {
        let (w, h) = img.dimensions();
        let ink = sign_ink_rgb(color_name);
        let line_step = 28u32;
        let top_pad = 10u32;

        for (line_idx, text) in lines.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let base_px_w: u32 = text
                .chars()
                .map(|c| self.char_advance(c))
                .sum::<u32>()
                .max(1);
            let max_w = w - 12;
            let scale = if base_px_w * 3 <= max_w {
                3u32
            } else if base_px_w * 2 <= max_w {
                2u32
            } else {
                1u32
            };

            let y_offset = (24u32.saturating_sub(8 * scale)) / 2;
            let start_y = top_pad + (line_idx as u32) * line_step + y_offset;

            if base_px_w * scale <= w - 8 {
                let total_px_w = base_px_w * scale;
                let start_x = (w - total_px_w) / 2;
                let mut cur_x = start_x;
                for ch in text.chars() {
                    let adv = self.draw_char(img, ch, cur_x, start_y, scale, ink);
                    cur_x += adv * scale;
                    if cur_x >= w - 4 {
                        break;
                    }
                }
            } else {
                // Dynamically downscale very long lines (>14 chars at 1x) so text never truncates
                let max_gh = text
                    .chars()
                    .filter_map(|c| self.glyphs.get(&c).map(|g| g.gh))
                    .max()
                    .unwrap_or(8)
                    .max(9);
                let mut tmp: RgbaImage = ImageBuffer::new(base_px_w + 2, max_gh);
                let mut cur_x = 0u32;
                for ch in text.chars() {
                    let adv = self.draw_char(&mut tmp, ch, cur_x, 0, 1, ink);
                    cur_x += adv;
                }
                let target_w = w - 12;
                let target_h = ((max_gh * target_w) / base_px_w.max(1)).clamp(8, 22);
                let resized = image::imageops::resize(
                    &tmp,
                    target_w,
                    target_h,
                    image::imageops::FilterType::Nearest,
                );
                let paste_y = top_pad + (line_idx as u32) * line_step + (24u32.saturating_sub(target_h)) / 2;
                for ry in 0..resized.height() {
                    for rx in 0..resized.width() {
                        let px = *resized.get_pixel(rx, ry);
                        if px[3] > 64 {
                            let dx = 6 + rx;
                            let dy = paste_y + ry;
                            if dx < w && dy < h {
                                img.put_pixel(dx, dy, Rgba([px[0], px[1], px[2], 255]));
                            }
                        }
                    }
                }
            }
        }
    }

    fn char_advance(&self, ch: char) -> u32 {
        if ch == ' ' {
            return 4;
        }
        if let Some(g) = self.glyphs.get(&ch) {
            return (g.content_width + 1).max(3);
        }
        if let Some(_bits) = fallback_5x7_bitmap(ch) {
            return 6;
        }
        5
    }

    fn draw_char(
        &self,
        dst: &mut RgbaImage,
        ch: char,
        x0: u32,
        y0: u32,
        scale: u32,
        ink: [u8; 3],
    ) -> u32 {
        if ch == ' ' {
            return 4;
        }
        if let Some(g) = self.glyphs.get(&ch) {
            let draw_w = g.content_width.min(g.gw);
            for gy_off in 0..g.gh {
                for gx_off in 0..draw_w {
                    let px = *g.sheet.get_pixel(g.gx + gx_off, g.gy + gy_off);
                    if px[3] < 64 {
                        continue;
                    }
                    let c = Rgba([
                        ((px[0] as u16 * ink[0] as u16) / 255) as u8,
                        ((px[1] as u16 * ink[1] as u16) / 255) as u8,
                        ((px[2] as u16 * ink[2] as u16) / 255) as u8,
                        255,
                    ]);
                    put_scaled_pixel(dst, x0 + gx_off * scale, y0 + gy_off * scale, scale, c);
                }
            }
            return (g.content_width + 1).max(3);
        }

        if let Some(rows) = fallback_5x7_bitmap(ch) {
            for (ry, &bits) in rows.iter().enumerate() {
                for rx in 0..5u32 {
                    if (bits >> (4 - rx)) & 1 != 0 {
                        put_scaled_pixel(
                            dst,
                            x0 + rx * scale,
                            y0 + (ry as u32) * scale,
                            scale,
                            Rgba([ink[0], ink[1], ink[2], 255]),
                        );
                    }
                }
            }
            return 6;
        }
        5
    }
}

fn put_scaled_pixel(dst: &mut RgbaImage, x: u32, y: u32, scale: u32, color: Rgba<u8>) {
    for sy in 0..scale {
        let py = y + sy;
        if py >= dst.height() {
            continue;
        }
        for sx in 0..scale {
            let px = x + sx;
            if px >= dst.width() {
                continue;
            }
            dst.put_pixel(px, py, color);
        }
    }
}

pub(crate) fn measure_glyph_width(sheet: &RgbaImage, gx: u32, gy: u32, gw: u32, gh: u32) -> u32 {
    let mut max_col = 0u32;
    let mut found = false;
    let x_end = gx.saturating_add(gw).min(sheet.width());
    let y_end = gy.saturating_add(gh).min(sheet.height());
    for py in gy..y_end {
        for px in gx..x_end {
            let p = sheet.get_pixel(px, py);
            if p[3] >= 64 {
                max_col = max_col.max((px - gx) + 1);
                found = true;
            }
        }
    }
    if found {
        max_col
    } else {
        (gw / 2).max(2)
    }
}

fn sign_ink_rgb(color: &str) -> [u8; 3] {
    match color.to_lowercase().as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [176, 46, 38],
        "green" => [94, 124, 22],
        "brown" => [131, 84, 50],
        "blue" => [60, 68, 170],
        "purple" => [137, 50, 184],
        "cyan" => [22, 156, 156],
        "light_gray" | "silver" => [157, 157, 151],
        "gray" => [71, 79, 82],
        "pink" => [243, 139, 170],
        "lime" => [128, 199, 31],
        "yellow" => [254, 216, 61],
        "light_blue" => [58, 179, 218],
        "magenta" => [199, 78, 189],
        "orange" => [249, 128, 29],
        _ => [24, 20, 16],
    }
}

fn sign_wood_rgb(block_name: &str) -> [u8; 3] {
    let s = block_name.to_lowercase();
    if s.contains("dark_oak") {
        [72, 48, 28]
    } else if s.contains("spruce") {
        [108, 80, 48]
    } else if s.contains("birch") {
        [210, 195, 145]
    } else if s.contains("jungle") {
        [160, 115, 80]
    } else if s.contains("acacia") {
        [168, 90, 50]
    } else if s.contains("mangrove") {
        [118, 54, 48]
    } else if s.contains("cherry") {
        [226, 178, 172]
    } else if s.contains("bamboo") {
        [196, 175, 76]
    } else if s.contains("crimson") {
        [101, 48, 70]
    } else if s.contains("warped") {
        [43, 104, 99]
    } else if s.contains("pale_oak") {
        [228, 218, 214]
    } else {
        [162, 130, 78]
    }
}

fn extract_sign_wood(short_lower: &str) -> &str {
    ["_wall_hanging_sign", "_hanging_sign", "_wall_sign", "_sign"]
        .iter()
        .find_map(|sfx| short_lower.strip_suffix(sfx))
        .filter(|w| !w.is_empty())
        .unwrap_or("oak")
}

pub fn sign_base_key(block_name: &str) -> String {
    let (_, short) = split_namespaced(block_name);
    let short_lower = short.to_ascii_lowercase();
    let is_hanging = short_lower.contains("hanging_sign");
    let wood = extract_sign_wood(&short_lower);
    if is_hanging {
        format!("base:hanging:{}", wood)
    } else {
        format!("base:sign:{}", wood)
    }
}

fn is_sign_block_name(short_name: &str) -> bool {
    let s = short_name.to_ascii_lowercase();
    s.ends_with("_sign") || s.contains("hanging_sign") || s == "sign"
}

fn crop_sign_entity_panel(tex: &RgbaImage, w: u32, h: u32, is_hanging: bool) -> RgbaImage {
    let (tw, th) = tex.dimensions();
    if tw >= 64 && th >= 32 {
        let sx = (tw / 64).max(1);
        let sy = (th / 32).max(1);
        let (cx, cy, cw, ch) = if is_hanging {
            (2 * sx, 14 * sy, 16 * sx, 10 * sy)
        } else {
            (2 * sx, 2 * sy, 24 * sx, 12 * sy)
        };
        if cx + cw <= tw && cy + ch <= th {
            let cropped = image::imageops::crop_imm(tex, cx, cy, cw, ch).to_image();
            return image::imageops::resize(&cropped, w, h, image::imageops::FilterType::Nearest);
        }
    }
    image::imageops::resize(tex, w, h, image::imageops::FilterType::Nearest)
}

fn procedural_sign_wood_panel(w: u32, h: u32, wood_tint: [u8; 3]) -> RgbaImage {
    let mut bg = ImageBuffer::new(w, h);
    for y in 0..h {
        let plank = ((y / 16) % 2) as i16;
        for x in 0..w {
            let grain = (((x * 7 + y * 13) % 11) as i16) - 5;
            let shade = plank * 6 + grain;
            let r = (wood_tint[0] as i16 + shade).clamp(0, 255) as u8;
            let g = (wood_tint[1] as i16 + shade).clamp(0, 255) as u8;
            let b = (wood_tint[2] as i16 + shade).clamp(0, 255) as u8;
            bg.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
    bg
}

fn resolve_sign_base_panel(block_name: &str, assets: &AssetIndex) -> RgbaImage {
    let w = 256u32;
    let h = 128u32;
    let (ns, short) = split_namespaced(block_name);
    let short_lower = short.to_ascii_lowercase();
    let is_hanging = short_lower.contains("hanging_sign");
    let wood = extract_sign_wood(&short_lower);

    if let Some(tex) = assets.load_sign_entity_texture(block_name) {
        return crop_sign_entity_panel(&tex, w, h, is_hanging);
    }

    let mut plank_candidates = vec![format!("{}:block/{}_planks", ns, wood)];
    if ns != "minecraft" {
        plank_candidates.push(format!("minecraft:block/{}_planks", wood));
    }
    if wood == "bamboo" {
        plank_candidates.push(format!("{}:block/bamboo_mosaic", ns));
        if ns != "minecraft" {
            plank_candidates.push("minecraft:block/bamboo_mosaic".to_string());
        }
    }

    for cand in &plank_candidates {
        if let Some(plank) = assets.load_texture_image(cand) {
            let (pw, ph) = plank.dimensions();
            if pw > 0 && ph > 0 {
                let mut tiled: RgbaImage = ImageBuffer::new(pw * 2, ph);
                for y in 0..ph {
                    for x in 0..pw {
                        let px = *plank.get_pixel(x, y);
                        tiled.put_pixel(x, y, px);
                        tiled.put_pixel(x + pw, y, px);
                    }
                }
                return image::imageops::resize(&tiled, w, h, image::imageops::FilterType::Nearest);
            }
        }
    }

    procedural_sign_wood_panel(w, h, sign_wood_rgb(block_name))
}

pub fn build_sign_atlas_with_assets(
    grid: &crate::anvil::VoxelGrid,
    assets: &AssetIndex,
    font: &FontRenderer,
) -> SignAtlas {
    let mut base_block_by_key: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();

    for bs in &grid.palette {
        if !bs.is_air && is_sign_block_name(bs.short_name()) {
            let bk = sign_base_key(&bs.name);
            base_block_by_key.entry(bk).or_insert_with(|| bs.name.clone());
        }
    }

    let mut keys: Vec<_> = grid.signs.keys().copied().collect();
    keys.sort();
    for &pos in &keys {
        let block_name = &grid.get_block(pos.0, pos.1, pos.2).name;
        let bk = sign_base_key(block_name);
        base_block_by_key
            .entry(bk)
            .or_insert_with(|| block_name.clone());
    }

    if grid.signs.is_empty() && base_block_by_key.is_empty() {
        return empty_sign_atlas();
    }

    let mut base_panels: HashMap<String, RgbaImage> =
        HashMap::with_capacity(base_block_by_key.len());
    let mut entries: Vec<(String, RgbaImage)> =
        Vec::with_capacity(base_block_by_key.len() + keys.len() * 2);

    for (bk, block_name) in &base_block_by_key {
        let base_img = resolve_sign_base_panel(block_name, assets);
        entries.push((bk.clone(), base_img.clone()));
        base_panels.insert(bk.clone(), base_img);
    }

    for pos in keys {
        let sd = &grid.signs[&pos];
        let block_name = &grid.get_block(pos.0, pos.1, pos.2).name;
        let bk = sign_base_key(block_name);
        let base_img = &base_panels[&bk];

        if sd.front_lines.iter().any(|s| !s.is_empty()) {
            let mut im = base_img.clone();
            font.render_sign_text_onto(&mut im, &sd.front_lines, &sd.front_color);
            entries.push((format!("{},{},{},front", pos.0, pos.1, pos.2), im));
        }
        if sd.back_lines.iter().any(|s| !s.is_empty()) {
            let mut im = base_img.clone();
            font.render_sign_text_onto(&mut im, &sd.back_lines, &sd.back_color);
            entries.push((format!("{},{},{},back", pos.0, pos.1, pos.2), im));
        }
    }

    pack_sign_entries(entries)
}

fn empty_sign_atlas() -> SignAtlas {
    let mut fallback = ImageBuffer::from_pixel(64, 64, Rgba([162, 130, 78, 255]));
    blit_with_extruded_padding(
        &ImageBuffer::from_pixel(16, 16, Rgba([162, 130, 78, 255])),
        &mut fallback,
        8,
        8,
        8,
    );
    SignAtlas {
        image: fallback,
        rects: HashMap::new(),
    }
}

fn pack_sign_entries(mut entries: Vec<(String, RgbaImage)>) -> SignAtlas {
    if entries.is_empty() {
        let fallback = ImageBuffer::from_pixel(64, 64, Rgba([162, 130, 78, 255]));
        return SignAtlas {
            image: fallback,
            rects: HashMap::new(),
        };
    }

    // Handle >450 sign panels gracefully by downscaling panels so all fit inside a single atlas
    if entries.len() > 1800 {
        for (_, im) in &mut entries {
            *im = image::imageops::resize(im, 64, 32, image::imageops::FilterType::Triangle);
        }
    } else if entries.len() > 420 {
        for (_, im) in &mut entries {
            *im = image::imageops::resize(im, 128, 64, image::imageops::FilterType::Triangle);
        }
    }

    let pad = 4u32;
    let sizes: Vec<(u32, u32)> = entries
        .iter()
        .map(|(_, im)| (im.width() + pad * 2, im.height() + pad * 2))
        .collect();
    let mut dim = 512u32;
    let (atlas_w, atlas_h, placements) = loop {
        if let Some(p) = try_shelf_pack(&sizes, dim, dim) {
            break (dim, dim, p.into_iter().map(Some).collect::<Vec<_>>());
        }
        if dim >= 4096 {
            if let Some(p) = try_shelf_pack(&sizes, 4096, 8192) {
                break (4096, 8192, p.into_iter().map(Some).collect::<Vec<_>>());
            }
            if let Some(p) = try_shelf_pack(&sizes, 8192, 8192) {
                break (8192, 8192, p.into_iter().map(Some).collect::<Vec<_>>());
            }
            let p = shelf_pack_best_effort(&sizes, 8192, 8192);
            break (8192, 8192, p);
        }
        dim *= 2;
    };

    let mut atlas_img: RgbaImage =
        ImageBuffer::from_pixel(atlas_w, atlas_h, Rgba([162, 130, 78, 255]));
    let mut rects = HashMap::new();
    let aw = atlas_w as f32;
    let ah = atlas_h as f32;
    let half_u = 0.25 / aw;
    let half_v = 0.25 / ah;

    for ((k, tile), placement) in entries.into_iter().zip(placements) {
        let Some((ox, oy)) = placement else {
            continue;
        };
        let px = ox + pad;
        let py = oy + pad;
        if px + tile.width() <= atlas_w && py + tile.height() <= atlas_h {
            blit_with_extruded_padding(&tile, &mut atlas_img, px, py, pad);
        }
        let u_min = ((px as f32) / aw + half_u).clamp(0.0, 1.0);
        let u_max = (((px + tile.width()) as f32) / aw - half_u).clamp(u_min, 1.0);
        let v_top = (1.0 - ((py as f32) / ah + half_v)).clamp(0.0, 1.0);
        let v_bot = (1.0 - (((py + tile.height()) as f32) / ah - half_v)).clamp(0.0, v_top);
        rects.insert(
            k,
            UvRect {
                bucket: RenderBucket::TerrainOpaque,
                u_min,
                u_max,
                v_top,
                v_bot,
            },
        );
    }

    SignAtlas {
        image: atlas_img,
        rects,
    }
}

/// Compact embedded 5x7 fallback bitmap font for signs.
fn fallback_5x7_bitmap(ch: char) -> Option<[u8; 7]> {
    let upper = match ch {
        'а' => 'А', 'б' => 'Б', 'в' => 'В', 'г' => 'Г', 'д' => 'Д', 'е' => 'Е', 'ё' => 'Ё',
        'ж' => 'Ж', 'з' => 'З', 'и' => 'И', 'й' => 'Й', 'к' => 'К', 'л' => 'Л', 'м' => 'М',
        'н' => 'Н', 'о' => 'О', 'п' => 'П', 'р' => 'Р', 'с' => 'С', 'т' => 'Т', 'у' => 'У',
        'ф' => 'Ф', 'х' => 'Х', 'ц' => 'Ц', 'ч' => 'Ч', 'ш' => 'Ш', 'щ' => 'Щ', 'ъ' => 'Ъ',
        'ы' => 'Ы', 'ь' => 'Ь', 'э' => 'Э', 'ю' => 'Ю', 'я' => 'Я',
        other => other.to_ascii_uppercase(),
    };
    Some(match upper {
        // Extended uppercase
        'А' => [0b00100, 0b01010, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001],
        'Б' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b11110],
        'В' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'Г' => [0b11111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000],
        'Д' => [0b00110, 0b01010, 0b01010, 0b01010, 0b01010, 0b11111, 0b10001],
        'Е' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'Ё' => [0b01010, 0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'Ж' => [0b10101, 0b10101, 0b01110, 0b00100, 0b01110, 0b10101, 0b10101],
        'З' => [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
        'И' => [0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b10001, 0b10001],
        'Й' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b10001],
        'К' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'Л' => [0b00111, 0b01001, 0b01001, 0b01001, 0b01001, 0b10001, 0b10001],
        'М' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'Н' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'О' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'П' => [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001],
        'Р' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'С' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'Т' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'У' => [0b10001, 0b10001, 0b10001, 0b01111, 0b00001, 0b10001, 0b01110],
        'Ф' => [0b00100, 0b01110, 0b10101, 0b10101, 0b01110, 0b00100, 0b00100],
        'Х' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Ц' => [0b10010, 0b10010, 0b10010, 0b10010, 0b10010, 0b11111, 0b00001],
        'Ч' => [0b10001, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b00001],
        'Ш' => [0b10101, 0b10101, 0b10101, 0b10101, 0b10101, 0b10101, 0b11111],
        'Щ' => [0b10101, 0b10101, 0b10101, 0b10101, 0b10101, 0b11111, 0b00001],
        'Ъ' => [0b11000, 0b01000, 0b01000, 0b01110, 0b01001, 0b01001, 0b01110],
        'Ы' => [0b10001, 0b10001, 0b11101, 0b10011, 0b10011, 0b11101, 0b10001],
        'Ь' => [0b10000, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b11110],
        'Э' => [0b01110, 0b10001, 0b00001, 0b00111, 0b00001, 0b10001, 0b01110],
        'Ю' => [0b10010, 0b10101, 0b11101, 0b10101, 0b10101, 0b10101, 0b10010],
        'Я' => [0b01111, 0b10001, 0b10001, 0b01111, 0b00101, 0b01001, 0b10001],
        // Latin + Digits
        'A' => [0b00100, 0b01010, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100],
        '?' => [0b01110, 0b10001, 0b00001, 0b00110, 0b00100, 0b00000, 0b00100],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00110, 0b00110],
        ',' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00110, 0b00100, 0b01000],
        ':' => [0b00000, 0b00110, 0b00110, 0b00000, 0b00110, 0b00110, 0b00000],
        _ => return None,
    })
}

pub struct AnimatedMaterialEntry {
    pub mat_name: String,
    pub frames: Vec<RgbaImage>,
    pub fps: f32,
    pub bucket: RenderBucket,
    pub unlit_emissive: bool,
}

fn sanitize_anim_stem(tex_ref: &str) -> String {
    let (_, rel) = split_namespaced(tex_ref);
    let trimmed = rel.trim_start_matches('/');
    let after_tex = trimmed.strip_prefix("textures/").unwrap_or(trimmed);
    let clean = after_tex.strip_prefix("block/").unwrap_or(after_tex);
    let stem = clean.rsplit('/').next().unwrap_or(clean);
    let stem = stem.strip_suffix(".png").unwrap_or(stem);
    let mut out = String::with_capacity(stem.len() + 6);
    out.push_str("anim_");
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    out
}

pub fn collect_animated_materials(
    requests: &HashSet<TexRequestKey>,
    assets: &AssetIndex,
) -> (HashMap<TexRequestKey, (String, RenderBucket)>, Vec<AnimatedMaterialEntry>) {
    let mut unique_refs: Vec<String> = Vec::new();
    let mut seen_refs: HashSet<&str> = HashSet::new();
    for k in requests {
        let low = k.tex_ref.to_ascii_lowercase();
        if low.contains("water_still") || low.contains("water_flow") || low.contains("water_overlay") {
            continue;
        }
        if seen_refs.insert(k.tex_ref.as_str()) {
            unique_refs.push(k.tex_ref.clone());
        }
    }

    let strip_map: HashMap<String, crate::assets::AnimatedStrip> = unique_refs
        .into_par_iter()
        .filter_map(|tr| assets.load_animated_texture_frames(&tr).map(|st| (tr, st)))
        .collect();

    if strip_map.is_empty() {
        return (HashMap::new(), Vec::new());
    }

    let mut anim_keys: Vec<TexRequestKey> = requests
        .iter()
        .filter(|k| strip_map.contains_key(&k.tex_ref))
        .cloned()
        .collect();

    let priority_of = |tex_ref: &str| -> u8 {
        let l = tex_ref.to_ascii_lowercase();
        if l.contains("campfire") || l.contains("fire") || l.contains("lava") {
            0
        } else if l.contains("portal") || l.contains("lantern") || l.contains("torch") || l.contains("furnace") {
            1
        } else if l.contains("prismarine") || l.contains("magma") || l.contains("sculk") || l.contains("seagrass") || l.contains("kelp") {
            2
        } else {
            3
        }
    };

    anim_keys.sort_by(|a, b| {
        priority_of(&a.tex_ref)
            .cmp(&priority_of(&b.tex_ref))
            .then_with(|| a.tex_ref.cmp(&b.tex_ref))
            .then_with(|| a.tint_rgb.cmp(&b.tint_rgb))
    });
    anim_keys.truncate(24);

    let mut lookup: HashMap<TexRequestKey, (String, RenderBucket)> = HashMap::with_capacity(anim_keys.len());
    let mut entries: Vec<AnimatedMaterialEntry> = Vec::with_capacity(anim_keys.len());
    let mut used_names: HashSet<String> = HashSet::new();

    for key in anim_keys {
        let Some(strip) = strip_map.get(&key.tex_ref) else {
            continue;
        };
        if strip.frames.len() < 2 {
            continue;
        }
        let bucket = classify_texture_bucket(&key.tex_ref, &strip.frames[0]);
        let is_gray = is_grayscale_image(&strip.frames[0]);
        let low = key.tex_ref.to_ascii_lowercase();
        let unlit_emissive = low.contains("campfire_fire")
            || low.contains("soul_campfire_fire")
            || low.ends_with("/fire_0")
            || low.ends_with("/fire_1")
            || low.ends_with("/soul_fire_0")
            || low.ends_with("/soul_fire_1")
            || low.contains("lava_still")
            || low.contains("lava_flow")
            || low.contains("nether_portal");

        let mut base_name = sanitize_anim_stem(&key.tex_ref);
        if let Some(rgb) = key.tint_rgb {
            if is_gray {
                base_name = format!("{}_t{:02x}{:02x}{:02x}", base_name, rgb[0], rgb[1], rgb[2]);
            }
        }
        let mut mat_name = base_name.clone();
        let mut suffix = 2usize;
        while !used_names.insert(mat_name.clone()) {
            mat_name = format!("{}_{}", base_name, suffix);
            suffix += 1;
        }

        let frames: Vec<RgbaImage> = strip
            .frames
            .iter()
            .map(|f| {
                let tinted = if let Some(rgb) = key.tint_rgb {
                    if is_gray {
                        apply_biome_tint(f, rgb)
                    } else {
                        f.clone()
                    }
                } else {
                    f.clone()
                };
                let cropped = if low.contains("_flow") && f.width() >= 32 && f.width() == f.height() {
                    let tile_side = (f.width() / 2).max(16);
                    image::imageops::crop_imm(&tinted, 0, 0, tile_side, tile_side).to_image()
                } else {
                    tinted
                };
                let mut up = image::imageops::resize(&cropped, 64, 64, image::imageops::FilterType::Nearest);
                if bucket == RenderBucket::TerrainOpaque {
                    for px in up.as_mut().as_chunks_mut::<4>().0 {
                        px[3] = 255;
                    }
                } else {
                    bleed_transparent_rgb(&mut up);
                }
                up
            })
            .collect();

        lookup.insert(key, (mat_name.clone(), bucket));
        entries.push(AnimatedMaterialEntry {
            mat_name,
            frames,
            fps: strip.fps,
            bucket,
            unlit_emissive,
        });
    }

    (lookup, entries)
}
