use fastnbt::Value;
use flate2::read::{GzDecoder, ZlibDecoder};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldCoords {
    pub min_x: i32,
    pub min_y: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub max_z: i32,
}

impl WorldCoords {
    pub fn normalized(self) -> Self {
        Self {
            min_x: self.min_x.min(self.max_x),
            min_y: self.min_y.min(self.max_y),
            min_z: self.min_z.min(self.max_z),
            max_x: self.min_x.max(self.max_x),
            max_y: self.min_y.max(self.max_y),
            max_z: self.min_z.max(self.max_z),
        }
    }

    pub fn size_x(&self) -> usize {
        ((self.max_x as i64) - (self.min_x as i64) + 1).max(0) as usize
    }

    pub fn size_y(&self) -> usize {
        ((self.max_y as i64) - (self.min_y as i64) + 1).max(0) as usize
    }

    pub fn size_z(&self) -> usize {
        ((self.max_z as i64) - (self.min_z as i64) + 1).max(0) as usize
    }

    pub fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        x >= self.min_x
            && x <= self.max_x
            && y >= self.min_y
            && y <= self.max_y
            && z >= self.min_z
            && z <= self.max_z
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockState {
    pub name: String,
    pub props: BTreeMap<String, String>,
    pub canonical_key: String,
    pub is_air: bool,
    pub is_water_tag: bool,
    pub waterlogged: bool,
}

impl BlockState {
    pub fn new(name: String, props: BTreeMap<String, String>) -> Self {
        let norm_name = if name.contains(':') {
            name.to_lowercase()
        } else {
            format!("minecraft:{}", name.to_lowercase())
        };
        let is_air = matches!(
            norm_name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        );
        let short_name = norm_name
            .split_once(':')
            .map(|(_, r)| r)
            .unwrap_or(&norm_name);
        let waterlogged = props
            .get("waterlogged")
            .map(|v| v == "true")
            .unwrap_or(false)
            || matches!(
                short_name,
                "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant"
            );
        let is_water_tag = norm_name == "minecraft:water"
            || norm_name.ends_with(":water")
            || norm_name == "minecraft:flowing_water"
            || norm_name.ends_with(":flowing_water")
            || norm_name == "minecraft:bubble_column";

        let canonical_key = Self::build_key(&norm_name, &props);
        Self {
            name: norm_name,
            props,
            canonical_key,
            is_air,
            is_water_tag,
            waterlogged,
        }
    }

    pub fn build_key(name: &str, props: &BTreeMap<String, String>) -> String {
        if props.is_empty() {
            return name.to_string();
        }
        let mut s = String::with_capacity(name.len() + props.len() * 12);
        s.push_str(name);
        s.push('[');
        for (idx, (k, v)) in props.iter().enumerate() {
            if idx > 0 {
                s.push(',');
            }
            s.push_str(k);
            s.push('=');
            s.push_str(v);
        }
        s.push(']');
        s
    }

    pub fn short_name(&self) -> &str {
        self.name
            .split_once(':')
            .map(|(_, r)| r)
            .unwrap_or(&self.name)
    }

    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props.get(key).map(|s| s.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct SignData {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub front_lines: [String; 4],
    pub back_lines: [String; 4],
    pub front_color: String,
    pub back_color: String,
}

#[derive(Debug, Clone)]
pub struct VoxelGrid {
    pub coords: WorldCoords,
    pub palette: Vec<BlockState>,
    pub air_id: u16,
    pub blocks: Vec<u16>,
    pub biome_palette: Vec<String>,
    pub biomes: Vec<u16>,
    pub signs: HashMap<(i32, i32, i32), SignData>,
    pub player_spawn_mc: Option<[f64; 3]>,
}

impl VoxelGrid {
    pub fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        if !self.coords.contains(x, y, z) {
            return None;
        }
        let dx = (x - self.coords.min_x) as usize;
        let dy = (y - self.coords.min_y) as usize;
        let dz = (z - self.coords.min_z) as usize;
        let sx = self.coords.size_x();
        let sz = self.coords.size_z();
        Some((dy * sz + dz) * sx + dx)
    }

    #[inline]
    pub fn get_block_id(&self, x: i32, y: i32, z: i32) -> u16 {
        if let Some(idx) = self.index(x, y, z) {
            self.blocks[idx]
        } else {
            self.air_id
        }
    }

    #[inline]
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> &BlockState {
        let id = self.get_block_id(x, y, z) as usize;
        &self.palette[id]
    }

    #[inline]
    pub fn get_biome_id(&self, gx: usize, gy: usize, gz: usize) -> usize {
        let sx = self.coords.size_x();
        let sz = self.coords.size_z();
        if self.biomes.len() == self.blocks.len() {
            return self.biomes[(gy * sz + gz) * sx + gx] as usize;
        }
        let wx = self.coords.min_x + gx as i32;
        let wy = self.coords.min_y + gy as i32;
        let wz = self.coords.min_z + gz as i32;
        let min_bx = self.coords.min_x.div_euclid(4);
        let min_by = self.coords.min_y.div_euclid(4);
        let min_bz = self.coords.min_z.div_euclid(4);
        let bsx = (self.coords.max_x.div_euclid(4) - min_bx + 1) as usize;
        let bsz = (self.coords.max_z.div_euclid(4) - min_bz + 1) as usize;
        let bx = (wx.div_euclid(4) - min_bx) as usize;
        let by = (wy.div_euclid(4) - min_by) as usize;
        let bz = (wz.div_euclid(4) - min_bz) as usize;
        self.biomes[(by * bsz + bz) * bsx + bx] as usize
    }

    #[inline]
    #[cfg(test)]
    pub fn get_biome(&self, x: i32, y: i32, z: i32) -> &str {
        if !self.coords.contains(x, y, z) {
            return "minecraft:plains";
        }
        let gx = (x - self.coords.min_x) as usize;
        let gy = (y - self.coords.min_y) as usize;
        let gz = (z - self.coords.min_z) as usize;
        let b_idx = self.get_biome_id(gx, gy, gz);
        self.biome_palette
            .get(b_idx)
            .map(|s| s.as_str())
            .unwrap_or("minecraft:plains")
    }
}

#[derive(Debug, Deserialize)]
struct SignTextNbt {
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    messages: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct BlockEntityNbt {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    y: Option<i32>,
    #[serde(default)]
    z: Option<i32>,
    #[serde(default)]
    front_text: Option<SignTextNbt>,
    #[serde(default)]
    back_text: Option<SignTextNbt>,
    #[serde(rename = "Color", default)]
    legacy_color: Option<String>,
    #[serde(rename = "Text1", default)]
    text1: Option<Value>,
    #[serde(rename = "Text2", default)]
    text2: Option<Value>,
    #[serde(rename = "Text3", default)]
    text3: Option<Value>,
    #[serde(rename = "Text4", default)]
    text4: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ChunkNbt {
    #[serde(rename = "DataVersion", default)]
    data_version: i32,
    #[serde(rename = "xPos", default)]
    x_pos: i32,
    #[serde(rename = "zPos", default)]
    z_pos: i32,
    #[serde(default)]
    sections: Vec<SectionNbt>,
    #[serde(default)]
    block_entities: Vec<BlockEntityNbt>,
    #[serde(rename = "Level", default)]
    level: Option<LegacyLevelNbt>,
}

#[derive(Debug, Deserialize)]
struct LegacyLevelNbt {
    #[serde(rename = "xPos", default)]
    x_pos: i32,
    #[serde(rename = "zPos", default)]
    z_pos: i32,
    #[serde(rename = "Sections", default)]
    sections: Vec<LegacySectionNbt>,
    #[serde(rename = "TileEntities", default)]
    tile_entities: Vec<BlockEntityNbt>,
    #[serde(rename = "Biomes", default)]
    biomes: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct SectionNbt {
    #[serde(rename = "Y", default)]
    y: i8,
    #[serde(default)]
    block_states: Option<BlockStatesNbt>,
    #[serde(default)]
    biomes: Option<BiomesNbt>,
}

#[derive(Debug, Deserialize)]
struct BlockStatesNbt {
    #[serde(default)]
    palette: Vec<Value>,
    #[serde(default)]
    data: Option<fastnbt::LongArray>,
}

#[derive(Debug, Deserialize)]
struct BiomesNbt {
    #[serde(default)]
    palette: Vec<Value>,
    #[serde(default)]
    data: Option<fastnbt::LongArray>,
}

#[derive(Debug, Deserialize)]
struct PaletteEntryNbt {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Properties", default)]
    properties: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct LegacySectionNbt {
    #[serde(rename = "Y", default)]
    y: i8,
    #[serde(rename = "Palette", default)]
    palette: Vec<PaletteEntryNbt>,
    #[serde(rename = "BlockStates", default)]
    block_states: Option<fastnbt::LongArray>,
    #[serde(rename = "Blocks", default)]
    blocks: Option<fastnbt::ByteArray>,
    #[serde(rename = "Data", default)]
    data: Option<fastnbt::ByteArray>,
}

struct CompactChunk {
    chunk_x: i32,
    chunk_z: i32,
    local_palette: Vec<BlockState>,
    local_biome_palette: Vec<String>,
    sections: Vec<CompactSection>,
    signs: Vec<SignData>,
}

struct CompactSection {
    sec_y: i32,
    block_indices: Vec<u16>, // 4096 entries mapping to CompactChunk::local_palette
    biome_indices: Vec<u8>,  // 64 entries mapping to CompactChunk::local_biome_palette (4x4x4)
}

fn value_to_prop_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Byte(b) => {
            if *b == 0 {
                "false".to_string()
            } else if *b == 1 {
                "true".to_string()
            } else {
                b.to_string()
            }
        }
        Value::Int(i) => i.to_string(),
        Value::Short(s) => s.to_string(),
        Value::Long(l) => l.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(d) => d.to_string(),
        _ => String::new(),
    }
}

fn populate_default_block_props(name: &str, props: &mut BTreeMap<String, String>) {
    let lower = name.trim().to_ascii_lowercase();
    let short = lower.split(':').next_back().unwrap_or(lower.as_str());

    let mut set = |k: &str, v: &str| {
        props.entry(k.to_string()).or_insert_with(|| v.to_string());
    };

    if short.ends_with("_stairs") {
        set("facing", "north");
        set("half", "bottom");
        set("shape", "straight");
        set("waterlogged", "false");
    } else if short.ends_with("_slab") {
        set("type", "bottom");
        set("waterlogged", "false");
    } else if short == "mushroom_stem" || short.ends_with("_mushroom_block") {
        set("up", "true");
        set("down", "true");
        set("north", "true");
        set("south", "true");
        set("east", "true");
        set("west", "true");
    } else if short.ends_with("_log")
        || short.ends_with("_wood")
        || short.ends_with("_stem")
        || short.ends_with("_hyphae")
        || short.ends_with("_pillar")
        || short.ends_with("_froglight")
        || matches!(
            short,
            "hay_block"
                | "basalt"
                | "polished_basalt"
                | "bone_block"
                | "chain"
                | "bamboo_block"
                | "stripped_bamboo_block"
                | "deepslate"
                | "muddy_mangrove_roots"
        )
    {
        set("axis", "y");
    } else if short.ends_with("_trapdoor") {
        set("facing", "north");
        set("half", "bottom");
        set("open", "false");
        set("powered", "false");
        set("waterlogged", "false");
    } else if short.ends_with("_door") {
        set("facing", "north");
        set("half", "lower");
        set("hinge", "left");
        set("open", "false");
        set("powered", "false");
    } else if short.ends_with("_fence_gate") {
        set("facing", "north");
        set("in_wall", "false");
        set("open", "false");
        set("powered", "false");
    } else if short.ends_with("_wall") {
        set("up", "true");
        set("north", "none");
        set("south", "none");
        set("east", "none");
        set("west", "none");
        set("waterlogged", "false");
    } else if short.ends_with("_fence") || short.ends_with("_pane") || short == "iron_bars" {
        set("north", "false");
        set("south", "false");
        set("east", "false");
        set("west", "false");
        set("waterlogged", "false");
    } else if short.ends_with("_button") || short == "lever" {
        set("face", "wall");
        set("facing", "north");
        set("powered", "false");
    } else if matches!(
        short,
        "ladder"
            | "carved_pumpkin"
            | "jack_o_lantern"
            | "stonecutter"
            | "loom"
            | "beehive"
            | "bee_nest"
            | "vault"
            | "anvil"
            | "chipped_anvil"
            | "damaged_anvil"
    ) || short.ends_with("_wall_torch")
        || short.ends_with("_wall_sign")
        || short.ends_with("_wall_hanging_sign")
        || short.ends_with("_wall_banner")
        || short.ends_with("_glazed_terracotta")
    {
        set("facing", "north");
    } else if matches!(short, "furnace" | "blast_furnace" | "smoker") {
        set("facing", "north");
        set("lit", "false");
    } else if matches!(short, "dispenser" | "dropper") {
        set("facing", "north");
        set("triggered", "false");
    } else if short == "observer" {
        set("facing", "south");
        set("powered", "false");
    } else if matches!(short, "piston" | "sticky_piston") {
        set("facing", "north");
        set("extended", "false");
    } else if short == "lectern" {
        set("facing", "north");
        set("has_book", "false");
        set("powered", "false");
    } else if short == "repeater" {
        set("facing", "north");
        set("delay", "1");
        set("locked", "false");
        set("powered", "false");
    } else if short == "comparator" {
        set("facing", "north");
        set("mode", "compare");
        set("powered", "false");
    } else if matches!(short, "chest" | "trapped_chest") {
        set("facing", "north");
        set("type", "single");
        set("waterlogged", "false");
    } else if short == "ender_chest" {
        set("facing", "north");
        set("waterlogged", "false");
    } else if short == "barrel" {
        set("facing", "north");
        set("open", "false");
    } else if short == "decorated_pot" {
        set("cracked", "false");
        set("facing", "north");
        set("waterlogged", "false");
    } else if matches!(short, "campfire" | "soul_campfire") {
        set("facing", "north");
        set("lit", "true");
        set("signal_fire", "false");
        set("waterlogged", "false");
    } else if short == "cocoa" {
        set("facing", "north");
        set("age", "0");
    } else if short.ends_with("_bed") {
        set("facing", "north");
        set("occupied", "false");
        set("part", "foot");
    } else if short == "tripwire_hook" {
        set("attached", "false");
        set("facing", "north");
        set("powered", "false");
    } else if short == "chiseled_bookshelf" {
        set("facing", "north");
        set("slot_0_occupied", "false");
        set("slot_1_occupied", "false");
        set("slot_2_occupied", "false");
        set("slot_3_occupied", "false");
        set("slot_4_occupied", "false");
        set("slot_5_occupied", "false");
    } else if short.ends_with("_shelf") {
        set("facing", "north");
        set("powered", "false");
        set("side_chain", "unconnected");
        set("waterlogged", "false");
    } else if short == "bell" {
        set("attachment", "floor");
        set("facing", "north");
        set("powered", "false");
    } else if short == "grindstone" {
        set("face", "wall");
        set("facing", "north");
    } else if short == "rail" {
        set("shape", "north_south");
        set("waterlogged", "false");
    } else if matches!(short, "powered_rail" | "detector_rail" | "activator_rail") {
        set("powered", "false");
        set("shape", "north_south");
        set("waterlogged", "false");
    } else if short == "end_rod" {
        set("facing", "up");
    } else if short == "amethyst_cluster" || short.ends_with("_amethyst_bud") {
        set("facing", "up");
        set("waterlogged", "false");
    } else if short == "lightning_rod" || short.ends_with("_lightning_rod") {
        set("facing", "up");
        set("powered", "false");
        set("waterlogged", "false");
    } else if short == "pointed_dripstone" {
        set("thickness", "tip");
        set("vertical_direction", "up");
        set("waterlogged", "false");
    } else if short == "fire" {
        set("age", "0");
        set("north", "false");
        set("south", "false");
        set("east", "false");
        set("west", "false");
        set("up", "false");
    } else if (short == "lantern" || short == "soul_lantern" || short.ends_with("_lantern"))
        && short != "sea_lantern"
    {
        set("hanging", "false");
        set("waterlogged", "false");
    } else if matches!(short, "pink_petals" | "wildflowers") {
        set("facing", "north");
        set("flower_amount", "1");
    } else if short == "leaf_litter" {
        set("facing", "north");
        set("segment_amount", "1");
    } else if short == "pale_moss_carpet" {
        set("bottom", "true");
        set("north", "none");
        set("south", "none");
        set("east", "none");
        set("west", "none");
    }
}

fn palette_entry_to_block_state(entry: &PaletteEntryNbt) -> BlockState {
    let mut props = BTreeMap::new();
    for (k, v) in &entry.properties {
        let s = value_to_prop_string(v);
        if !s.is_empty() {
            props.insert(k.clone(), s);
        }
    }
    if props.is_empty() {
        populate_default_block_props(&entry.name, &mut props);
    }
    BlockState::new(entry.name.clone(), props)
}

fn palette_value_to_block_state(val: &Value) -> BlockState {
    match val {
        Value::String(name) => {
            let mut props = BTreeMap::new();
            populate_default_block_props(name, &mut props);
            BlockState::new(name.clone(), props)
        }
        Value::Compound(map) => {
            let name = map
                .get("Name")
                .or_else(|| map.get("id"))
                .or_else(|| map.get(""))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "minecraft:air".to_string());
            let mut props = BTreeMap::new();
            if let Some(Value::Compound(prop_map)) =
                map.get("Properties").or_else(|| map.get("properties"))
            {
                for (k, v) in prop_map {
                    let s = value_to_prop_string(v);
                    if !s.is_empty() {
                        props.insert(k.clone(), s);
                    }
                }
            }
            if props.is_empty() {
                populate_default_block_props(&name, &mut props);
            }
            BlockState::new(name, props)
        }
        _ => BlockState::new("minecraft:air".to_string(), BTreeMap::new()),
    }
}

fn biome_value_to_name(val: &Value) -> &str {
    match val {
        Value::String(s) => s.as_str(),
        Value::Compound(map) => map
            .get("")
            .or_else(|| map.get("id"))
            .or_else(|| map.get("Name"))
            .and_then(|v| match v {
                Value::String(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("minecraft:plains"),
        _ => "minecraft:plains",
    }
}

fn decode_packed_and_remap_u16(
    data: &[i64],
    remap: &[u16],
    total_entries: usize,
    min_bits: usize,
    spanning: bool,
) -> Vec<u16> {
    let palette_len = remap.len();
    if palette_len <= 1 || data.is_empty() {
        return vec![remap.first().copied().unwrap_or(0); total_entries];
    }
    let bits_needed = (usize::BITS - (palette_len - 1).leading_zeros()) as usize;
    let bits = bits_needed.max(min_bits);
    let mask = (1u64 << bits) - 1;
    let max_idx = palette_len - 1;
    let mut out = vec![0u16; total_entries];

    if !spanning {
        let per_long = 64 / bits;
        if per_long == 0 {
            return out;
        }
        let mut pos = 0usize;
        for &word in data {
            let mut w = word as u64;
            let count = per_long.min(total_entries - pos);
            for _ in 0..count {
                let idx = ((w & mask) as usize).min(max_idx);
                out[pos] = remap[idx];
                pos += 1;
                w >>= bits;
            }
            if pos >= total_entries {
                break;
            }
        }
    } else {
        for (i, dst) in out.iter_mut().enumerate() {
            let bit_index = i * bits;
            let long_idx = bit_index >> 6;
            let bit_off = bit_index & 63;
            let lo = data.get(long_idx).copied().unwrap_or(0) as u64;
            let mut val = lo >> bit_off;
            if bit_off + bits > 64 {
                let hi = data.get(long_idx + 1).copied().unwrap_or(0) as u64;
                val |= hi << (64 - bit_off);
            }
            let idx = ((val & mask) as usize).min(max_idx);
            *dst = remap[idx];
        }
    }
    out
}

fn decode_packed_and_remap_u8(
    data: &[i64],
    remap: &[u8],
    total_entries: usize,
    min_bits: usize,
) -> Vec<u8> {
    let palette_len = remap.len();
    if palette_len <= 1 || data.is_empty() {
        return vec![remap.first().copied().unwrap_or(0); total_entries];
    }
    let bits_needed = (usize::BITS - (palette_len - 1).leading_zeros()) as usize;
    let bits = bits_needed.max(min_bits);
    let mask = (1u64 << bits) - 1;
    let max_idx = palette_len - 1;
    let per_long = 64 / bits;
    let mut out = vec![0u8; total_entries];
    if per_long == 0 {
        return out;
    }
    let mut pos = 0usize;
    for &word in data {
        let mut w = word as u64;
        let count = per_long.min(total_entries - pos);
        for _ in 0..count {
            let idx = ((w & mask) as usize).min(max_idx);
            out[pos] = remap[idx];
            pos += 1;
            w >>= bits;
        }
        if pos >= total_entries {
            break;
        }
    }
    out
}

fn legacy_biome_id_to_name(id: i32) -> &'static str {
    match id {
        0 => "minecraft:ocean",
        1 => "minecraft:plains",
        2 => "minecraft:desert",
        3 => "minecraft:windswept_hills",
        4 => "minecraft:forest",
        5 => "minecraft:taiga",
        6 => "minecraft:swamp",
        7 => "minecraft:river",
        8 => "minecraft:nether_wastes",
        9 => "minecraft:the_end",
        10 => "minecraft:frozen_ocean",
        11 => "minecraft:frozen_river",
        12 => "minecraft:snowy_plains",
        13 => "minecraft:snowy_mountains",
        14 => "minecraft:mushroom_fields",
        15 => "minecraft:mushroom_field_shore",
        16 => "minecraft:beach",
        17 => "minecraft:desert_hills",
        18 => "minecraft:wooded_hills",
        19 => "minecraft:taiga_hills",
        20 => "minecraft:mountain_edge",
        21 => "minecraft:jungle",
        22 => "minecraft:jungle_hills",
        23 => "minecraft:sparse_jungle",
        24 => "minecraft:deep_ocean",
        25 => "minecraft:stony_shore",
        26 => "minecraft:snowy_beach",
        27 => "minecraft:birch_forest",
        28 => "minecraft:birch_forest_hills",
        29 => "minecraft:dark_forest",
        30 => "minecraft:snowy_taiga",
        31 => "minecraft:snowy_taiga_hills",
        32 => "minecraft:old_growth_pine_taiga",
        33 => "minecraft:giant_tree_taiga_hills",
        34 => "minecraft:windswept_forest",
        35 => "minecraft:savanna",
        36 => "minecraft:savanna_plateau",
        37 => "minecraft:badlands",
        38 => "minecraft:wooded_badlands",
        39 => "minecraft:badlands_plateau",
        40 => "minecraft:small_end_islands",
        41 => "minecraft:end_midlands",
        42 => "minecraft:end_highlands",
        43 => "minecraft:end_barrens",
        44 => "minecraft:warm_ocean",
        45 => "minecraft:lukewarm_ocean",
        46 => "minecraft:cold_ocean",
        47 => "minecraft:deep_warm_ocean",
        48 => "minecraft:deep_lukewarm_ocean",
        49 => "minecraft:deep_cold_ocean",
        50 => "minecraft:deep_frozen_ocean",
        127 => "minecraft:the_void",
        129 => "minecraft:sunflower_plains",
        130 => "minecraft:desert_lakes",
        131 => "minecraft:windswept_gravelly_hills",
        132 => "minecraft:flower_forest",
        133 => "minecraft:taiga_mountains",
        134 => "minecraft:swamp_hills",
        140 => "minecraft:ice_spikes",
        149 => "minecraft:modified_jungle",
        151 => "minecraft:modified_jungle_edge",
        155 => "minecraft:old_growth_birch_forest",
        156 => "minecraft:tall_birch_hills",
        157 => "minecraft:dark_forest_hills",
        158 => "minecraft:snowy_taiga_mountains",
        160 => "minecraft:old_growth_spruce_taiga",
        161 => "minecraft:giant_spruce_taiga_hills",
        162 => "minecraft:modified_gravelly_mountains",
        163 => "minecraft:windswept_savanna",
        164 => "minecraft:shattered_savanna_plateau",
        165 => "minecraft:eroded_badlands",
        166 => "minecraft:modified_wooded_badlands_plateau",
        167 => "minecraft:modified_badlands_plateau",
        168 => "minecraft:bamboo_jungle",
        169 => "minecraft:bamboo_jungle_hills",
        170 => "minecraft:soul_sand_valley",
        171 => "minecraft:crimson_forest",
        172 => "minecraft:warped_forest",
        173 => "minecraft:basalt_deltas",
        174 => "minecraft:dripstone_caves",
        175 => "minecraft:lush_caves",
        _ => "minecraft:plains",
    }
}

fn legacy_color_name(data: u8) -> usize {
    (data & 0x0F) as usize
}

const LEGACY_WOOLS: [&str; 16] = [
    "minecraft:white_wool", "minecraft:orange_wool", "minecraft:magenta_wool", "minecraft:light_blue_wool",
    "minecraft:yellow_wool", "minecraft:lime_wool", "minecraft:pink_wool", "minecraft:gray_wool",
    "minecraft:light_gray_wool", "minecraft:cyan_wool", "minecraft:purple_wool", "minecraft:blue_wool",
    "minecraft:brown_wool", "minecraft:green_wool", "minecraft:red_wool", "minecraft:black_wool",
];
const LEGACY_STAINED_GLASS: [&str; 16] = [
    "minecraft:white_stained_glass", "minecraft:orange_stained_glass", "minecraft:magenta_stained_glass", "minecraft:light_blue_stained_glass",
    "minecraft:yellow_stained_glass", "minecraft:lime_stained_glass", "minecraft:pink_stained_glass", "minecraft:gray_stained_glass",
    "minecraft:light_gray_stained_glass", "minecraft:cyan_stained_glass", "minecraft:purple_stained_glass", "minecraft:blue_stained_glass",
    "minecraft:brown_stained_glass", "minecraft:green_stained_glass", "minecraft:red_stained_glass", "minecraft:black_stained_glass",
];
const LEGACY_STAINED_GLASS_PANE: [&str; 16] = [
    "minecraft:white_stained_glass_pane", "minecraft:orange_stained_glass_pane", "minecraft:magenta_stained_glass_pane", "minecraft:light_blue_stained_glass_pane",
    "minecraft:yellow_stained_glass_pane", "minecraft:lime_stained_glass_pane", "minecraft:pink_stained_glass_pane", "minecraft:gray_stained_glass_pane",
    "minecraft:light_gray_stained_glass_pane", "minecraft:cyan_stained_glass_pane", "minecraft:purple_stained_glass_pane", "minecraft:blue_stained_glass_pane",
    "minecraft:brown_stained_glass_pane", "minecraft:green_stained_glass_pane", "minecraft:red_stained_glass_pane", "minecraft:black_stained_glass_pane",
];
const LEGACY_TERRACOTTA: [&str; 16] = [
    "minecraft:white_terracotta", "minecraft:orange_terracotta", "minecraft:magenta_terracotta", "minecraft:light_blue_terracotta",
    "minecraft:yellow_terracotta", "minecraft:lime_terracotta", "minecraft:pink_terracotta", "minecraft:gray_terracotta",
    "minecraft:light_gray_terracotta", "minecraft:cyan_terracotta", "minecraft:purple_terracotta", "minecraft:blue_terracotta",
    "minecraft:brown_terracotta", "minecraft:green_terracotta", "minecraft:red_terracotta", "minecraft:black_terracotta",
];
const LEGACY_CARPETS: [&str; 16] = [
    "minecraft:white_carpet", "minecraft:orange_carpet", "minecraft:magenta_carpet", "minecraft:light_blue_carpet",
    "minecraft:yellow_carpet", "minecraft:lime_carpet", "minecraft:pink_carpet", "minecraft:gray_carpet",
    "minecraft:light_gray_carpet", "minecraft:cyan_carpet", "minecraft:purple_carpet", "minecraft:blue_carpet",
    "minecraft:brown_carpet", "minecraft:green_carpet", "minecraft:red_carpet", "minecraft:black_carpet",
];
const LEGACY_CONCRETE: [&str; 16] = [
    "minecraft:white_concrete", "minecraft:orange_concrete", "minecraft:magenta_concrete", "minecraft:light_blue_concrete",
    "minecraft:yellow_concrete", "minecraft:lime_concrete", "minecraft:pink_concrete", "minecraft:gray_concrete",
    "minecraft:light_gray_concrete", "minecraft:cyan_concrete", "minecraft:purple_concrete", "minecraft:blue_concrete",
    "minecraft:brown_concrete", "minecraft:green_concrete", "minecraft:red_concrete", "minecraft:black_concrete",
];
const LEGACY_CONCRETE_POWDER: [&str; 16] = [
    "minecraft:white_concrete_powder", "minecraft:orange_concrete_powder", "minecraft:magenta_concrete_powder", "minecraft:light_blue_concrete_powder",
    "minecraft:yellow_concrete_powder", "minecraft:lime_concrete_powder", "minecraft:pink_concrete_powder", "minecraft:gray_concrete_powder",
    "minecraft:light_gray_concrete_powder", "minecraft:cyan_concrete_powder", "minecraft:purple_concrete_powder", "minecraft:blue_concrete_powder",
    "minecraft:brown_concrete_powder", "minecraft:green_concrete_powder", "minecraft:red_concrete_powder", "minecraft:black_concrete_powder",
];
const LEGACY_SHULKER: [&str; 16] = [
    "minecraft:white_shulker_box", "minecraft:orange_shulker_box", "minecraft:magenta_shulker_box", "minecraft:light_blue_shulker_box",
    "minecraft:yellow_shulker_box", "minecraft:lime_shulker_box", "minecraft:pink_shulker_box", "minecraft:gray_shulker_box",
    "minecraft:light_gray_shulker_box", "minecraft:cyan_shulker_box", "minecraft:purple_shulker_box", "minecraft:blue_shulker_box",
    "minecraft:brown_shulker_box", "minecraft:green_shulker_box", "minecraft:red_shulker_box", "minecraft:black_shulker_box",
];
const LEGACY_GLAZED: [&str; 16] = [
    "minecraft:white_glazed_terracotta", "minecraft:orange_glazed_terracotta", "minecraft:magenta_glazed_terracotta", "minecraft:light_blue_glazed_terracotta",
    "minecraft:yellow_glazed_terracotta", "minecraft:lime_glazed_terracotta", "minecraft:pink_glazed_terracotta", "minecraft:gray_glazed_terracotta",
    "minecraft:light_gray_glazed_terracotta", "minecraft:cyan_glazed_terracotta", "minecraft:purple_glazed_terracotta", "minecraft:blue_glazed_terracotta",
    "minecraft:brown_glazed_terracotta", "minecraft:green_glazed_terracotta", "minecraft:red_glazed_terracotta", "minecraft:black_glazed_terracotta",
];

fn legacy_stair_props(data: u8, props: &mut BTreeMap<String, String>) {
    let facing = match data & 3 {
        0 => "east",
        1 => "west",
        2 => "south",
        _ => "north",
    };
    props.insert("facing".to_string(), facing.to_string());
    props.insert(
        "half".to_string(),
        if (data & 4) != 0 { "top" } else { "bottom" }.to_string(),
    );
    props.insert("shape".to_string(), "straight".to_string());
}

fn legacy_log_axis(data: u8, props: &mut BTreeMap<String, String>) {
    let axis = match (data >> 2) & 3 {
        1 => "x",
        2 => "z",
        _ => "y",
    };
    props.insert("axis".to_string(), axis.to_string());
}

fn legacy_slab_type(is_double: bool, data: u8, props: &mut BTreeMap<String, String>) {
    let t = if is_double {
        "double"
    } else if (data & 8) != 0 {
        "top"
    } else {
        "bottom"
    };
    props.insert("type".to_string(), t.to_string());
}

fn legacy_door_props(data: u8, props: &mut BTreeMap<String, String>) {
    if (data & 8) != 0 {
        props.insert("half".to_string(), "upper".to_string());
        props.insert(
            "hinge".to_string(),
            if (data & 1) != 0 { "right" } else { "left" }.to_string(),
        );
    } else {
        props.insert("half".to_string(), "lower".to_string());
        let facing = match data & 3 {
            0 => "east",
            1 => "south",
            2 => "west",
            _ => "north",
        };
        props.insert("facing".to_string(), facing.to_string());
        props.insert(
            "open".to_string(),
            if (data & 4) != 0 { "true" } else { "false" }.to_string(),
        );
    }
}

fn legacy_block_name(id: u8, data: u8) -> (&'static str, BTreeMap<String, String>) {
    let mut props = BTreeMap::new();
    let name = match id {
        0 => "minecraft:air",
        1 => match data & 7 {
            1 => "minecraft:granite",
            2 => "minecraft:polished_granite",
            3 => "minecraft:diorite",
            4 => "minecraft:polished_diorite",
            5 => "minecraft:andesite",
            6 => "minecraft:polished_andesite",
            _ => "minecraft:stone",
        },
        2 => "minecraft:grass_block",
        3 => match data & 3 {
            1 => "minecraft:coarse_dirt",
            2 => "minecraft:podzol",
            _ => "minecraft:dirt",
        },
        4 => "minecraft:cobblestone",
        5 => match data & 7 {
            1 => "minecraft:spruce_planks",
            2 => "minecraft:birch_planks",
            3 => "minecraft:jungle_planks",
            4 => "minecraft:acacia_planks",
            5 => "minecraft:dark_oak_planks",
            _ => "minecraft:oak_planks",
        },
        6 => match data & 7 {
            1 => "minecraft:spruce_sapling",
            2 => "minecraft:birch_sapling",
            3 => "minecraft:jungle_sapling",
            4 => "minecraft:acacia_sapling",
            5 => "minecraft:dark_oak_sapling",
            _ => "minecraft:oak_sapling",
        },
        7 => "minecraft:bedrock",
        8 => {
            props.insert("level".to_string(), (data & 0x0F).to_string());
            "minecraft:flowing_water"
        }
        9 => {
            props.insert("level".to_string(), (data & 0x0F).to_string());
            "minecraft:water"
        }
        10 => {
            props.insert("level".to_string(), (data & 0x0F).to_string());
            "minecraft:flowing_lava"
        }
        11 => {
            props.insert("level".to_string(), (data & 0x0F).to_string());
            "minecraft:lava"
        }
        12 => if (data & 1) == 1 { "minecraft:red_sand" } else { "minecraft:sand" },
        13 => "minecraft:gravel",
        14 => "minecraft:gold_ore",
        15 => "minecraft:iron_ore",
        16 => "minecraft:coal_ore",
        17 => {
            legacy_log_axis(data, &mut props);
            match data & 3 {
                1 => "minecraft:spruce_log",
                2 => "minecraft:birch_log",
                3 => "minecraft:jungle_log",
                _ => "minecraft:oak_log",
            }
        }
        18 => match data & 3 {
            1 => "minecraft:spruce_leaves",
            2 => "minecraft:birch_leaves",
            3 => "minecraft:jungle_leaves",
            _ => "minecraft:oak_leaves",
        },
        19 => if (data & 1) == 1 { "minecraft:wet_sponge" } else { "minecraft:sponge" },
        20 => "minecraft:glass",
        21 => "minecraft:lapis_ore",
        22 => "minecraft:lapis_block",
        23 => "minecraft:dispenser",
        24 => match data & 3 {
            1 => "minecraft:chiseled_sandstone",
            2 => "minecraft:cut_sandstone",
            _ => "minecraft:sandstone",
        },
        25 => "minecraft:note_block",
        26 => "minecraft:red_bed",
        27 => "minecraft:powered_rail",
        28 => "minecraft:detector_rail",
        29 => "minecraft:sticky_piston",
        30 => "minecraft:cobweb",
        31 => match data & 3 {
            0 => "minecraft:dead_bush",
            2 => "minecraft:fern",
            _ => "minecraft:short_grass",
        },
        32 => "minecraft:dead_bush",
        33 => "minecraft:piston",
        34 => "minecraft:piston_head",
        35 => LEGACY_WOOLS[legacy_color_name(data)],
        37 => "minecraft:dandelion",
        38 => match data & 0x0F {
            1 => "minecraft:blue_orchid",
            2 => "minecraft:allium",
            3 => "minecraft:azure_bluet",
            4 => "minecraft:red_tulip",
            5 => "minecraft:orange_tulip",
            6 => "minecraft:white_tulip",
            7 => "minecraft:pink_tulip",
            8 => "minecraft:oxeye_daisy",
            _ => "minecraft:poppy",
        },
        39 => "minecraft:brown_mushroom",
        40 => "minecraft:red_mushroom",
        41 => "minecraft:gold_block",
        42 => "minecraft:iron_block",
        43 | 44 => {
            legacy_slab_type(id == 43, data, &mut props);
            match data & 7 {
                1 => "minecraft:sandstone_slab",
                2 | 3 => "minecraft:cobblestone_slab",
                4 => "minecraft:brick_slab",
                5 => "minecraft:stone_brick_slab",
                6 => "minecraft:nether_brick_slab",
                7 => "minecraft:quartz_slab",
                _ => "minecraft:stone_slab",
            }
        }
        45 => "minecraft:bricks",
        46 => "minecraft:tnt",
        47 => "minecraft:bookshelf",
        48 => "minecraft:mossy_cobblestone",
        49 => "minecraft:obsidian",
        50 => "minecraft:torch",
        51 => "minecraft:fire",
        52 => "minecraft:spawner",
        53 => {
            legacy_stair_props(data, &mut props);
            "minecraft:oak_stairs"
        }
        54 => "minecraft:chest",
        55 => "minecraft:redstone_wire",
        56 => "minecraft:diamond_ore",
        57 => "minecraft:diamond_block",
        58 => "minecraft:crafting_table",
        59 => "minecraft:wheat",
        60 => "minecraft:farmland",
        61 | 62 => "minecraft:furnace",
        63 => {
            props.insert("rotation".to_string(), (data & 0x0F).to_string());
            "minecraft:oak_sign"
        }
        64 => {
            legacy_door_props(data, &mut props);
            "minecraft:oak_door"
        }
        65 => "minecraft:ladder",
        66 => "minecraft:rail",
        67 => {
            legacy_stair_props(data, &mut props);
            "minecraft:cobblestone_stairs"
        }
        68 => "minecraft:oak_wall_sign",
        69 => "minecraft:lever",
        70 => "minecraft:stone_pressure_plate",
        71 => {
            legacy_door_props(data, &mut props);
            "minecraft:iron_door"
        }
        72 => "minecraft:oak_pressure_plate",
        73 | 74 => "minecraft:redstone_ore",
        75 | 76 => "minecraft:redstone_torch",
        77 => "minecraft:stone_button",
        78 => {
            props.insert("layers".to_string(), ((data & 7) + 1).to_string());
            "minecraft:snow"
        }
        79 => "minecraft:ice",
        80 => "minecraft:snow_block",
        81 => "minecraft:cactus",
        82 => "minecraft:clay",
        83 => "minecraft:sugar_cane",
        84 => "minecraft:jukebox",
        85 => "minecraft:oak_fence",
        86 => "minecraft:carved_pumpkin",
        87 => "minecraft:netherrack",
        88 => "minecraft:soul_sand",
        89 => "minecraft:glowstone",
        90 => "minecraft:nether_portal",
        91 => "minecraft:jack_o_lantern",
        92 => "minecraft:cake",
        93 | 94 => "minecraft:repeater",
        95 => LEGACY_STAINED_GLASS[legacy_color_name(data)],
        96 => "minecraft:oak_trapdoor",
        97 => match data & 7 {
            1 => "minecraft:infested_cobblestone",
            2 => "minecraft:infested_stone_bricks",
            3 => "minecraft:infested_mossy_stone_bricks",
            4 => "minecraft:infested_cracked_stone_bricks",
            5 => "minecraft:infested_chiseled_stone_bricks",
            _ => "minecraft:infested_stone",
        },
        98 => match data & 3 {
            1 => "minecraft:mossy_stone_bricks",
            2 => "minecraft:cracked_stone_bricks",
            3 => "minecraft:chiseled_stone_bricks",
            _ => "minecraft:stone_bricks",
        },
        99 => "minecraft:brown_mushroom_block",
        100 => "minecraft:red_mushroom_block",
        101 => "minecraft:iron_bars",
        102 => "minecraft:glass_pane",
        103 => "minecraft:melon",
        104 => "minecraft:pumpkin_stem",
        105 => "minecraft:melon_stem",
        106 => "minecraft:vine",
        107 => "minecraft:oak_fence_gate",
        108 => {
            legacy_stair_props(data, &mut props);
            "minecraft:brick_stairs"
        }
        109 => {
            legacy_stair_props(data, &mut props);
            "minecraft:stone_brick_stairs"
        }
        110 => "minecraft:mycelium",
        111 => "minecraft:lily_pad",
        112 => "minecraft:nether_bricks",
        113 => "minecraft:nether_brick_fence",
        114 => {
            legacy_stair_props(data, &mut props);
            "minecraft:nether_brick_stairs"
        }
        115 => "minecraft:nether_wart",
        116 => "minecraft:enchanting_table",
        117 => "minecraft:brewing_stand",
        118 => "minecraft:cauldron",
        119 => "minecraft:end_portal",
        120 => "minecraft:end_portal_frame",
        121 => "minecraft:end_stone",
        122 => "minecraft:dragon_egg",
        123 | 124 => "minecraft:redstone_lamp",
        125 | 126 => {
            legacy_slab_type(id == 125, data, &mut props);
            match data & 7 {
                1 => "minecraft:spruce_slab",
                2 => "minecraft:birch_slab",
                3 => "minecraft:jungle_slab",
                4 => "minecraft:acacia_slab",
                5 => "minecraft:dark_oak_slab",
                _ => "minecraft:oak_slab",
            }
        }
        127 => "minecraft:cocoa",
        128 => {
            legacy_stair_props(data, &mut props);
            "minecraft:sandstone_stairs"
        }
        129 => "minecraft:emerald_ore",
        130 => "minecraft:ender_chest",
        131 => "minecraft:tripwire_hook",
        132 => "minecraft:tripwire",
        133 => "minecraft:emerald_block",
        134 => {
            legacy_stair_props(data, &mut props);
            "minecraft:spruce_stairs"
        }
        135 => {
            legacy_stair_props(data, &mut props);
            "minecraft:birch_stairs"
        }
        136 => {
            legacy_stair_props(data, &mut props);
            "minecraft:jungle_stairs"
        }
        137 => "minecraft:command_block",
        138 => "minecraft:beacon",
        139 => if (data & 1) == 1 { "minecraft:mossy_cobblestone_wall" } else { "minecraft:cobblestone_wall" },
        140 => "minecraft:flower_pot",
        141 => "minecraft:carrots",
        142 => "minecraft:potatoes",
        143 => "minecraft:oak_button",
        144 => "minecraft:skeleton_skull",
        145 => "minecraft:anvil",
        146 => "minecraft:trapped_chest",
        147 => "minecraft:light_weighted_pressure_plate",
        148 => "minecraft:heavy_weighted_pressure_plate",
        149 | 150 => "minecraft:comparator",
        151 => "minecraft:daylight_detector",
        152 => "minecraft:redstone_block",
        153 => "minecraft:nether_quartz_ore",
        154 => "minecraft:hopper",
        155 => match data & 7 {
            1 => "minecraft:chiseled_quartz_block",
            2..=4 => "minecraft:quartz_pillar",
            _ => "minecraft:quartz_block",
        },
        156 => {
            legacy_stair_props(data, &mut props);
            "minecraft:quartz_stairs"
        }
        157 => "minecraft:activator_rail",
        158 => "minecraft:dropper",
        159 => LEGACY_TERRACOTTA[legacy_color_name(data)],
        160 => LEGACY_STAINED_GLASS_PANE[legacy_color_name(data)],
        161 => if (data & 1) == 1 { "minecraft:dark_oak_leaves" } else { "minecraft:acacia_leaves" },
        162 => {
            legacy_log_axis(data, &mut props);
            if (data & 1) == 1 { "minecraft:dark_oak_log" } else { "minecraft:acacia_log" }
        }
        163 => {
            legacy_stair_props(data, &mut props);
            "minecraft:acacia_stairs"
        }
        164 => {
            legacy_stair_props(data, &mut props);
            "minecraft:dark_oak_stairs"
        }
        165 => "minecraft:slime_block",
        166 => "minecraft:barrier",
        167 => "minecraft:iron_trapdoor",
        168 => match data & 3 {
            1 => "minecraft:prismarine_bricks",
            2 => "minecraft:dark_prismarine",
            _ => "minecraft:prismarine",
        },
        169 => "minecraft:sea_lantern",
        170 => "minecraft:hay_block",
        171 => LEGACY_CARPETS[legacy_color_name(data)],
        172 => "minecraft:terracotta",
        173 => "minecraft:coal_block",
        174 => "minecraft:packed_ice",
        175 => {
            props.insert(
                "half".to_string(),
                if (data & 8) != 0 { "upper" } else { "lower" }.to_string(),
            );
            match data & 7 {
                1 => "minecraft:lilac",
                2 => "minecraft:tall_grass",
                3 => "minecraft:large_fern",
                4 => "minecraft:rose_bush",
                5 => "minecraft:peony",
                _ => "minecraft:sunflower",
            }
        }
        176 => "minecraft:white_banner",
        177 => "minecraft:white_wall_banner",
        178 => "minecraft:daylight_detector",
        179 => match data & 3 {
            1 => "minecraft:chiseled_red_sandstone",
            2 => "minecraft:cut_red_sandstone",
            _ => "minecraft:red_sandstone",
        },
        180 => {
            legacy_stair_props(data, &mut props);
            "minecraft:red_sandstone_stairs"
        }
        181 | 182 => {
            legacy_slab_type(id == 181, data, &mut props);
            "minecraft:red_sandstone_slab"
        }
        183 => "minecraft:spruce_fence_gate",
        184 => "minecraft:birch_fence_gate",
        185 => "minecraft:jungle_fence_gate",
        186 => "minecraft:dark_oak_fence_gate",
        187 => "minecraft:acacia_fence_gate",
        188 => "minecraft:spruce_fence",
        189 => "minecraft:birch_fence",
        190 => "minecraft:jungle_fence",
        191 => "minecraft:dark_oak_fence",
        192 => "minecraft:acacia_fence",
        193 => { legacy_door_props(data, &mut props); "minecraft:spruce_door" }
        194 => { legacy_door_props(data, &mut props); "minecraft:birch_door" }
        195 => { legacy_door_props(data, &mut props); "minecraft:jungle_door" }
        196 => { legacy_door_props(data, &mut props); "minecraft:acacia_door" }
        197 => { legacy_door_props(data, &mut props); "minecraft:dark_oak_door" }
        198 => "minecraft:end_rod",
        199 => "minecraft:chorus_plant",
        200 => "minecraft:chorus_flower",
        201 => "minecraft:purpur_block",
        202 => {
            legacy_log_axis(data, &mut props);
            "minecraft:purpur_pillar"
        }
        203 => {
            legacy_stair_props(data, &mut props);
            "minecraft:purpur_stairs"
        }
        204 | 205 => {
            legacy_slab_type(id == 204, data, &mut props);
            "minecraft:purpur_slab"
        }
        206 => "minecraft:end_stone_bricks",
        207 => "minecraft:beetroots",
        208 => "minecraft:dirt_path",
        209 => "minecraft:end_gateway",
        210 => "minecraft:repeating_command_block",
        211 => "minecraft:chain_command_block",
        212 => "minecraft:frosted_ice",
        213 => "minecraft:magma_block",
        214 => "minecraft:nether_wart_block",
        215 => "minecraft:red_nether_bricks",
        216 => "minecraft:bone_block",
        217 => "minecraft:structure_void",
        218 => "minecraft:observer",
        219..=234 => LEGACY_SHULKER[(id - 219) as usize],
        235..=250 => LEGACY_GLAZED[(id - 235) as usize],
        251 => LEGACY_CONCRETE[legacy_color_name(data)],
        252 => LEGACY_CONCRETE_POWDER[legacy_color_name(data)],
        255 => "minecraft:structure_block",
        _ => "minecraft:stone",
    };
    (name, props)
}

fn strip_mc_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '§' {
            let _ = chars.next();
            continue;
        }
        out.push(ch);
    }
    out
}

fn extract_json_text_component(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with('"') || trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            return strip_mc_formatting(&flatten_json_component(&val));
        }
    }
    strip_mc_formatting(trimmed)
}

fn flatten_json_component(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr.iter().map(flatten_json_component).collect(),
        serde_json::Value::Object(obj) => {
            let mut s = obj
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(extra) = obj.get("extra").and_then(|e| e.as_array()) {
                for item in extra {
                    s.push_str(&flatten_json_component(item));
                }
            }
            s
        }
        _ => String::new(),
    }
}

fn value_to_nbt_text(v: &Value) -> String {
    match v {
        Value::String(s) => extract_json_text_component(s),
        Value::List(arr) => {
            let joined: String = arr.iter().map(value_to_nbt_text).collect();
            strip_mc_formatting(&joined)
        }
        Value::Compound(map) => {
            let mut s = match map.get("text") {
                Some(Value::String(st)) => st.clone(),
                Some(other) => value_to_nbt_text(other),
                None => String::new(),
            };
            if let Some(Value::List(extra)) = map.get("extra") {
                for item in extra {
                    s.push_str(&value_to_nbt_text(item));
                }
            }
            strip_mc_formatting(&s)
        }
        _ => String::new(),
    }
}

fn parse_signs_from_block_entities(entities: &[BlockEntityNbt]) -> Vec<SignData> {
    let mut signs = Vec::new();
    for ent in entities {
        let Some(ref id_raw) = ent.id else { continue };
        let id = id_raw.to_lowercase();
        if !id.contains("sign") {
            continue;
        }
        let (Some(x), Some(y), Some(z)) = (ent.x, ent.y, ent.z) else {
            continue;
        };

        let mut front_lines = [String::new(), String::new(), String::new(), String::new()];
        let mut back_lines = [String::new(), String::new(), String::new(), String::new()];
        let mut front_color = "black".to_string();
        let mut back_color = "black".to_string();

        if let Some(ref ft) = ent.front_text {
            if let Some(ref c) = ft.color {
                front_color = c.clone();
            }
            for (i, msg) in ft.messages.iter().take(4).enumerate() {
                front_lines[i] = value_to_nbt_text(msg);
            }
        } else {
            // 1.19 and older
            if let Some(ref c) = ent.legacy_color {
                front_color = c.clone();
            }
            for (i, v_opt) in [&ent.text1, &ent.text2, &ent.text3, &ent.text4].into_iter().enumerate() {
                if let Some(v) = v_opt {
                    front_lines[i] = value_to_nbt_text(v);
                }
            }
        }

        if let Some(ref bt) = ent.back_text {
            if let Some(ref c) = bt.color {
                back_color = c.clone();
            }
            for (i, msg) in bt.messages.iter().take(4).enumerate() {
                back_lines[i] = value_to_nbt_text(msg);
            }
        }

        signs.push(SignData {
            x,
            y,
            z,
            front_lines,
            back_lines,
            front_color,
            back_color,
        });
    }
    signs
}

fn extract_legacy_section_biomes(
    biomes_val: Option<&Value>,
    sec_y: i32,
    intern_biome: &mut dyn FnMut(&str) -> u8,
) -> Vec<u8> {
    let mut out = vec![0u8; 64];
    match biomes_val {
        Some(Value::IntArray(arr)) if arr.len() >= 1024 => {
            let max_by = (arr.len() / 16).saturating_sub(1) as i32;
            for by in 0..4usize {
                let global_by = (sec_y * 4 + by as i32).clamp(0, max_by) as usize;
                for bz in 0..4usize {
                    for bx in 0..4usize {
                        let idx_3d = global_by * 16 + bz * 4 + bx;
                        if let Some(&bid) = arr.get(idx_3d) {
                            let bname = legacy_biome_id_to_name(bid);
                            out[(by * 4 + bz) * 4 + bx] = intern_biome(bname);
                        }
                    }
                }
            }
        }
        Some(Value::IntArray(arr)) if arr.len() >= 256 => {
            for bz in 0..4usize {
                for bx in 0..4usize {
                    let lx = bx * 4 + 2;
                    let lz = bz * 4 + 2;
                    if let Some(&bid) = arr.get(lz * 16 + lx) {
                        let bname = legacy_biome_id_to_name(bid);
                        let b_idx = intern_biome(bname);
                        for by in 0..4usize {
                            out[(by * 4 + bz) * 4 + bx] = b_idx;
                        }
                    }
                }
            }
        }
        Some(Value::ByteArray(arr)) if arr.len() >= 256 => {
            for bz in 0..4usize {
                for bx in 0..4usize {
                    let lx = bx * 4 + 2;
                    let lz = bz * 4 + 2;
                    if let Some(&bid) = arr.get(lz * 16 + lx) {
                        let bname = legacy_biome_id_to_name((bid as u8) as i32);
                        let b_idx = intern_biome(bname);
                        for by in 0..4usize {
                            out[(by * 4 + bz) * 4 + bx] = b_idx;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn parse_chunk_bytes(raw_nbt: &[u8], coords: WorldCoords) -> Option<CompactChunk> {
    let chunk: ChunkNbt = fastnbt::from_bytes(raw_nbt).ok()?;
    let mut local_palette: Vec<BlockState> = Vec::new();
    let mut local_map: HashMap<String, u16> = HashMap::new();
    let mut intern_local = |bs: BlockState| -> u16 {
        if let Some(&idx) = local_map.get(&bs.canonical_key) {
            return idx;
        }
        if local_palette.len() >= u16::MAX as usize {
            return 0;
        }
        let idx = local_palette.len() as u16;
        local_map.insert(bs.canonical_key.clone(), idx);
        local_palette.push(bs);
        idx
    };

    let mut local_biome_palette: Vec<String> = vec!["minecraft:plains".to_string()];
    let mut biome_map: HashMap<String, u8> = HashMap::new();
    biome_map.insert("minecraft:plains".to_string(), 0);
    let mut intern_biome = |name: &str| -> u8 {
        if let Some(&idx) = biome_map.get(name) {
            return idx;
        }
        if local_biome_palette.len() >= 255 {
            return 0;
        }
        let idx = local_biome_palette.len() as u8;
        biome_map.insert(name.to_string(), idx);
        local_biome_palette.push(name.to_string());
        idx
    };

    let mut sections = Vec::new();
    let (chunk_x, chunk_z, signs) = if !chunk.sections.is_empty() {
        let signs = parse_signs_from_block_entities(&chunk.block_entities);
        let spanning = chunk.data_version > 0 && chunk.data_version < 2529;
        for sec in &chunk.sections {
            let sec_y = sec.y as i32;
            let sy_min = sec_y * 16;
            let sy_max = sy_min + 15;
            if sy_max < coords.min_y || sy_min > coords.max_y {
                continue;
            }
            let Some(bs) = &sec.block_states else {
                continue;
            };
            if bs.palette.is_empty() {
                continue;
            }
            let sec_remap: Vec<u16> = bs
                .palette
                .iter()
                .map(|p| intern_local(palette_value_to_block_state(p)))
                .collect();

            let block_indices = if let Some(data) = &bs.data {
                decode_packed_and_remap_u16(data, &sec_remap, 4096, 4, spanning)
            } else {
                vec![sec_remap[0]; 4096]
            };

            let biome_indices = if let Some(bi) = &sec.biomes {
                if bi.palette.is_empty() {
                    vec![0u8; 64]
                } else {
                    let b_remap: Vec<u8> = bi
                        .palette
                        .iter()
                        .map(|b| intern_biome(biome_value_to_name(b)))
                        .collect();
                    if let Some(bdata) = &bi.data {
                        let bits_needed = if b_remap.len() <= 1 {
                            1
                        } else {
                            (usize::BITS - (b_remap.len() - 1).leading_zeros()) as usize
                        };
                        decode_packed_and_remap_u8(bdata, &b_remap, 64, bits_needed.max(1))
                    } else {
                        vec![b_remap[0]; 64]
                    }
                }
            } else {
                vec![0u8; 64]
            };

            sections.push(CompactSection {
                sec_y,
                block_indices,
                biome_indices,
            });
        }
        (chunk.x_pos, chunk.z_pos, signs)
    } else if let Some(lvl) = &chunk.level {
        let signs = parse_signs_from_block_entities(&lvl.tile_entities);
        let spanning = chunk.data_version == 0 || chunk.data_version < 2529;
        for sec in &lvl.sections {
            let sec_y = sec.y as i32;
            let sy_min = sec_y * 16;
            let sy_max = sy_min + 15;
            if sy_max < coords.min_y || sy_min > coords.max_y {
                continue;
            }
            if !sec.palette.is_empty() {
                let sec_remap: Vec<u16> = sec
                    .palette
                    .iter()
                    .map(|p| intern_local(palette_entry_to_block_state(p)))
                    .collect();
                let block_indices = if let Some(data) = &sec.block_states {
                    let use_span = if spanning {
                        let bits = (usize::BITS - (sec_remap.len().max(2) - 1).leading_zeros())
                            .max(4) as usize;
                        let non_span_len = 4096usize.div_ceil(64 / bits);
                        data.len() != non_span_len
                    } else {
                        false
                    };
                    decode_packed_and_remap_u16(data, &sec_remap, 4096, 4, use_span)
                } else {
                    vec![sec_remap[0]; 4096]
                };
                let biome_indices =
                    extract_legacy_section_biomes(lvl.biomes.as_ref(), sec_y, &mut intern_biome);
                sections.push(CompactSection {
                    sec_y,
                    block_indices,
                    biome_indices,
                });
            } else if let Some(blocks) = &sec.blocks {
                let nibbles = sec.data.as_deref();
                let mut block_indices = vec![0u16; 4096];
                let mut legacy_cache = [u16::MAX; 4096];
                for i in 0..4096.min(blocks.len()) {
                    let bid = blocks[i] as u8;
                    let bdata = nibbles
                        .and_then(|arr| arr.get(i / 2))
                        .map(|&b| {
                            let ub = b as u8;
                            if i % 2 == 0 {
                                ub & 0x0F
                            } else {
                                (ub >> 4) & 0x0F
                            }
                        })
                        .unwrap_or(0);
                    let key = ((bid as usize) << 4) | (bdata as usize);
                    let cached = legacy_cache[key];
                    if cached != u16::MAX {
                        block_indices[i] = cached;
                    } else {
                        let (name, props) = legacy_block_name(bid, bdata);
                        let id = intern_local(BlockState::new(name.to_string(), props));
                        legacy_cache[key] = id;
                        block_indices[i] = id;
                    }
                }
                let biome_indices =
                    extract_legacy_section_biomes(lvl.biomes.as_ref(), sec_y, &mut intern_biome);
                sections.push(CompactSection {
                    sec_y,
                    block_indices,
                    biome_indices,
                });
            }
        }
        (lvl.x_pos, lvl.z_pos, signs)
    } else {
        return None;
    };

    Some(CompactChunk {
        chunk_x,
        chunk_z,
        local_palette,
        local_biome_palette,
        sections,
        signs,
    })
}

const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

fn read_bounded<R: Read>(reader: R, max_bytes: usize, hint_cap: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(hint_cap.min(4 * 1024 * 1024));
    let mut limited = reader.take((max_bytes as u64).saturating_add(1));
    limited.read_to_end(&mut out).ok()?;
    if out.len() > max_bytes {
        return None;
    }
    Some(out)
}

/// Decompresses Minecraft 1.20.5+ `LZ4BlockInputStream` payloads (`comp_type == 4`).
pub fn decompress_lz4_java_blocks(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos < input.len() {
        if pos + 21 > input.len() || &input[pos..pos + 8] != b"LZ4Block" {
            return None;
        }
        let token = input[pos + 8];
        let method = token & 0xF0;
        let comp_len = u32::from_le_bytes([
            input[pos + 9],
            input[pos + 10],
            input[pos + 11],
            input[pos + 12],
        ]) as usize;
        let decomp_len = u32::from_le_bytes([
            input[pos + 13],
            input[pos + 14],
            input[pos + 15],
            input[pos + 16],
        ]) as usize;
        pos += 21;

        if comp_len == 0 && decomp_len == 0 {
            break;
        }
        let end = pos.checked_add(comp_len)?;
        if end > input.len() || out.len().checked_add(decomp_len)? > MAX_DECOMPRESSED_BYTES {
            return None;
        }
        let block = &input[pos..end];
        pos = end;

        if method == 0x10 {
            if comp_len != decomp_len {
                return None;
            }
            out.extend_from_slice(block);
        } else if method == 0x20 {
            let start_out = out.len();
            out.reserve(decomp_len);
            let mut bpos = 0usize;
            while bpos < block.len() {
                let seq_tok = block[bpos];
                bpos += 1;

                let mut lit_len = (seq_tok >> 4) as usize;
                if lit_len == 15 {
                    loop {
                        let extra = *block.get(bpos)?;
                        bpos += 1;
                        lit_len += extra as usize;
                        if extra != 255 {
                            break;
                        }
                    }
                }
                let lit_end = bpos.checked_add(lit_len)?;
                if lit_end > block.len() {
                    return None;
                }
                out.extend_from_slice(&block[bpos..lit_end]);
                bpos = lit_end;

                if bpos >= block.len() {
                    break;
                }

                let off_lo = *block.get(bpos)? as usize;
                let off_hi = *block.get(bpos + 1)? as usize;
                bpos += 2;
                let offset = off_lo | (off_hi << 8);
                if offset == 0 || offset > out.len() - start_out {
                    return None;
                }

                let mut match_len = ((seq_tok & 0x0F) as usize) + 4;
                if (seq_tok & 0x0F) == 15 {
                    loop {
                        let extra = *block.get(bpos)?;
                        bpos += 1;
                        match_len += extra as usize;
                        if extra != 255 {
                            break;
                        }
                    }
                }
                if out.len() - start_out + match_len > decomp_len {
                    return None;
                }
                for _ in 0..match_len {
                    let b = out[out.len() - offset];
                    out.push(b);
                }
            }
            if out.len() - start_out != decomp_len {
                return None;
            }
        } else {
            return None;
        }
    }

    Some(out)
}

pub(crate) fn decompress_chunk_payload(comp_type: u8, comp_slice: &[u8]) -> Option<Vec<u8>> {
    match comp_type {
        1 => read_bounded(
            GzDecoder::new(comp_slice),
            MAX_DECOMPRESSED_BYTES,
            comp_slice.len().saturating_mul(4),
        ),
        2 => read_bounded(
            ZlibDecoder::new(comp_slice),
            MAX_DECOMPRESSED_BYTES,
            comp_slice.len().saturating_mul(4),
        ),
        3 => (comp_slice.len() <= MAX_DECOMPRESSED_BYTES).then(|| comp_slice.to_vec()),
        4 => decompress_lz4_java_blocks(comp_slice),
        _ => None,
    }
}

fn read_region_chunks(mca_path: &Path, coords: WorldCoords) -> Vec<CompactChunk> {
    let Ok(mut file) = std::fs::File::open(mca_path) else {
        return Vec::new();
    };
    let Ok(meta) = file.metadata() else {
        return Vec::new();
    };
    let file_len = meta.len() as usize;
    if file_len < 8192 {
        return Vec::new();
    }

    // Extract region coordinates from filename r.X.Z.mca
    let fname = mca_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parts: Vec<&str> = fname.split('.').collect();
    if parts.len() != 3 {
        return Vec::new();
    }
    let Ok(rx) = parts[1].parse::<i32>() else {
        return Vec::new();
    };
    let Ok(rz) = parts[2].parse::<i32>() else {
        return Vec::new();
    };

    let mut header = [0u8; 4096];
    if file.read_exact(&mut header).is_err() {
        return Vec::new();
    }

    let min_cx = coords.min_x.div_euclid(16);
    let max_cx = coords.max_x.div_euclid(16);
    let min_cz = coords.min_z.div_euclid(16);
    let max_cz = coords.max_z.div_euclid(16);

    let mut target_slots = Vec::new();
    for lcz in 0..32 {
        let gcz = rz * 32 + lcz;
        if gcz < min_cz || gcz > max_cz {
            continue;
        }
        for lcx in 0..32 {
            let gcx = rx * 32 + lcx;
            if gcx < min_cx || gcx > max_cx {
                continue;
            }
            let slot = (lcx + lcz * 32) as usize;
            let h_off = slot * 4;
            let offset_sectors = ((header[h_off] as usize) << 16)
                | ((header[h_off + 1] as usize) << 8)
                | (header[h_off + 2] as usize);
            let sector_count = header[h_off + 3] as usize;
            if offset_sectors < 2 || sector_count == 0 {
                continue;
            }
            let byte_off = offset_sectors * 4096;
            if byte_off + 5 > file_len {
                continue;
            }
            target_slots.push((byte_off, sector_count, gcx, gcz));
        }
    }

    if target_slots.is_empty() {
        return Vec::new();
    }
    target_slots.sort_unstable_by_key(|&(off, _, _, _)| off);

    let mut raw_payloads: Vec<(Vec<u8>, i32, i32)> = Vec::with_capacity(target_slots.len());
    for (byte_off, sector_count, gcx, gcz) in target_slots {
        let max_bytes = (sector_count * 4096).min(file_len - byte_off);
        if max_bytes < 5 {
            continue;
        }
        if file.seek(SeekFrom::Start(byte_off as u64)).is_err() {
            continue;
        }
        let mut hdr = [0u8; 5];
        if file.read_exact(&mut hdr).is_err() {
            continue;
        }
        let payload_len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let comp_type = hdr[4];
        if (comp_type & 0x80) != 0 {
            if payload_len >= 1 && 4 + payload_len <= max_bytes {
                raw_payloads.push((vec![comp_type], gcx, gcz));
            }
            continue;
        }
        if payload_len < 2 || 4 + payload_len > max_bytes {
            continue;
        }
        let mut buf = vec![0u8; payload_len];
        buf[0] = comp_type;
        if file.read_exact(&mut buf[1..]).is_err() {
            continue;
        }
        raw_payloads.push((buf, gcx, gcz));
    }

    let region_dir = mca_path.parent();
    raw_payloads
        .into_par_iter()
        .filter_map(|(buf, gcx, gcz)| {
            let comp_type = buf[0];
            let decompressed = if (comp_type & 0x80) != 0 {
                let mcc_path = region_dir?.join(format!("c.{}.{}.mcc", gcx, gcz));
                let ext_bytes = std::fs::read(&mcc_path).ok()?;
                decompress_chunk_payload(comp_type & 0x7F, &ext_bytes)?
            } else {
                decompress_chunk_payload(comp_type, &buf[1..])?
            };
            let mut cc = parse_chunk_bytes(&decompressed, coords)?;
            // Trust region slot coordinates if NBT xPos/zPos were omitted
            if cc.chunk_x == 0 && cc.chunk_z == 0 && (gcx != 0 || gcz != 0) {
                cc.chunk_x = gcx;
                cc.chunk_z = gcz;
            }
            Some(cc)
        })
        .collect()
}

pub fn resolve_dimension_region_dir(world_dir: &Path, dimension: &str) -> Result<PathBuf, String> {
    let fname = world_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if fname.eq_ignore_ascii_case("region") && world_dir.is_dir() {
        return Ok(world_dir.to_path_buf());
    }
    if (fname.eq_ignore_ascii_case("DIM-1") || fname.eq_ignore_ascii_case("DIM1"))
        && world_dir.join("region").is_dir()
    {
        return Ok(world_dir.join("region"));
    }

    let resolved_wp = std::fs::canonicalize(world_dir).unwrap_or_else(|_| world_dir.to_path_buf());
    let wp_parent = world_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| resolved_wp.parent());
    let wp_name = world_dir
        .file_name()
        .and_then(|s| s.to_str())
        .or_else(|| resolved_wp.file_name().and_then(|s| s.to_str()));

    let dim_l = dimension.trim().to_lowercase();
    if dim_l.is_empty()
        || dim_l == "overworld"
        || dim_l == "minecraft:overworld"
        || dim_l == "0"
        || dim_l == "dim0"
        || dim_l == "dim_overworld"
    {
        let candidates = [
            world_dir.join("region"),
            world_dir
                .join("dimensions")
                .join("minecraft")
                .join("overworld")
                .join("region"),
        ];
        for cand in candidates {
            if cand.is_dir() {
                return Ok(cand);
            }
        }
        return Err(format!(
            "Overworld region directory not found in {} (expected region/ or dimensions/minecraft/overworld/region)",
            world_dir.display()
        ));
    }

    if dim_l == "nether"
        || dim_l == "the_nether"
        || dim_l == "dim-1"
        || dim_l == "dim_nether"
        || dim_l == "minecraft:the_nether"
        || dim_l == "-1"
    {
        let mut candidates = vec![
            world_dir.join("DIM-1").join("region"),
            world_dir
                .join("dimensions")
                .join("minecraft")
                .join("the_nether")
                .join("region"),
        ];
        if let (Some(parent), Some(wname)) = (wp_parent, wp_name) {
            candidates.push(
                parent
                    .join(format!("{}_nether", wname))
                    .join("DIM-1")
                    .join("region"),
            );
            candidates.push(parent.join(format!("{}_nether", wname)).join("region"));
        }
        for cand in candidates {
            if cand.is_dir() {
                return Ok(cand);
            }
        }
        return Err(format!(
            "Nether region directory not found in {} (expected DIM-1/region or <world>_nether/DIM-1/region)",
            world_dir.display()
        ));
    }

    if dim_l == "end"
        || dim_l == "the_end"
        || dim_l == "dim1"
        || dim_l == "dim_end"
        || dim_l == "minecraft:the_end"
        || dim_l == "1"
    {
        let mut candidates = vec![
            world_dir.join("DIM1").join("region"),
            world_dir
                .join("dimensions")
                .join("minecraft")
                .join("the_end")
                .join("region"),
        ];
        if let (Some(parent), Some(wname)) = (wp_parent, wp_name) {
            candidates.push(
                parent
                    .join(format!("{}_the_end", wname))
                    .join("DIM1")
                    .join("region"),
            );
            candidates.push(parent.join(format!("{}_the_end", wname)).join("region"));
        }
        for cand in candidates {
            if cand.is_dir() {
                return Ok(cand);
            }
        }
        return Err(format!(
            "End region directory not found in {} (expected DIM1/region or <world>_the_end/DIM1/region)",
            world_dir.display()
        ));
    }

    if let Some((ns, path)) = dim_l.split_once(':') {
        let cand = world_dir
            .join("dimensions")
            .join(ns)
            .join(path)
            .join("region");
        if cand.is_dir() {
            return Ok(cand);
        }
    } else {
        let cand = world_dir
            .join("dimensions")
            .join("minecraft")
            .join(&dim_l)
            .join("region");
        if cand.is_dir() {
            return Ok(cand);
        }
        let direct = world_dir.join(&dim_l).join("region");
        if direct.is_dir() {
            return Ok(direct);
        }
    }

    Err(format!(
        "Region directory for dimension '{}' not found in {}",
        dimension,
        world_dir.display()
    ))
}

fn parse_nbt_file_auto(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_DECOMPRESSED_BYTES as u64 {
        return None;
    }
    let raw = std::fs::read(path).ok()?;
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let decompressed = read_bounded(GzDecoder::new(&raw[..]), MAX_DECOMPRESSED_BYTES, raw.len().saturating_mul(4))?;
        fastnbt::from_bytes(&decompressed).ok()
    } else if raw.len() >= 2 && raw[0] == 0x78 {
        if let Some(decompressed) =
            read_bounded(ZlibDecoder::new(&raw[..]), MAX_DECOMPRESSED_BYTES, raw.len().saturating_mul(4))
        {
            if let Ok(v) = fastnbt::from_bytes(&decompressed) {
                return Some(v);
            }
        }
        fastnbt::from_bytes(&raw).ok()
    } else {
        fastnbt::from_bytes(&raw).ok()
    }
}

fn extract_pos_from_compound(map: &HashMap<String, Value>) -> Option<[f64; 3]> {
    if let Some(Value::List(pos)) = map.get("Pos") {
        if pos.len() >= 3 {
            let x = value_as_f64(&pos[0])?;
            let y = value_as_f64(&pos[1])?;
            let z = value_as_f64(&pos[2])?;
            return Some([x, y, z]);
        }
    }
    None
}

#[derive(Debug, Clone, Default)]
pub struct WorldVersionInfo {
    pub name: Option<String>,
    pub data_version: Option<i32>,
}

pub fn read_level_version(world_dir: &Path) -> Option<WorldVersionInfo> {
    let mut roots: Vec<PathBuf> = vec![world_dir.to_path_buf()];
    if let Some(p1) = world_dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        roots.push(p1.to_path_buf());
        if let Some(p2) = p1.parent().filter(|p| !p.as_os_str().is_empty()) {
            roots.push(p2.to_path_buf());
        }
    }

    for cand_root in &roots {
        let level_dat = cand_root.join("level.dat");
        if !level_dat.is_file() {
            continue;
        }
        if let Some(Value::Compound(root_map)) = parse_nbt_file_auto(&level_dat) {
            if let Some(Value::Compound(data_map)) = root_map.get("Data") {
                let mut name = None;
                let mut version_id = None;
                if let Some(Value::Compound(ver_map)) = data_map.get("Version") {
                    if let Some(Value::String(vname)) = ver_map.get("Name") {
                        if !vname.trim().is_empty() {
                            name = Some(vname.trim().to_string());
                        }
                    }
                    version_id = ver_map.get("Id").and_then(value_as_f64).map(|v| v as i32);
                }
                let data_version = data_map
                    .get("DataVersion")
                    .and_then(value_as_f64)
                    .map(|v| v as i32)
                    .or(version_id);
                if name.is_some() || data_version.is_some() {
                    return Some(WorldVersionInfo { name, data_version });
                }
            }
        }
        break;
    }
    None
}

pub fn read_level_spawn(world_dir: &Path) -> Option<[f64; 3]> {
    let mut roots: Vec<PathBuf> = vec![world_dir.to_path_buf()];
    if let Some(p1) = world_dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        roots.push(p1.to_path_buf());
        if let Some(p2) = p1.parent().filter(|p| !p.as_os_str().is_empty()) {
            roots.push(p2.to_path_buf());
        }
    }

    let mut save_root = world_dir.to_path_buf();
    let mut world_spawn: Option<[f64; 3]> = None;

    for cand_root in &roots {
        let level_dat = cand_root.join("level.dat");
        if !level_dat.is_file() {
            continue;
        }
        save_root = cand_root.clone();
        if let Some(Value::Compound(root_map)) = parse_nbt_file_auto(&level_dat) {
            if let Some(Value::Compound(data_map)) = root_map.get("Data") {
                if let Some(Value::Compound(player)) = data_map.get("Player") {
                    if let Some(pos) = extract_pos_from_compound(player) {
                        return Some(pos);
                    }
                }
                if world_spawn.is_none() {
                    // Minecraft 1.21.9+ moved world spawn into Data.spawn.pos
                    if let Some(Value::Compound(spawn_map)) = data_map.get("spawn") {
                        match spawn_map.get("pos") {
                            Some(Value::IntArray(arr)) if arr.len() >= 3 => {
                                world_spawn = Some([arr[0] as f64, arr[1] as f64, arr[2] as f64]);
                            }
                            Some(Value::List(list)) if list.len() >= 3 => {
                                if let (Some(x), Some(y), Some(z)) = (
                                    value_as_f64(&list[0]),
                                    value_as_f64(&list[1]),
                                    value_as_f64(&list[2]),
                                ) {
                                    world_spawn = Some([x, y, z]);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if world_spawn.is_none() {
                    if let (Some(sx), Some(sy), Some(sz)) = (
                        data_map.get("SpawnX").and_then(value_as_f64),
                        data_map.get("SpawnY").and_then(value_as_f64),
                        data_map.get("SpawnZ").and_then(value_as_f64),
                    ) {
                        world_spawn = Some([sx, sy, sz]);
                    }
                }
            }
        }
        break;
    }

    // Fallback to newest players/data/*.dat (26.2+) or playerdata/*.dat (e.g. dedicated server saves)
    let mut pdata_dirs = vec![
        save_root.join("players").join("data"),
        save_root.join("playerdata"),
    ];
    if save_root != world_dir {
        pdata_dirs.push(world_dir.join("players").join("data"));
        pdata_dirs.push(world_dir.join("playerdata"));
    }
    for pdir in pdata_dirs {
        if !pdir.is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&pdir) {
            let mut dat_files: Vec<(std::time::SystemTime, PathBuf)> = rd
                .flatten()
                .filter_map(|ent| {
                    let p = ent.path();
                    let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    if p.is_file()
                        && !fname.starts_with("._")
                        && p.extension()
                            .and_then(|s| s.to_str())
                            .map(|s| s.eq_ignore_ascii_case("dat"))
                            .unwrap_or(false)
                    {
                        let mtime = ent
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        Some((mtime, p))
                    } else {
                        None
                    }
                })
                .collect();
            dat_files.sort_by_key(|a| std::cmp::Reverse(a.0));
            for (_, dat_path) in dat_files {
                if let Some(Value::Compound(root_map)) = parse_nbt_file_auto(&dat_path) {
                    if let Some(pos) = extract_pos_from_compound(&root_map) {
                        return Some(pos);
                    }
                }
            }
        }
    }

    world_spawn
}

pub fn check_and_clamp_bounds(coords: WorldCoords, scale_factor: f64) -> WorldCoords {
    let coords = coords.normalized();
    let max_source_units = 32000.0_f64;
    let max_blocks = 800_i32;
    let sf = scale_factor.max(1e-6);
    let effective_max_blocks = max_blocks.min((max_source_units / sf).floor().max(16.0) as i32);

    let span_x = coords.max_x.saturating_sub(coords.min_x);
    let span_z = coords.max_z.saturating_sub(coords.min_z);
    let mut new_min_x = coords.min_x;
    let mut new_max_x = coords.max_x;
    let mut new_min_z = coords.min_z;
    let mut new_max_z = coords.max_z;
    let mut clamped = false;

    if (span_x as f64) * sf > max_source_units || span_x > effective_max_blocks {
        clamped = true;
        let mid_x = ((coords.min_x as i64 + coords.max_x as i64) / 2) as i32;
        let half_w = effective_max_blocks / 2;
        new_min_x = mid_x - half_w;
        new_max_x = mid_x + half_w - 1;
    }

    if (span_z as f64) * sf > max_source_units || span_z > effective_max_blocks {
        clamped = true;
        let mid_z = ((coords.min_z as i64 + coords.max_z as i64) / 2) as i32;
        let half_d = effective_max_blocks / 2;
        new_min_z = mid_z - half_d;
        new_max_z = mid_z + half_d - 1;
    }

    if clamped {
        eprintln!(
            "[Anvil] WARNING: World bounds ({}x{} blocks) exceed Source Engine [-16384..16384] limits ({} units / {} blocks). Clamping to ({}, {}, {}) .. ({}, {}, {}).",
            span_x, span_z, max_source_units, effective_max_blocks,
            new_min_x, coords.min_y, new_min_z, new_max_x, coords.max_y, new_max_z
        );
    }

    WorldCoords {
        min_x: new_min_x,
        min_y: coords.min_y,
        min_z: new_min_z,
        max_x: new_max_x,
        max_y: coords.max_y,
        max_z: new_max_z,
    }
}

fn value_as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Double(d) => Some(*d),
        Value::Float(f) => Some(*f as f64),
        Value::Int(i) => Some(*i as f64),
        Value::Long(l) => Some(*l as f64),
        _ => None,
    }
}

fn scan_region_bounds(region_dir: &Path) -> (usize, Option<(i32, i32, i32, i32)>) {
    let Ok(rd) = std::fs::read_dir(region_dir) else {
        return (0, None);
    };
    let mut min_rx = i32::MAX;
    let mut max_rx = i32::MIN;
    let mut min_rz = i32::MAX;
    let mut max_rz = i32::MIN;
    let mut count = 0usize;
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        let Some(fname) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !fname.starts_with("r.") || !fname.ends_with(".mca") {
            continue;
        }
        let stem = &fname[2..fname.len() - 4];
        let Some((rx_s, rz_s)) = stem.split_once('.') else {
            continue;
        };
        if let (Ok(rx), Ok(rz)) = (rx_s.parse::<i32>(), rz_s.parse::<i32>()) {
            min_rx = min_rx.min(rx);
            max_rx = max_rx.max(rx);
            min_rz = min_rz.min(rz);
            max_rz = max_rz.max(rz);
            count += 1;
        }
    }
    if count > 0 {
        (
            count,
            Some((
                min_rx * 512,
                min_rz * 512,
                (max_rx + 1) * 512 - 1,
                (max_rz + 1) * 512 - 1,
            )),
        )
    } else {
        (0, None)
    }
}

pub fn load_world_region(
    world_dir: &Path,
    dimension: &str,
    coords: WorldCoords,
    scale_factor: f64,
) -> Result<VoxelGrid, String> {
    // Region bounds must respect the *effective* scale: a larger scale means fewer
    // blocks fit inside Source's +/-16384 unit limit, a smaller one allows more.
    let eff_scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        39.37
    };
    let coords = check_and_clamp_bounds(coords.normalized(), eff_scale);
    let sx = coords.size_x();
    let sy = coords.size_y();
    let sz = coords.size_z();
    let total_voxels = sx
        .checked_mul(sy)
        .and_then(|v| v.checked_mul(sz))
        .ok_or_else(|| "Coordinate volume overflow".to_string())?;
    if total_voxels == 0 {
        return Err("Empty coordinate volume".to_string());
    }

    let region_dir = resolve_dimension_region_dir(world_dir, dimension)?;
    if !region_dir.is_dir() {
        return Err(format!("Region directory not found: {}", region_dir.display()));
    }

    let min_rx = coords.min_x.div_euclid(512);
    let max_rx = coords.max_x.div_euclid(512);
    let min_rz = coords.min_z.div_euclid(512);
    let max_rz = coords.max_z.div_euclid(512);

    let mut mca_paths = Vec::new();
    for rz in min_rz..=max_rz {
        for rx in min_rx..=max_rx {
            let p = region_dir.join(format!("r.{}.{}.mca", rx, rz));
            if p.is_file() {
                mca_paths.push(p);
            }
        }
    }

    let air_state = BlockState::new("minecraft:air".to_string(), BTreeMap::new());
    let mut palette = vec![air_state.clone()];
    let mut palette_map: HashMap<String, u16> = HashMap::new();
    palette_map.insert(air_state.canonical_key.clone(), 0);

    let mut biome_palette = vec!["minecraft:plains".to_string()];
    let mut biome_map: HashMap<String, u16> = HashMap::new();
    biome_map.insert("minecraft:plains".to_string(), 0);

    let min_bx = coords.min_x.div_euclid(4);
    let max_bx = coords.max_x.div_euclid(4);
    let min_by = coords.min_y.div_euclid(4);
    let max_by = coords.max_y.div_euclid(4);
    let min_bz = coords.min_z.div_euclid(4);
    let max_bz = coords.max_z.div_euclid(4);
    let bsx = (max_bx - min_bx + 1).max(1) as usize;
    let bsy = (max_by - min_by + 1).max(1) as usize;
    let bsz = (max_bz - min_bz + 1).max(1) as usize;

    let mut blocks = vec![0u16; total_voxels];
    let mut biomes = vec![0u16; bsx * bsy * bsz];
    let mut signs = HashMap::new();
    let mut non_air_count = 0usize;

    struct RemappedChunk {
        chunk_x: i32,
        chunk_z: i32,
        global_block_remap: Vec<u16>,
        sections: Vec<CompactSection>,
    }

    for mca_path in &mca_paths {
        let parsed_chunks = read_region_chunks(mca_path, coords);
        if parsed_chunks.is_empty() {
            continue;
        }

        let mut remapped_chunks = Vec::with_capacity(parsed_chunks.len());
        for cc in parsed_chunks {
            let global_block_remap: Vec<u16> = cc
                .local_palette
                .into_iter()
                .map(|bs| {
                    if bs.is_air {
                        0u16
                    } else if let Some(&id) = palette_map.get(&bs.canonical_key) {
                        id
                    } else if palette.len() < (u16::MAX as usize) {
                        let id = palette.len() as u16;
                        palette_map.insert(bs.canonical_key.clone(), id);
                        palette.push(bs);
                        id
                    } else {
                        0u16
                    }
                })
                .collect();

            let global_biome_remap: Vec<u16> = cc
                .local_biome_palette
                .into_iter()
                .map(|b| {
                    if let Some(&id) = biome_map.get(&b) {
                        id
                    } else {
                        let id = biome_palette.len() as u16;
                        biome_map.insert(b.clone(), id);
                        biome_palette.push(b);
                        id
                    }
                })
                .collect();

            for s in cc.signs {
                if coords.contains(s.x, s.y, s.z) {
                    signs.insert((s.x, s.y, s.z), s);
                }
            }

            // Scatter 4x4x4 biomes into compact biome grid
            let chunk_bx0 = cc.chunk_x * 4;
            let chunk_bz0 = cc.chunk_z * 4;
            let bx_s = (min_bx - chunk_bx0).max(0);
            let bx_e = (max_bx - chunk_bx0).min(3);
            let bz_s = (min_bz - chunk_bz0).max(0);
            let bz_e = (max_bz - chunk_bz0).min(3);
            if bx_s <= bx_e && bz_s <= bz_e {
                for sec in &cc.sections {
                    let sec_by0 = sec.sec_y * 4;
                    let by_s = (min_by - sec_by0).max(0);
                    let by_e = (max_by - sec_by0).min(3);
                    if by_s > by_e {
                        continue;
                    }
                    for lby in by_s..=by_e {
                        let gby = (sec_by0 + lby - min_by) as usize;
                        for lbz in bz_s..=bz_e {
                            let gbz = (chunk_bz0 + lbz - min_bz) as usize;
                            let dst_row = (gby * bsz + gbz) * bsx;
                            let src_row = (lby as usize * 4 + lbz as usize) * 4;
                            for lbx in bx_s..=bx_e {
                                let gbx = (chunk_bx0 + lbx - min_bx) as usize;
                                let local_b = sec.biome_indices.get(src_row + lbx as usize).copied().unwrap_or(0) as usize;
                                biomes[dst_row + gbx] = global_biome_remap.get(local_b).copied().unwrap_or(0);
                            }
                        }
                    }
                }
            }

            remapped_chunks.push(RemappedChunk {
                chunk_x: cc.chunk_x,
                chunk_z: cc.chunk_z,
                global_block_remap,
                sections: cc.sections,
            });
        }

        // Index sections by sec_y so each Y layer can be scattered in parallel without contention
        let mut by_sec_y: HashMap<i32, Vec<(usize, usize)>> = HashMap::new();
        for (c_idx, rc) in remapped_chunks.iter().enumerate() {
            for (s_idx, sec) in rc.sections.iter().enumerate() {
                by_sec_y.entry(sec.sec_y).or_default().push((c_idx, s_idx));
            }
        }

        let region_non_air: usize = blocks
            .par_chunks_mut(sx * sz)
            .enumerate()
            .map(|(gy, layer)| {
                let wy = coords.min_y + gy as i32;
                let sec_y = wy.div_euclid(16);
                let Some(entries) = by_sec_y.get(&sec_y) else {
                    return 0usize;
                };
                let ly = (wy - sec_y * 16) as usize;
                let row_src_y = ly * 256;
                let mut layer_non_air = 0usize;

                for &(c_idx, s_idx) in entries {
                    let rc = &remapped_chunks[c_idx];
                    let sec = &rc.sections[s_idx];
                    let base_wx = (rc.chunk_x as i64) * 16;
                    let base_wz = (rc.chunk_z as i64) * 16;
                    let lz_s = (coords.min_z as i64 - base_wz).max(0);
                    let lz_e = (coords.max_z as i64 - base_wz).min(15);
                    let lx_s = (coords.min_x as i64 - base_wx).max(0);
                    let lx_e = (coords.max_x as i64 - base_wx).min(15);
                    if lz_s > lz_e || lx_s > lx_e {
                        continue;
                    }
                    let (lz_start, lz_end) = (lz_s as usize, lz_e as usize);
                    let (lx_start, lx_end) = (lx_s as usize, lx_e as usize);
                    let remap = &rc.global_block_remap;

                    for lz in lz_start..=lz_end {
                        let wz = (base_wz + lz as i64) as i32;
                        let gz = (wz - coords.min_z) as usize;
                        let row_dst = gz * sx;
                        let row_src = row_src_y + lz * 16;
                        for lx in lx_start..=lx_end {
                            let wx = (base_wx + lx as i64) as i32;
                            let gx = (wx - coords.min_x) as usize;
                            let local_id = sec.block_indices[row_src + lx] as usize;
                            let gid = remap[local_id];
                            if gid != 0 {
                                layer_non_air += 1;
                            }
                            layer[row_dst + gx] = gid;
                        }
                    }
                }
                layer_non_air
            })
            .sum();
        non_air_count += region_non_air;
    }

    let player_spawn_mc = read_level_spawn(world_dir);

    if non_air_count == 0 {
        let mut msg = vec![format!(
            "0 non-air blocks found in coordinate bounds ({}, {}, {}) .. ({}, {}, {}) (dimension={}, region_dir={}).",
            coords.min_x, coords.min_y, coords.min_z,
            coords.max_x, coords.max_y, coords.max_z,
            dimension, region_dir.display()
        )];
        let (reg_count, reg_bounds) = scan_region_bounds(&region_dir);
        msg.push(format!(
            "Matched {} .mca region file(s) in bounds ({} total region file(s) in directory).",
            mca_paths.len(),
            reg_count
        ));
        if let Some((rx0, rz0, rx1, rz1)) = reg_bounds {
            msg.push(format!(
                "Generated region bounds: X from {} to {}, Z from {} to {}",
                rx0, rx1, rz0, rz1
            ));
        }
        if let Some([px, py, pz]) = player_spawn_mc {
            msg.push(format!(
                "Detected player/spawn position: X={}, Y={}, Z={}",
                px.round() as i32,
                py.round() as i32,
                pz.round() as i32
            ));
            let sx0 = (px as i32) - 30;
            let sy0 = ((py as i32) - 15).max(-64);
            let sz0 = (pz as i32) - 30;
            let sx1 = (px as i32) + 30;
            let sy1 = ((py as i32) + 35).min(320);
            let sz1 = (pz as i32) + 30;
            msg.push(format!(
                "Suggested coords: --coords {} {} {} {} {} {}",
                sx0, sy0, sz0, sx1, sy1, sz1
            ));
        }
        return Err(msg.join("\n"));
    }

    Ok(VoxelGrid {
        coords,
        palette,
        air_id: 0,
        blocks,
        biome_palette,
        biomes,
        signs,
        player_spawn_mc,
    })
}
