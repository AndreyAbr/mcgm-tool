use crate::anvil::{BlockState, VoxelGrid};
use crate::assets::{split_namespaced, FaceDir, ModelElement, ObjTri, ResolvedBlockModel};
use crate::atlas::{effective_biome_tint, sign_base_key, RenderBucket, SignAtlas, TexRequestKey, TextureAtlas, UvRect};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct MeshTri {
    pub material: Cow<'static, str>, // "terrain_opaque", "terrain_cutout", "terrain_translucent", "sign_atlas", "mc_water_flow"
    pub pos: [[f32; 3]; 3],
    pub norm: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
}

#[derive(Debug, Clone)]
pub struct ChunkMesh {
    pub cx: i32,
    pub cz: i32,
    pub name: String,
    pub origin_source: [f64; 3],
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub solid_tris: Vec<MeshTri>,       // terrain_opaque + terrain_cutout + sign_atlas
    pub translucent_tris: Vec<MeshTri>, // terrain_translucent
    #[allow(dead_code)]
    pub has_cutout: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractableDoor {
    pub mc_pos: [i32; 3],
    pub box_coords: [f64; 6],
    pub origin: [f64; 3], // hinge pivot
    pub facing: String,
    pub hinge: String,
    pub open: bool,
    pub spawnflags: u32,
    pub mat_name: String, // door_<type>
    pub short_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractableButton {
    pub kind: String, // "button" or "lever"
    pub box_coords: [f64; 6],
    pub origin: [f64; 3],
    pub facing: String,
    pub face: String,
    pub move_dir: [f64; 3],
    pub target_ids: Vec<String>,
    pub powered: bool,
    pub material: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockLight {
    pub pos: [f64; 3],
    pub color: String,
    pub brightness: i32,
    pub dist50: i32,
    pub dist0: i32,
}

#[derive(Debug, Clone)]
pub struct ParkourMarker {
    pub id: String,
    pub kind: crate::anvil::ParkourMarkerKind,
    pub name: String,
    pub mc_pos: [i32; 3],
    pub is_wall: bool,
    pub zone_min: [f64; 3],
    pub zone_max: [f64; 3],
    pub anchor_pos: [f64; 3],
    pub forward_dir: [f64; 2],
    pub yaw: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ExtractedFeatures {
    pub doors: Vec<InteractableDoor>,
    pub door_block_names: HashSet<String>,
    pub door_voxels: HashSet<(i32, i32, i32)>,
    pub flat_water_voxels: HashSet<(i32, i32, i32)>,
    pub skip_mesh_bits: Vec<u64>,
    pub buttons: Vec<InteractableButton>,
    pub lights: Vec<BlockLight>,
    pub spawn_points_mc: Vec<[f64; 3]>,
}

#[derive(Clone, Copy)]
enum NameMatch {
    Prefix(&'static str),
    Suffix(&'static str),
    Contains(&'static str),
}

impl NameMatch {
    #[inline]
    fn matches(self, s: &str) -> bool {
        match self {
            Self::Prefix(p) => s.starts_with(p),
            Self::Suffix(p) => s.ends_with(p),
            Self::Contains(p) => s.contains(p),
        }
    }
}

#[derive(Clone, Copy)]
struct OccluderRule {
    matcher: NameMatch,
    except: Option<&'static str>,
}

const NON_OCCLUDER_EXACT: &[&str] = &[
    "dirt_path",
    "farmland",
    "leaves",
    "iron_bars",
    "ice",
    "frosted_ice",
    "slime_block",
    "honey_block",
    "cobweb",
    "ladder",
    "vine",
    "dandelion",
    "poppy",
    "blue_orchid",
    "allium",
    "azure_bluet",
    "oxeye_daisy",
    "lily_of_the_valley",
    "wither_rose",
    "rose_bush",
    "peony",
    "lilac",
    "pitcher_plant",
    "pitcher_crop",
    "wildflowers",
    "open_eyeblossom",
    "closed_eyeblossom",
    "eyeblossom",
    "dead_bush",
    "bush",
    "sugar_cane",
    "wheat",
    "carrots",
    "potatoes",
    "beetroots",
    "nether_wart",
    "cocoa",
    "red_mushroom",
    "brown_mushroom",
    "crimson_fungus",
    "warped_fungus",
    "crimson_roots",
    "warped_roots",
    "hanging_roots",
    "mangrove_roots",
    "nether_sprouts",
    "glow_lichen",
    "sculk_vein",
    "resin_clump",
    "seagrass",
    "tall_seagrass",
    "kelp",
    "kelp_plant",
    "lily_pad",
    "sea_pickle",
    "frogspawn",
    "azalea",
    "flowering_azalea",
    "spore_blossom",
    "big_dripleaf",
    "small_dripleaf",
    "mangrove_propagule",
    "pale_hanging_moss",
    "chorus_plant",
    "redstone_wire",
    "repeater",
    "comparator",
    "tripwire",
    "tripwire_hook",
    "daylight_detector",
    "stonecutter",
    "grindstone",
    "beacon",
    "conduit",
    "end_portal_frame",
    "dragon_egg",
    "turtle_egg",
    "sniffer_egg",
    "cake",
    "candle",
    "pointed_dripstone",
    "lightning_rod",
    "heavy_core",
    "decorated_pot",
    "amethyst_cluster",
    "end_portal",
    "end_gateway",
    "nether_portal",
    "barrier",
    "light",
    "structure_void",
    "piston_head",
    "moving_piston",
    "spawner",
    "mob_spawner",
    "trial_spawner",
    "vault",
    "copper_golem_statue",
    "lever",
    "lava",
    "flowing_lava",
    "water",
    "flowing_water",
    "cactus",
    "anvil",
    "chipped_anvil",
    "damaged_anvil",
    "enchanting_table",
    "brewing_stand",
    "hopper",
    "cauldron",
    "lectern",
    "bell",
    "chain",
    "end_rod",
    "composter",
    "scaffolding",
    "bamboo",
    "flower_pot",
    "leaf_litter",
    "fallen_leaves",
    "leaf_carpet",
    "pink_petals",
];

const NON_OCCLUDER_RULES: &[OccluderRule] = &[
    OccluderRule { matcher: NameMatch::Prefix("potted_"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_leaves"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_stairs"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_fence"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_fence_gate"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_wall"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_pane"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_door"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_trapdoor"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_bed"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_tulip"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_bush"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_stem"), except: Some("mushroom_stem") },
    OccluderRule { matcher: NameMatch::Suffix("_coral"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_cake"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_candle"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_head"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_skull"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_amethyst_bud"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("shulker_box"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_grate"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_bars"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_chain"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_lightning_rod"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_shelf"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_copper_golem_statue"), except: None },
    OccluderRule { matcher: NameMatch::Suffix("_cauldron"), except: None },
    OccluderRule { matcher: NameMatch::Contains("leaves"), except: None },
    OccluderRule { matcher: NameMatch::Contains("glass"), except: None },
    OccluderRule { matcher: NameMatch::Contains("torch"), except: None },
    OccluderRule { matcher: NameMatch::Contains("lantern"), except: None },
    OccluderRule { matcher: NameMatch::Contains("sign"), except: None },
    OccluderRule { matcher: NameMatch::Contains("banner"), except: None },
    OccluderRule { matcher: NameMatch::Contains("chest"), except: None },
    OccluderRule { matcher: NameMatch::Contains("sapling"), except: None },
    OccluderRule { matcher: NameMatch::Contains("flower"), except: None },
    OccluderRule { matcher: NameMatch::Contains("vine"), except: None },
    OccluderRule { matcher: NameMatch::Contains("_coral_fan"), except: None },
    OccluderRule { matcher: NameMatch::Contains("grass"), except: Some("grass_block") },
    OccluderRule { matcher: NameMatch::Contains("fern"), except: None },
    OccluderRule { matcher: NameMatch::Contains("rail"), except: None },
    OccluderRule { matcher: NameMatch::Contains("button"), except: None },
    OccluderRule { matcher: NameMatch::Contains("pressure_plate"), except: None },
    OccluderRule { matcher: NameMatch::Contains("carpet"), except: None },
    OccluderRule { matcher: NameMatch::Contains("fire"), except: None },
];

const CREATE_NON_OCCLUDER_TOKENS: &[&str] = &[
    "cogwheel", "shaft", "belt", "pipe", "track", "girder", "funnel", "chute", "wheel",
];

pub fn is_full_opaque_occluder(state: &BlockState) -> bool {
    if state.is_air || state.is_water_tag {
        return false;
    }
    let s = state.short_name();
    if NON_OCCLUDER_EXACT.contains(&s) {
        return false;
    }
    if NON_OCCLUDER_RULES
        .iter()
        .any(|r| r.matcher.matches(s) && r.except != Some(s))
    {
        return false;
    }
    if s == "snow" {
        let layers: i32 = state.prop("layers").and_then(|v| v.parse().ok()).unwrap_or(1);
        return layers >= 8;
    }
    if s.ends_with("_slab") {
        return state.prop("type") == Some("double");
    }
    if state.name.starts_with("create:") && CREATE_NON_OCCLUDER_TOKENS.iter().any(|t| s.contains(t)) {
        return false;
    }
    true
}

#[inline]
fn is_carpet_name(s: &str) -> bool {
    s.ends_with("_carpet") || s == "carpet" || s == "moss_carpet" || s == "pale_moss_carpet"
}

#[inline]
fn is_lava_name(s: &str) -> bool {
    s == "lava" || s == "flowing_lava"
}

#[inline]
fn is_water_host(bs: &BlockState) -> bool {
    bs.is_water_tag
        || bs.waterlogged
        || matches!(
            bs.short_name(),
            "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant"
        )
}

#[inline]
fn is_leaves_name(s: &str) -> bool {
    if matches!(
        s,
        "leaf_litter" | "fallen_leaves" | "leaf_carpet" | "pink_petals" | "moss_carpet"
    ) {
        return false;
    }
    s == "leaves" || s.ends_with("_leaves") || s.contains("leaves")
}

#[inline]
fn is_self_culling_translucent(s: &str) -> bool {
    matches!(
        s,
        "ice" | "frosted_ice" | "slime_block" | "honey_block" | "nether_portal"
    )
}

#[cfg(test)]
pub fn should_cull_face(self_block: &BlockState, neighbor_block: &BlockState, face: FaceDir) -> bool {
    should_cull_face_with_occluder(self_block, neighbor_block, is_full_opaque_occluder(neighbor_block), face)
}

const CULL_AIR: u16 = 1 << 0;
const CULL_WATER: u16 = 1 << 1;
const CULL_LAVA: u16 = 1 << 2;
const CULL_LEAVES: u16 = 1 << 3;
const CULL_SNOW: u16 = 1 << 4;
const CULL_CARPET: u16 = 1 << 5;
const CULL_FARMLAND_OR_PATH: u16 = 1 << 6;
const CULL_OCCLUDER: u16 = 1 << 7;
const CULL_WATER_HOST: u16 = 1 << 8;

#[derive(Clone, Copy)]
struct PaletteCullInfo {
    flags: u16,
    snow_layers: u8,
    fluid_level: u8,
    self_cull_group: u16, // 0 = none, >0 = glass/ice/slime/honey/nether_portal group id
}

fn build_palette_cull_info(palette: &[BlockState], palette_occluder: &[bool]) -> Vec<PaletteCullInfo> {
    let mut self_cull_map: HashMap<String, u16> = HashMap::new();
    let mut next_group: u16 = 1;

    palette
        .iter()
        .enumerate()
        .map(|(pid, bs)| {
            let s = bs.short_name();
            let mut flags = 0u16;
            if bs.is_air {
                flags |= CULL_AIR;
            }
            if bs.is_water_tag {
                flags |= CULL_WATER;
            }
            if is_water_host(bs) {
                flags |= CULL_WATER_HOST;
            }
            if is_lava_name(s) {
                flags |= CULL_LAVA;
            }
            if is_leaves_name(s) {
                flags |= CULL_LEAVES;
            }
            if s == "snow" {
                flags |= CULL_SNOW;
            }
            if is_carpet_name(s) {
                flags |= CULL_CARPET;
            }
            if matches!(s, "farmland" | "dirt_path") {
                flags |= CULL_FARMLAND_OR_PATH;
            }
            if palette_occluder.get(pid).copied().unwrap_or(false) {
                flags |= CULL_OCCLUDER;
            }

            let snow_layers = if s == "snow" {
                bs.prop("layers").and_then(|v| v.parse::<u8>().ok()).unwrap_or(1)
            } else {
                0
            };
            let fluid_level = if bs.is_water_tag || is_lava_name(s) {
                bs.prop("level").and_then(|v| v.parse::<u8>().ok()).unwrap_or(0)
            } else {
                0
            };
            let self_cull_group = if (s.contains("glass") && !s.ends_with("_pane")) || is_self_culling_translucent(s) {
                *self_cull_map.entry(s.to_string()).or_insert_with(|| {
                    let g = next_group;
                    next_group = next_group.saturating_add(1);
                    g
                })
            } else {
                0
            };

            PaletteCullInfo {
                flags,
                snow_layers,
                fluid_level,
                self_cull_group,
            }
        })
        .collect()
}

#[inline]
fn should_cull_pid(self_info: PaletteCullInfo, nei_info: PaletteCullInfo, face: FaceDir) -> bool {
    if (nei_info.flags & CULL_AIR) != 0 {
        return false;
    }
    let is_horiz = !matches!(face, FaceDir::Up | FaceDir::Down);
    if !is_horiz && ((self_info.flags & nei_info.flags) & CULL_WATER) != 0 {
        return true;
    }
    if !is_horiz && (self_info.flags & CULL_WATER) != 0 && (nei_info.flags & CULL_WATER_HOST) != 0 {
        return true;
    }
    if (self_info.flags & (CULL_WATER | CULL_LAVA)) != 0
        && is_horiz
        && (nei_info.flags & CULL_FARMLAND_OR_PATH) != 0
        && self_info.fluid_level < 8
    {
        return true;
    }
    if !is_horiz && ((self_info.flags & nei_info.flags) & CULL_LAVA) != 0 {
        return true;
    }
    if ((self_info.flags & nei_info.flags) & CULL_LEAVES) != 0 {
        return true;
    }
    if self_info.self_cull_group != 0 && self_info.self_cull_group == nei_info.self_cull_group {
        return true;
    }
    if ((self_info.flags & nei_info.flags) & CULL_SNOW) != 0
        && is_horiz
        && nei_info.snow_layers >= self_info.snow_layers
    {
        return true;
    }
    if ((self_info.flags & nei_info.flags) & CULL_CARPET) != 0 && is_horiz {
        return true;
    }
    (nei_info.flags & CULL_OCCLUDER) != 0
}

#[inline]
#[cfg(test)]
fn should_cull_face_with_occluder(
    self_block: &BlockState,
    neighbor_block: &BlockState,
    neighbor_is_occluder: bool,
    face: FaceDir,
) -> bool {
    if neighbor_block.is_air {
        return false;
    }
    if self_block.is_water_tag && neighbor_block.is_water_tag {
        return true;
    }
    let s_self = self_block.short_name();
    let s_nei = neighbor_block.short_name();
    if (self_block.is_water_tag || is_lava_name(s_self))
        && !matches!(face, FaceDir::Up | FaceDir::Down)
        && matches!(s_nei, "farmland" | "dirt_path")
    {
        let lvl: i32 = self_block
            .prop("level")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if lvl < 8 {
            return true;
        }
    }
    if is_lava_name(s_self) && is_lava_name(s_nei) {
        return true;
    }
    if is_leaves_name(s_self) && is_leaves_name(s_nei) {
        return true;
    }
    if s_self.contains("glass") && !s_self.ends_with("_pane") && s_self == s_nei {
        return true;
    }
    if is_self_culling_translucent(s_self) && s_self == s_nei {
        return true;
    }
    if s_self == "snow" && s_nei == "snow" && !matches!(face, FaceDir::Up | FaceDir::Down) {
        let l_self: i32 = self_block.prop("layers").and_then(|v| v.parse().ok()).unwrap_or(1);
        let l_nei: i32 = neighbor_block.prop("layers").and_then(|v| v.parse().ok()).unwrap_or(1);
        if l_nei >= l_self {
            return true;
        }
    }
    if is_carpet_name(s_self) && is_carpet_name(s_nei) && !matches!(face, FaceDir::Up | FaceDir::Down) {
        return true;
    }
    neighbor_is_occluder
}

fn model_has_full_cube(model: &ResolvedBlockModel) -> bool {
    match model {
        ResolvedBlockModel::Elements(els) => els.iter().any(|el| {
            el.rotation.is_none()
                && el.faces.len() >= 6
                && el.from[0] <= 0.01
                && el.from[1] <= 0.01
                && el.from[2] <= 0.01
                && el.to[0] >= 15.99
                && el.to[1] >= 15.99
                && el.to[2] >= 15.99
        }),
        ResolvedBlockModel::Obj(_) | ResolvedBlockModel::Empty => false,
    }
}

pub fn compute_world_origin_mc(grid: &VoxelGrid) -> [f64; 3] {
    let c = grid.coords;
    [
        (c.min_x + c.max_x) as f64 / 2.0 + 0.5,
        c.min_y as f64,
        (c.min_z + c.max_z) as f64 / 2.0 + 0.5,
    ]
}

#[inline]
pub fn mc_point_to_source(mc: [f64; 3], origin_mc: [f64; 3], scale: f64) -> [f32; 3] {
    [
        ((mc[0] - origin_mc[0]) * scale) as f32,
        (-(mc[2] - origin_mc[2]) * scale) as f32,
        ((mc[1] - origin_mc[1]) * scale) as f32,
    ]
}

#[inline]
pub fn mc_normal_to_source(n: [f32; 3]) -> [f32; 3] {
    [n[0], -n[2], n[1]]
}

struct PaletteFeatMeta {
    is_air: bool,
    is_door: bool,
    is_fence_gate: bool,
    is_trigger: bool,
    is_flat_water_candidate: bool,
    is_occluder: bool,
    contains_flat_water: bool,
    below_ok_for_flat_water: bool,
    light_spec: Option<(&'static str, i32, u8)>,
}

type RawLightCandidate = (u8, i32, i32, i32, f64, f64, f64, String, i32);

struct SliceFeatureAcc {
    doors: Vec<InteractableDoor>,
    door_block_names: HashSet<String>,
    door_voxels: Vec<(i32, i32, i32)>,
    flat_water_voxels: Vec<(i32, i32, i32)>,
    raw_triggers: Vec<([i32; 3], InteractableButton)>,
    raw_lights: Vec<RawLightCandidate>,
    seen_pid_biome: Vec<u64>,
}

impl SliceFeatureAcc {
    fn new(seen_words: usize) -> Self {
        Self {
            doors: Vec::new(),
            door_block_names: HashSet::new(),
            door_voxels: Vec::new(),
            flat_water_voxels: Vec::new(),
            raw_triggers: Vec::new(),
            raw_lights: Vec::new(),
            seen_pid_biome: vec![0u64; seen_words],
        }
    }

    fn merge(mut self, other: Self) -> Self {
        self.doors.extend(other.doors);
        self.door_block_names.extend(other.door_block_names);
        self.door_voxels.extend(other.door_voxels);
        self.flat_water_voxels.extend(other.flat_water_voxels);
        self.raw_triggers.extend(other.raw_triggers);
        self.raw_lights.extend(other.raw_lights);
        for (dst, src) in self.seen_pid_biome.iter_mut().zip(other.seen_pid_biome.iter()) {
            *dst |= *src;
        }
        self
    }
}

pub fn extract_features_and_requests(
    grid: &VoxelGrid,
    models: &[ResolvedBlockModel],
    origin_mc: [f64; 3],
    scale: f64,
) -> (ExtractedFeatures, HashSet<TexRequestKey>) {
    let mut feat = ExtractedFeatures::default();
    let mut requests = HashSet::new();

    let c = grid.coords;
    let sx = c.size_x();
    let sy = c.size_y();
    let sz = c.size_z();

    let num_biomes = grid.biome_palette.len().max(1);
    let seen_words = (grid.palette.len() * num_biomes).div_ceil(64);

    let pal_meta: Vec<PaletteFeatMeta> = grid
        .palette
        .iter()
        .map(|bs| {
            let sname = bs.short_name();
            let is_occluder = is_full_opaque_occluder(bs);
            let contains_flat_water = is_water_host(bs)
                || is_occluder
                || matches!(sname, "farmland" | "dirt_path");
            let below_ok_for_flat_water = bs.is_water_tag || is_occluder || !bs.is_air;
            PaletteFeatMeta {
                is_air: bs.is_air,
                is_door: sname.ends_with("_door"),
                is_fence_gate: sname.ends_with("_fence_gate"),
                is_trigger: sname.ends_with("_button") || sname == "lever",
                is_flat_water_candidate: crate::collision::is_water_brush_eligible(bs)
                    || matches!(sname, "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant"),
                is_occluder,
                contains_flat_water,
                below_ok_for_flat_water,
                light_spec: block_light_spec(bs),
            }
        })
        .collect();

    let mut acc = (0..sy)
        .into_par_iter()
        .fold(
            || SliceFeatureAcc::new(seen_words),
            |mut acc, gy| {
                let wy = c.min_y + gy as i32;
                for gz in 0..sz {
                    let wz = c.min_z + gz as i32;
                    let row = (gy * sz + gz) * sx;
                    for gx in 0..sx {
                        let wx = c.min_x + gx as i32;
                        let pid = grid.blocks[row + gx] as usize;
                        let pm = &pal_meta[pid];
                        if pm.is_air {
                            continue;
                        }
                        let bs = &grid.palette[pid];
                        let sname = bs.short_name();

                        // Doors
                        if pm.is_door {
                            acc.door_voxels.push((wx, wy, wz));
                            acc.door_block_names.insert(bs.name.clone());
                            let half = bs.prop("half").unwrap_or("lower");
                            let below_is_door = wy > c.min_y
                                && pal_meta[grid.blocks[((gy - 1) * sz + gz) * sx + gx] as usize].is_door;
                            if half == "lower" || !below_is_door {
                                let is_two_high = half == "lower"
                                    && gy + 1 < sy
                                    && pal_meta[grid.blocks[((gy + 1) * sz + gz) * sx + gx] as usize].is_door;
                                let facing = bs.prop("facing").unwrap_or("north").to_string();
                                let hinge = bs.prop("hinge").unwrap_or("left").to_string();
                                let open = bs.prop("open").map(|v| v == "true").unwrap_or(false);
                                let (door_box, origin, spawnflags) = compute_door_geometry(
                                    wx,
                                    wy,
                                    wz,
                                    if is_two_high { 2.0 } else { 1.0 },
                                    &facing,
                                    &hinge,
                                    open,
                                    origin_mc,
                                    scale,
                                );
                                let (_, short) = split_namespaced(&bs.name);
                                let mat_name = format!("door_{}", short.replace('/', "_"));
                                acc.doors.push(InteractableDoor {
                                    mc_pos: [wx, wy, wz],
                                    box_coords: door_box,
                                    origin,
                                    facing,
                                    hinge,
                                    open,
                                    spawnflags,
                                    mat_name,
                                    short_type: short.to_string(),
                                });
                            }
                            continue;
                        }

                        // Fence gates
                        if pm.is_fence_gate {
                            acc.door_voxels.push((wx, wy, wz));
                            acc.door_block_names.insert(bs.name.clone());
                            let facing = bs.prop("facing").unwrap_or("north").to_string();
                            let open = bs.prop("open").map(|v| v == "true").unwrap_or(false);
                            let (gate_box, origin, spawnflags) =
                                compute_fence_gate_geometry(wx, wy, wz, &facing, open, origin_mc, scale);
                            let (_, short) = split_namespaced(&bs.name);
                            acc.doors.push(InteractableDoor {
                                mc_pos: [wx, wy, wz],
                                box_coords: gate_box,
                                origin,
                                facing,
                                hinge: "left".to_string(),
                                open,
                                spawnflags,
                                mat_name: format!("door_{}", short.replace('/', "_")),
                                short_type: short.to_string(),
                            });
                            continue;
                        }

                        // Buttons & Levers (collected and proximity-filtered against doors/gates below)
                        if pm.is_trigger {
                            acc.door_block_names.insert(bs.name.clone());
                            let (_, short) = split_namespaced(&bs.name);
                            let is_lever = sname == "lever";
                            let facing = bs.prop("facing").unwrap_or("south").to_string();
                            let face = bs.prop("face").unwrap_or("wall").to_string();
                            let powered = bs.prop("powered").map(|v| v == "true").unwrap_or(false);
                            let (bx0, by0, bz0, bx1, by1, bz1, move_dir) = if is_lever {
                                match face.as_str() {
                                    "floor" => (0.3125, 0.0, 0.25, 0.6875, 0.25, 0.75, [0.0, -1.0, 0.0]),
                                    "ceiling" => (0.3125, 0.75, 0.25, 0.6875, 1.0, 0.75, [0.0, 1.0, 0.0]),
                                    _ => match facing.as_str() {
                                        "south" => (0.3125, 0.25, 0.0, 0.6875, 0.75, 0.25, [0.0, 0.0, -1.0]),
                                        "north" => (0.3125, 0.25, 0.75, 0.6875, 0.75, 1.0, [0.0, 0.0, 1.0]),
                                        "east" => (0.0, 0.25, 0.3125, 0.25, 0.75, 0.6875, [-1.0, 0.0, 0.0]),
                                        _ => (0.75, 0.25, 0.3125, 1.0, 0.75, 0.6875, [1.0, 0.0, 0.0]),
                                    },
                                }
                            } else {
                                match face.as_str() {
                                    "floor" => (0.3125, 0.0, 0.375, 0.6875, 0.125, 0.625, [0.0, -1.0, 0.0]),
                                    "ceiling" => (0.3125, 0.875, 0.375, 0.6875, 1.0, 0.625, [0.0, 1.0, 0.0]),
                                    _ => match facing.as_str() {
                                        "south" => (0.3125, 0.375, 0.0, 0.6875, 0.625, 0.125, [0.0, 0.0, -1.0]),
                                        "north" => (0.3125, 0.375, 0.875, 0.6875, 0.625, 1.0, [0.0, 0.0, 1.0]),
                                        "east" => (0.0, 0.375, 0.3125, 0.125, 0.625, 0.6875, [-1.0, 0.0, 0.0]),
                                        _ => (0.875, 0.375, 0.3125, 1.0, 0.625, 0.6875, [1.0, 0.0, 0.0]),
                                    },
                                }
                            };
                            let p0 = mc_point_to_source(
                                [wx as f64 + bx0, wy as f64 + by0, wz as f64 + bz0],
                                origin_mc,
                                scale,
                            );
                            let p1 = mc_point_to_source(
                                [wx as f64 + bx1, wy as f64 + by1, wz as f64 + bz1],
                                origin_mc,
                                scale,
                            );
                            let box_coords = [
                                p0[0].min(p1[0]) as f64,
                                p0[1].min(p1[1]) as f64,
                                p0[2].min(p1[2]) as f64,
                                p0[0].max(p1[0]) as f64,
                                p0[1].max(p1[1]) as f64,
                                p0[2].max(p1[2]) as f64,
                            ];
                            let origin = [
                                (box_coords[0] + box_coords[3]) * 0.5,
                                (box_coords[1] + box_coords[4]) * 0.5,
                                (box_coords[2] + box_coords[5]) * 0.5,
                            ];
                            acc.raw_triggers.push((
                                [wx, wy, wz],
                                InteractableButton {
                                    kind: if is_lever { "lever" } else { "button" }.to_string(),
                                    box_coords,
                                    origin,
                                    facing,
                                    face,
                                    move_dir,
                                    target_ids: Vec::new(),
                                    powered,
                                    material: format!("door_{}", short.replace('/', "_")),
                                },
                            ));
                        }

                        // Contained flat water pool voxels (rendered as BSP water brushes)
                        if pm.is_flat_water_candidate {
                            let below_ok = if gy > 0 {
                                pal_meta[grid.blocks[((gy - 1) * sz + gz) * sx + gx] as usize]
                                    .below_ok_for_flat_water
                            } else {
                                wy == c.min_y
                            };
                            if below_ok {
                                let open_x_pos = gx + 1 < sx
                                    && !pal_meta[grid.blocks[row + gx + 1] as usize].contains_flat_water;
                                let open_x_neg = gx > 0
                                    && !pal_meta[grid.blocks[row + gx - 1] as usize].contains_flat_water;
                                let open_z_pos = gz + 1 < sz
                                    && !pal_meta[grid.blocks[(gy * sz + gz + 1) * sx + gx] as usize]
                                        .contains_flat_water;
                                let open_z_neg = gz > 0
                                    && !pal_meta[grid.blocks[(gy * sz + gz - 1) * sx + gx] as usize]
                                        .contains_flat_water;
                                if !(open_x_pos || open_x_neg || open_z_pos || open_z_neg) {
                                    acc.flat_water_voxels.push((wx, wy, wz));
                                }
                            }
                        }

                        // Block lights (offset emitters cleanly out of opaque model geometry into open air)
                        if let Some((col, br, prio)) = pm.light_spec {
                            let mut lx = wx as f64 + 0.5;
                            let mut ly = wy as f64 + 0.55;
                            let mut lz = wz as f64 + 0.5;
                            let mut emit_light = true;
                            if sname == "wall_torch" || sname == "soul_wall_torch" || sname.ends_with("_wall_torch") {
                                match bs.prop("facing").unwrap_or("north") {
                                    "north" => lz -= 0.08,
                                    "south" => lz += 0.08,
                                    "east" => lx += 0.08,
                                    "west" => lx -= 0.08,
                                    _ => {}
                                }
                                ly = wy as f64 + 0.86;
                            } else if sname == "torch" || sname == "soul_torch" || sname.ends_with("_torch") {
                                ly = wy as f64 + 0.78;
                            } else if sname == "campfire" || sname == "soul_campfire" {
                                ly = wy as f64 + 0.90;
                            } else if sname == "lantern" || sname == "soul_lantern" || sname.ends_with("_lantern") {
                                if bs.prop("hanging") == Some("true") {
                                    let below_open = gy == 0
                                        || !pal_meta[grid.blocks[((gy - 1) * sz + gz) * sx + gx] as usize]
                                            .is_occluder;
                                    ly = if below_open { wy as f64 - 0.15 } else { wy as f64 + 0.05 };
                                } else {
                                    ly = wy as f64 + 0.65;
                                }
                            } else if matches!(sname, "lava" | "flowing_lava") {
                                // Lava surface meshes sit at wy + 0.875 (below full block top) and
                                // still blocks/flowing blocks can occlude a light placed at +0.5.
                                // Prefer placing above the lava surface; otherwise offset into an
                                // adjacent non-occluding block, or skip if completely buried.
                                let above_open = !c.contains(wx, wy + 1, wz)
                                    || !pal_meta[grid.blocks
                                        [((gy + 1) * sz + gz) * sx + gx] as usize]
                                        .is_occluder;
                                if above_open {
                                    ly = wy as f64 + 1.15;
                                } else {
                                    let mut found_air = None;
                                    for (dx, dy, dz) in
                                        [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, -1, 0)]
                                    {
                                        let nx = wx + dx;
                                        let ny = wy + dy;
                                        let nz = wz + dz;
                                        if !c.contains(nx, ny, nz) {
                                            found_air = Some((dx, dy, dz));
                                            break;
                                        }
                                        let n_pid = grid.blocks[((gy as i32 + dy) as usize * sz
                                            + (gz as i32 + dz) as usize)
                                            * sx
                                            + (gx as i32 + dx) as usize]
                                            as usize;
                                        if !pal_meta[n_pid].is_occluder {
                                            found_air = Some((dx, dy, dz));
                                            break;
                                        }
                                    }
                                    if let Some((dx, dy, dz)) = found_air {
                                        lx += dx as f64 * 0.65;
                                        ly = wy as f64 + 0.5 + dy as f64 * 0.65;
                                        lz += dz as f64 * 0.65;
                                    } else {
                                        emit_light = false;
                                    }
                                }
                            } else if is_full_block_light_emitter(sname) {
                                let mut found_air = None;
                                for (dx, dy, dz) in
                                    [(0, -1, 0), (0, 1, 0), (1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)]
                                {
                                    let nx = wx + dx;
                                    let ny = wy + dy;
                                    let nz = wz + dz;
                                    if !c.contains(nx, ny, nz) {
                                        found_air = Some((dx, dy, dz));
                                        break;
                                    }
                                    let n_pid = grid.blocks[((gy as i32 + dy) as usize * sz
                                        + (gz as i32 + dz) as usize)
                                        * sx
                                        + (gx as i32 + dx) as usize] as usize;
                                    if !pal_meta[n_pid].is_occluder {
                                        found_air = Some((dx, dy, dz));
                                        break;
                                    }
                                }
                                if let Some((dx, dy, dz)) = found_air {
                                    lx += dx as f64 * 0.65;
                                    ly = wy as f64 + 0.5 + dy as f64 * 0.65;
                                    lz += dz as f64 * 0.65;
                                } else {
                                    emit_light = false;
                                }
                            }
                            if emit_light {
                                let s_pt = mc_point_to_source([lx, ly, lz], origin_mc, scale);
                                acc.raw_lights.push((
                                    prio,
                                    wy,
                                    wz,
                                    wx,
                                    s_pt[0] as f64,
                                    s_pt[1] as f64,
                                    s_pt[2] as f64,
                                    col.to_string(),
                                    br,
                                ));
                            }
                        }

                        // Collect texture requests (deduplicated per (pid, biome_idx) via thread-local bitset)
                        let bid = grid.get_biome_id(gx, gy, gz);
                        let seen_idx = pid * num_biomes + bid;
                        let word = seen_idx >> 6;
                        let bit = 1u64 << (seen_idx & 63);
                        if (acc.seen_pid_biome[word] & bit) == 0 {
                            acc.seen_pid_biome[word] |= bit;
                        }
                    }
                }
                acc
            },
        )
        .reduce(|| SliceFeatureAcc::new(seen_words), SliceFeatureAcc::merge);

    // Deterministic ordering matching sequential (gy, gz, gx) scan
    acc.doors.sort_by_key(|d| (d.mc_pos[1], d.mc_pos[2], d.mc_pos[0]));
    acc.raw_triggers.sort_by_key(|(p, _)| (p[1], p[2], p[0]));

    feat.doors = acc.doors;
    feat.door_block_names = acc.door_block_names;
    feat.door_voxels = acc.door_voxels.into_iter().collect();
    feat.flat_water_voxels = acc.flat_water_voxels.into_iter().collect();
    let mut raw_triggers = acc.raw_triggers;

    for pid in 0..grid.palette.len() {
        let bs = &grid.palette[pid];
        if bs.is_air {
            continue;
        }
        for bid in 0..num_biomes {
            let seen_idx = pid * num_biomes + bid;
            if (acc.seen_pid_biome[seen_idx >> 6] & (1u64 << (seen_idx & 63))) == 0 {
                continue;
            }
            let biome_id = grid
                .biome_palette
                .get(bid)
                .map(|s| s.as_str())
                .unwrap_or("minecraft:plains");
            match &models[pid] {
                ResolvedBlockModel::Elements(els) => {
                    for el in els {
                        for f in el.faces.values() {
                            let tint = effective_biome_tint(&bs.name, &f.texture, f.tint_index, biome_id);
                            requests.insert(TexRequestKey {
                                tex_ref: f.texture.clone(),
                                tint_rgb: tint,
                            });
                        }
                    }
                }
                ResolvedBlockModel::Obj(tris) => {
                    for t in tris {
                        requests.insert(TexRequestKey {
                            tex_ref: t.texture.clone(),
                            tint_rgb: None,
                        });
                    }
                }
                ResolvedBlockModel::Empty => {}
            }
        }
    }

    // Link buttons & levers within 2 blocks of doors/fence_gates and exclude linked ones from static mesh
    for ([tx, ty, tz], mut btn) in raw_triggers.drain(..) {
        let mut linked = Vec::new();
        for (idx, door) in feat.doors.iter().enumerate() {
            let [dx, dy, dz] = door.mc_pos;
            let dist_low = (tx - dx).abs().max((ty - dy).abs()).max((tz - dz).abs());
            let dist_high = (tx - dx).abs().max((ty - (dy + 1)).abs()).max((tz - dz).abs());
            if dist_low.min(dist_high) <= 2 {
                linked.push(format!("mc_door_{}_{}", idx, door.short_type));
            }
        }
        if !linked.is_empty() {
            btn.target_ids = linked;
            feat.buttons.push(btn);
            feat.door_voxels.insert((tx, ty, tz));
        }
    }

    // Build compact O(1) bitset for door_voxels + pure-water flat_water_voxels
    // (waterlogged plants/blocks in flat_water_voxels keep their own block mesh and only skip their water overlay)
    let total_voxels = grid.blocks.len();
    if !feat.door_voxels.is_empty() || !feat.flat_water_voxels.is_empty() {
        let mut bits = vec![0u64; total_voxels.div_ceil(64)];
        for &(vx, vy, vz) in &feat.door_voxels {
            if let Some(idx) = grid.index(vx, vy, vz) {
                bits[idx >> 6] |= 1u64 << (idx & 63);
            }
        }
        for &(vx, vy, vz) in &feat.flat_water_voxels {
            if let Some(idx) = grid.index(vx, vy, vz) {
                let pid = grid.blocks[idx] as usize;
                if grid.palette[pid].is_water_tag {
                    bits[idx >> 6] |= 1u64 << (idx & 63);
                }
            }
        }
        feat.skip_mesh_bits = bits;
    }

    // Sort lights by priority (0=torches/lanterns/campfires first, 1=magma/crying_obsidian, 2=lava)
    // and descending Y so surface lights are never crowded out by underground lava/magma.
    acc.raw_lights
        .sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)).then_with(|| a.3.cmp(&b.3)));

    let total_light_candidates = acc.raw_lights.len();
    let cell_size = (6.5 * scale).max(1e-3);
    let inv_cell = 1.0 / cell_size;
    let mut light_grid: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
    let mut prio1_count = 0usize;
    let mut prio2_count = 0usize;

    for (prio, wy, _wz, _wx, lx, ly, lz, col, br) in acc.raw_lights {
        if feat.lights.len() >= 1200 {
            eprintln!(
                "[Mesher] Warning: block light budget reached (1200); dropping {} lower-priority \
                 light candidates (underground lava/magma drops first)",
                total_light_candidates - feat.lights.len()
            );
            break;
        }
        if prio == 1 && prio1_count >= 250 {
            continue;
        }
        if prio == 2 && (prio2_count >= 200 || (wy < 0 && feat.lights.len() >= 900)) {
            continue;
        }
        let (min_dist, dist50, dist0) = match prio {
            0 => (2.0 * scale, (6.0 * scale) as i32, (14.0 * scale) as i32),
            1 => (5.0 * scale, (4.0 * scale) as i32, (9.0 * scale) as i32),
            _ => (6.5 * scale, (4.5 * scale) as i32, (10.5 * scale) as i32),
        };
        let min_dist_sq = min_dist * min_dist;
        let cx = (lx * inv_cell).floor() as i32;
        let cy = (ly * inv_cell).floor() as i32;
        let cz = (lz * inv_cell).floor() as i32;
        let mut too_close = false;
        'cell_scan: for ddx in -1..=1 {
            for ddy in -1..=1 {
                for ddz in -1..=1 {
                    if let Some(bucket) = light_grid.get(&(cx + ddx, cy + ddy, cz + ddz)) {
                        for &li in bucket {
                            let existing = &feat.lights[li];
                            let dx = existing.pos[0] - lx;
                            let dy = existing.pos[1] - ly;
                            let dz = existing.pos[2] - lz;
                            if dx * dx + dy * dy + dz * dz < min_dist_sq {
                                too_close = true;
                                break 'cell_scan;
                            }
                        }
                    }
                }
            }
        }
        if !too_close {
            let idx = feat.lights.len();
            light_grid.entry((cx, cy, cz)).or_default().push(idx);
            feat.lights.push(BlockLight {
                pos: [lx, ly, lz],
                color: col,
                brightness: br,
                dist50,
                dist0,
            });
            if prio == 1 {
                prio1_count += 1;
            } else if prio == 2 {
                prio2_count += 1;
            }
        }
    }

    // Discover 4-8 safe open-sky 3x3 walkable spawn candidates inside region
    if let Some(sp) = grid.player_spawn_mc {
        if grid
            .coords
            .contains(sp[0].floor() as i32, sp[1].floor() as i32, sp[2].floor() as i32)
        {
            feat.spawn_points_mc.push(sp);
        }
    }
    discover_safe_spawn_points(grid, &mut feat.spawn_points_mc);

    (feat, requests)
}

fn is_unsafe_spawn_block(bs: &BlockState) -> bool {
    let s = bs.short_name();
    is_leaves_name(s)
        || bs.is_water_tag
        || is_lava_name(s)
        || s.contains("magma")
        || s == "cactus"
        || s == "fire"
        || s == "soul_fire"
        || s == "campfire"
        || s == "soul_campfire"
        || s == "powder_snow"
        || s.contains("berry_bush")
        || s == "wither_rose"
        || s == "cobweb"
        || s == "pointed_dripstone"
}

fn is_safe_headroom_block(bs: &BlockState) -> bool {
    if bs.is_air {
        return true;
    }
    if is_unsafe_spawn_block(bs) {
        return false;
    }
    !is_full_opaque_occluder(bs)
}

fn discover_safe_spawn_points(grid: &VoxelGrid, out: &mut Vec<[f64; 3]>) {
    let c = grid.coords;
    let sx = c.size_x();
    let sy = c.size_y();
    let sz = c.size_z();
    if sx == 0 || sy < 3 || sz == 0 {
        return;
    }
    let slice_sz = sx * sz;
    let pal_occ: Vec<bool> = grid.palette.iter().map(is_full_opaque_occluder).collect();
    let pal_unsafe: Vec<bool> = grid.palette.iter().map(is_unsafe_spawn_block).collect();
    let pal_headroom: Vec<bool> = grid.palette.iter().map(is_safe_headroom_block).collect();

    let is_headroom = |gx: usize, gy: usize, gz: usize| -> bool {
        let pid = grid.blocks[gy * slice_sz + gz * sx + gx] as usize;
        pal_headroom[pid]
    };
    let is_safe_floor = |gx: usize, gy: usize, gz: usize| -> bool {
        let pid = grid.blocks[gy * slice_sz + gz * sx + gx] as usize;
        !pal_unsafe[pid]
    };

    let min_sep_sq = 1.5 * 1.5; // ~48-60 HU at default scale
    let push_if_far = |out: &mut Vec<[f64; 3]>, pt: [f64; 3]| {
        if out.len() >= 8 {
            return;
        }
        let far = out.iter().all(|p| {
            (p[0] - pt[0]).powi(2) + (p[1] - pt[1]).powi(2) + (p[2] - pt[2]).powi(2) >= min_sep_sq
        });
        if far {
            out.push(pt);
        }
    };

    // Scan columns in expanding Chebyshev rings from the center outward, top-down per column.
    // Open-sky 3x3 candidates found near the center are accepted immediately.
    let cx_mid = sx / 2;
    let cz_mid = sz / 2;
    let max_r = sx.max(sz);
    let mut fallback_1x1: Vec<[f64; 3]> = Vec::new();
    let mut fallback_interior_3x3: Vec<[f64; 3]> = Vec::new();

    for r in 0..max_r {
        if out.len() >= 8 {
            break;
        }
        let gx_lo = cx_mid.saturating_sub(r);
        let gx_hi = (cx_mid + r).min(sx - 1);
        let gz_lo = cz_mid.saturating_sub(r);
        let gz_hi = (cz_mid + r).min(sz - 1);

        for gz in gz_lo..=gz_hi {
            if out.len() >= 8 {
                break;
            }
            for gx in gx_lo..=gx_hi {
                if r > 0 && gx != gx_lo && gx != gx_hi && gz != gz_lo && gz != gz_hi {
                    continue;
                }
                let wx = c.min_x + gx as i32;
                let wz = c.min_z + gz as i32;
                let mut seen_opaque_above = false;

                for gy in (0..sy - 2).rev() {
                    let pid = grid.blocks[gy * slice_sz + gz * sx + gx] as usize;
                    if !pal_occ[pid] {
                        continue;
                    }
                    let sky_open = !seen_opaque_above;
                    seen_opaque_above = true;

                    if pal_unsafe[pid] {
                        continue;
                    }
                    if !is_headroom(gx, gy + 1, gz) || !is_headroom(gx, gy + 2, gz) {
                        continue;
                    }
                    let mut clear_3x3 = true;
                    'nb: for dx in -1isize..=1 {
                        let nx = gx as isize + dx;
                        if nx < 0 || nx >= sx as isize {
                            continue;
                        }
                        for dz in -1isize..=1 {
                            let nz = gz as isize + dz;
                            if nz < 0 || nz >= sz as isize {
                                continue;
                            }
                            let ux = nx as usize;
                            let uz = nz as usize;
                            if !is_headroom(ux, gy + 1, uz)
                                || !is_headroom(ux, gy + 2, uz)
                                || !is_safe_floor(ux, gy, uz)
                            {
                                clear_3x3 = false;
                                break 'nb;
                            }
                        }
                    }

                    let wy = c.min_y + gy as i32;
                    let pt = [wx as f64 + 0.5, wy as f64 + 1.1, wz as f64 + 0.5];
                    let on_edge = (sx > 2 && (gx == 0 || gx + 1 == sx)) || (sz > 2 && (gz == 0 || gz + 1 == sz));

                    if clear_3x3 && sky_open && !on_edge {
                        push_if_far(out, pt);
                        if out.len() >= 8 {
                            break;
                        }
                    } else if clear_3x3 && fallback_interior_3x3.len() < 32 {
                        fallback_interior_3x3.push(pt);
                    } else if fallback_1x1.len() < 32 {
                        fallback_1x1.push(pt);
                    }
                }
            }
        }
    }

    for pt in fallback_interior_3x3.into_iter().chain(fallback_1x1) {
        if out.len() >= 8 {
            break;
        }
        push_if_far(out, pt);
    }
}

fn compute_fence_gate_geometry(
    wx: i32,
    wy: i32,
    wz: i32,
    facing: &str,
    open: bool,
    origin_mc: [f64; 3],
    scale: f64,
) -> ([f64; 6], [f64; 3], u32) {
    let (mx0, mz0, mx1, mz1, px, pz) = if facing == "north" || facing == "south" {
        if open {
            (0.0, 0.375, 0.25, 1.0, 0.125, 0.5)
        } else {
            (0.0, 0.375, 1.0, 0.625, 0.0, 0.5)
        }
    } else if open {
        (0.375, 0.0, 1.0, 0.25, 0.5, 0.125)
    } else {
        (0.375, 0.0, 0.625, 1.0, 0.5, 0.0)
    };
    let p0 = mc_point_to_source([wx as f64 + mx0, wy as f64, wz as f64 + mz0], origin_mc, scale);
    let p1 = mc_point_to_source([wx as f64 + mx1, wy as f64 + 1.0, wz as f64 + mz1], origin_mc, scale);
    let box_coords = [
        p0[0].min(p1[0]) as f64,
        p0[1].min(p1[1]) as f64,
        p0[2].min(p1[2]) as f64,
        p0[0].max(p1[0]) as f64,
        p0[1].max(p1[1]) as f64,
        p0[2].max(p1[2]) as f64,
    ];
    let piv = mc_point_to_source([wx as f64 + px, wy as f64 + 0.5, wz as f64 + pz], origin_mc, scale);
    let spawnflags = if open { 306 } else { 304 };
    (box_coords, [piv[0] as f64, piv[1] as f64, piv[2] as f64], spawnflags)
}

#[allow(clippy::too_many_arguments)]
fn compute_door_geometry(
    wx: i32,
    wy: i32,
    wz: i32,
    height: f64,
    facing: &str,
    hinge: &str,
    open: bool,
    origin_mc: [f64; 3],
    scale: f64,
) -> ([f64; 6], [f64; 3], u32) {
    let thick = 3.0 / 16.0;
    let ht = thick / 2.0;
    let (px, pz) = match facing {
        "south" => (if hinge == "left" { 1.0 } else { 0.0 }, ht),
        "west" => (1.0 - ht, if hinge == "left" { 1.0 } else { 0.0 }),
        "east" => (ht, if hinge == "left" { 0.0 } else { 1.0 }),
        _ => (if hinge == "left" { 0.0 } else { 1.0 }, 1.0 - ht),
    };
    let eff_facing = if !open {
        facing
    } else if hinge == "left" {
        match facing {
            "north" => "east",
            "east" => "south",
            "south" => "west",
            _ => "north",
        }
    } else {
        match facing {
            "north" => "west",
            "west" => "south",
            "south" => "east",
            _ => "north",
        }
    };
    let (mx0, mz0, mx1, mz1) = match eff_facing {
        "south" => (0.0, 0.0, 1.0, thick),
        "west" => (1.0 - thick, 0.0, 1.0, 1.0),
        "east" => (0.0, 0.0, thick, 1.0),
        _ => (0.0, 1.0 - thick, 1.0, 1.0),
    };

    let p0 = mc_point_to_source([wx as f64 + mx0, wy as f64, wz as f64 + mz0], origin_mc, scale);
    let p1 = mc_point_to_source(
        [wx as f64 + mx1, wy as f64 + height, wz as f64 + mz1],
        origin_mc,
        scale,
    );
    let box_coords = [
        p0[0].min(p1[0]) as f64,
        p0[1].min(p1[1]) as f64,
        p0[2].min(p1[2]) as f64,
        p0[0].max(p1[0]) as f64,
        p0[1].max(p1[1]) as f64,
        p0[2].max(p1[2]) as f64,
    ];
    let piv = mc_point_to_source(
        [wx as f64 + px, wy as f64 + height * 0.5, wz as f64 + pz],
        origin_mc,
        scale,
    );
    // Source Engine func_door_rotating: bit 2 (Reverse Dir) = clockwise (-90 deg around +Z),
    // bit 2 = 0 = counter-clockwise (+90 deg around +Z). With mc_z -> -src_y, a closed left-hinged
    // door must swing CCW (0) into the block, and a closed right-hinged door swings CW (2).
    let reverse_flag = if (hinge == "right") ^ open { 2 } else { 0 };
    let spawnflags = 304 | reverse_flag;
    (box_coords, [piv[0] as f64, piv[1] as f64, piv[2] as f64], spawnflags)
}

fn is_full_block_light_emitter(s: &str) -> bool {
    matches!(
        s,
        "glowstone"
            | "sea_lantern"
            | "shroomlight"
            | "jack_o_lantern"
            | "redstone_lamp"
            | "beacon"
            | "crying_obsidian"
            | "magma_block"
    ) || s.ends_with("_froglight")
        || s == "froglight"
}

fn block_light_spec(bs: &BlockState) -> Option<(&'static str, i32, u8)> {
    let s = bs.short_name();
    match s {
        "torch" | "wall_torch" => Some(("255 202 125", 380, 0)),
        "soul_torch" | "soul_wall_torch" => Some(("115 225 255", 280, 0)),
        "lantern" => Some(("255 208 135", 400, 0)),
        "soul_lantern" => Some(("115 225 255", 300, 0)),
        "glowstone" | "shroomlight" | "jack_o_lantern" => Some(("255 215 140", 380, 0)),
        "sea_lantern" | "end_rod" => Some(("225 245 255", 340, 0)),
        "redstone_lamp" if bs.prop("lit") == Some("true") => Some(("255 200 125", 360, 0)),
        "campfire" if bs.prop("lit") != Some("false") => Some(("255 168 85", 420, 0)),
        "soul_campfire" if bs.prop("lit") != Some("false") => Some(("100 225 255", 340, 0)),
        "beacon" => Some(("190 250 255", 380, 0)),
        "ochre_froglight" | "verdant_froglight" | "pearlescent_froglight" | "froglight" => {
            Some(("255 230 195", 340, 0))
        }
        "copper_torch" | "copper_wall_torch" => Some(("140 255 185", 340, 0)),
        _ if s == "copper_lantern" || s.ends_with("_copper_lantern") => {
            Some(("145 255 190", 360, 0))
        }
        _ if (s == "copper_bulb" || s.ends_with("_copper_bulb"))
            && bs.prop("lit") == Some("true") =>
        {
            if s.contains("oxidized") {
                Some(("175 155 110", 130, 0))
            } else if s.contains("weathered") {
                Some(("215 170 115", 210, 0))
            } else if s.contains("exposed") {
                Some(("245 180 120", 290, 0))
            } else {
                Some(("255 190 120", 360, 0))
            }
        }
        "firefly_bush" => Some(("220 255 140", 140, 1)),
        "open_eyeblossom" => Some(("255 150 60", 120, 1)),
        "crying_obsidian" => Some(("160 80 255", 130, 1)),
        "magma_block" => Some(("255 125 45", 140, 1)),
        "lava" | "flowing_lava" => Some(("255 125 45", 180, 2)),
        _ if (s.ends_with("_torch") || s.ends_with("_lantern") || s.ends_with("_lamp"))
            && bs.prop("lit") != Some("false") =>
        {
            Some(("255 205 130", 340, 0))
        }
        _ => None,
    }
}

#[inline]
fn wrap_obj_uv(v: f32) -> f32 {
    if (v - 1.0).abs() < 1e-5 {
        1.0
    } else if (0.0..=1.0).contains(&v) {
        v
    } else {
        v.rem_euclid(1.0)
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_chunk_open_air_origin(
    grid: &VoxelGrid,
    palette_occluder: &[bool],
    x0: usize,
    x1: usize,
    z0: usize,
    z1: usize,
    origin_mc: [f64; 3],
    scale: f64,
) -> [f64; 3] {
    let c = grid.coords;
    let sx = c.size_x();
    let sy = c.size_y();
    let sz = c.size_z();

    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_z = 0.0f64;
    let mut count = 0usize;

    for gz in z0..z1 {
        let wz = c.min_z + gz as i32;
        for gx in x0..x1 {
            let wx = c.min_x + gx as i32;
            for gy in (0..sy).rev() {
                let wy = c.min_y + gy as i32;
                let pid = grid.blocks[(gy * sz + gz) * sx + gx] as usize;
                if !grid.palette[pid].is_air && palette_occluder[pid] {
                    let ab1 = grid.get_block_id(wx, wy + 1, wz) as usize;
                    let ab2 = grid.get_block_id(wx, wy + 2, wz) as usize;
                    if !palette_occluder[ab1] && !palette_occluder[ab2] {
                        sum_x += wx as f64 + 0.5;
                        sum_y += wy as f64 + 1.5;
                        sum_z += wz as f64 + 0.5;
                        count += 1;
                    }
                    break;
                }
            }
        }
    }

    let (mx, my, mz) = if count > 0 {
        (sum_x / count as f64, sum_y / count as f64, sum_z / count as f64)
    } else {
        // Fallback for chunks without full opaque occluders (e.g. pure water/leaves/glass):
        // average actual non-air voxels in the chunk so the origin stays strictly inside the skybox.
        let mut na_x = 0.0f64;
        let mut na_y = 0.0f64;
        let mut na_z = 0.0f64;
        let mut na_cnt = 0usize;
        for gz in z0..z1 {
            let wz = c.min_z + gz as i32;
            for gx in x0..x1 {
                let wx = c.min_x + gx as i32;
                for gy in 0..sy {
                    let pid = grid.blocks[(gy * sz + gz) * sx + gx] as usize;
                    if !grid.palette[pid].is_air {
                        na_x += wx as f64 + 0.5;
                        na_y += (c.min_y + gy as i32) as f64 + 0.5;
                        na_z += wz as f64 + 0.5;
                        na_cnt += 1;
                    }
                }
            }
        }
        if na_cnt > 0 {
            (na_x / na_cnt as f64, na_y / na_cnt as f64, na_z / na_cnt as f64)
        } else {
            (
                c.min_x as f64 + ((x0 + x1) as f64) * 0.5,
                c.min_y as f64 + 0.5,
                c.min_z as f64 + ((z0 + z1) as f64) * 0.5,
            )
        }
    };
    let s = mc_point_to_source([mx, my, mz], origin_mc, scale);
    [s[0] as f64, s[1] as f64, s[2] as f64]
}

#[derive(Clone)]
struct BakedBiomeFace {
    uvs: [[f32; 2]; 4],
    mat_name: Cow<'static, str>,
    is_translucent: bool,
    is_cutout: bool,
}

#[derive(Clone)]
struct BakedFace {
    world_dir: FaceDir,
    cull_dir: Option<FaceDir>,
    cull_off: [i32; 3],
    corners_mc: [[f32; 3]; 4],
    corners_mc_full_top: Option<[[f32; 3]; 4]>,
    norm_src: [f32; 3],
    per_biome: Vec<BakedBiomeFace>,
    is_water_overlay: bool,
}

#[derive(Clone)]
enum BakedModel {
    Faces {
        faces: Vec<BakedFace>,
        is_fluid: bool,
        is_water: bool,
        has_water_overlay: bool,
    },
    Obj(Vec<ObjTri>),
    Empty,
}

fn has_full_horizontal_face(faces: &[BakedFace], dir: FaceDir) -> bool {
    faces.iter().any(|bf| {
        if bf.world_dir != dir {
            return false;
        }
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_z = f32::INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for p in &bf.corners_mc {
            min_x = min_x.min(p[0]);
            max_x = max_x.max(p[0]);
            min_z = min_z.min(p[2]);
            max_z = max_z.max(p[2]);
            match dir {
                FaceDir::Up if p[1] < 0.99 => return false,
                FaceDir::Down if p[1] > 0.01 => return false,
                FaceDir::Up | FaceDir::Down => {}
                _ => return false,
            }
        }
        min_x <= 0.01 && max_x >= 0.99 && min_z <= 0.01 && max_z >= 0.99
    })
}

fn append_water_overlay_faces(baked_faces: &mut Vec<BakedFace>) {
    const TOP_Y: f32 = 14.2 / 16.0;
    const WATER_UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

    let skip_top = has_full_horizontal_face(baked_faces, FaceDir::Up);
    let skip_down = has_full_horizontal_face(baked_faces, FaceDir::Down);

    for dir in [
        FaceDir::Up,
        FaceDir::Down,
        FaceDir::North,
        FaceDir::South,
        FaceDir::East,
        FaceDir::West,
    ] {
        if (dir == FaceDir::Up && skip_top) || (dir == FaceDir::Down && skip_down) {
            continue;
        }
        let (corners_mc, corners_mc_full_top) = match dir {
            FaceDir::Up => (
                [
                    [0.0, TOP_Y, 1.0],
                    [1.0, TOP_Y, 1.0],
                    [1.0, TOP_Y, 0.0],
                    [0.0, TOP_Y, 0.0],
                ],
                None,
            ),
            FaceDir::Down => (
                [
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0],
                ],
                None,
            ),
            FaceDir::North => (
                [
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                    [0.0, TOP_Y, 0.0],
                    [1.0, TOP_Y, 0.0],
                ],
                Some([
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0],
                ]),
            ),
            FaceDir::South => (
                [
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 1.0],
                    [1.0, TOP_Y, 1.0],
                    [0.0, TOP_Y, 1.0],
                ],
                Some([
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 1.0],
                    [1.0, 1.0, 1.0],
                    [0.0, 1.0, 1.0],
                ]),
            ),
            FaceDir::East => (
                [
                    [1.0, 0.0, 1.0],
                    [1.0, 0.0, 0.0],
                    [1.0, TOP_Y, 0.0],
                    [1.0, TOP_Y, 1.0],
                ],
                Some([
                    [1.0, 0.0, 1.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 1.0, 0.0],
                    [1.0, 1.0, 1.0],
                ]),
            ),
            FaceDir::West => (
                [
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [0.0, TOP_Y, 1.0],
                    [0.0, TOP_Y, 0.0],
                ],
                Some([
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [0.0, 1.0, 1.0],
                    [0.0, 1.0, 0.0],
                ]),
            ),
        };
        let p0 = [corners_mc[0][0], -corners_mc[0][2], corners_mc[0][1]];
        let p1 = [corners_mc[1][0], -corners_mc[1][2], corners_mc[1][1]];
        let p2 = [corners_mc[2][0], -corners_mc[2][2], corners_mc[2][1]];
        let norm_src = compute_quad_normal(p0, p1, p2);
        baked_faces.push(BakedFace {
            world_dir: dir,
            cull_dir: Some(dir),
            cull_off: dir.offset(),
            corners_mc,
            corners_mc_full_top,
            norm_src,
            per_biome: vec![BakedBiomeFace {
                uvs: WATER_UVS,
                mat_name: Cow::Borrowed("mc_water_flow"),
                is_translucent: true,
                is_cutout: false,
            }],
            is_water_overlay: true,
        });
    }
}

fn bake_element_corners(
    orig_dir: FaceDir,
    el: &ModelElement,
    f0: [f32; 3],
    f1: [f32; 3],
    emitted_planes: &mut Option<&mut Vec<(FaceDir, f32)>>,
    apply_zero_thick_offset: bool,
) -> [[f32; 3]; 4] {
    let mut local_corners = match orig_dir {
        FaceDir::Up => [
            [f0[0], f1[1], f1[2]],
            [f1[0], f1[1], f1[2]],
            [f1[0], f1[1], f0[2]],
            [f0[0], f1[1], f0[2]],
        ],
        FaceDir::Down => [
            [f0[0], f0[1], f0[2]],
            [f1[0], f0[1], f0[2]],
            [f1[0], f0[1], f1[2]],
            [f0[0], f0[1], f1[2]],
        ],
        FaceDir::North => [
            [f1[0], f0[1], f0[2]],
            [f0[0], f0[1], f0[2]],
            [f0[0], f1[1], f0[2]],
            [f1[0], f1[1], f0[2]],
        ],
        FaceDir::South => [
            [f0[0], f0[1], f1[2]],
            [f1[0], f0[1], f1[2]],
            [f1[0], f1[1], f1[2]],
            [f0[0], f1[1], f1[2]],
        ],
        FaceDir::East => [
            [f1[0], f0[1], f1[2]],
            [f1[0], f0[1], f0[2]],
            [f1[0], f1[1], f0[2]],
            [f1[0], f1[1], f1[2]],
        ],
        FaceDir::West => [
            [f0[0], f0[1], f0[2]],
            [f0[0], f0[1], f1[2]],
            [f0[0], f1[1], f1[2]],
            [f0[0], f1[1], f0[2]],
        ],
    };

    let opposite = face_dir_opposite(orig_dir);
    let is_zero_thick_pair = el.faces.contains_key(&opposite)
        && match orig_dir {
            FaceDir::Up | FaceDir::Down => (f1[1] - f0[1]).abs() < 1e-4,
            FaceDir::North | FaceDir::South => (f1[2] - f0[2]).abs() < 1e-4,
            FaceDir::East | FaceDir::West => (f1[0] - f0[0]).abs() < 1e-4,
        };
    if is_zero_thick_pair && apply_zero_thick_offset {
        let off = orig_dir.offset();
        let eps = 0.003_f32;
        for p in &mut local_corners {
            p[0] += off[0] as f32 * eps;
            p[1] += off[1] as f32 * eps;
            p[2] += off[2] as f32 * eps;
        }
    }

    let mut pm_corners = [[0.0f32; 3]; 4];
    for (i, &p_el) in local_corners.iter().enumerate() {
        let mut pm = p_el;
        if let Some(ref er) = el.rotation {
            if let Some([rx, ry, rz]) = er.euler_xyz {
                if rx.abs() > 1e-4 {
                    pm = rotate_element_point(pm, er.origin, 'x', rx, er.rescale);
                }
                if ry.abs() > 1e-4 {
                    pm = rotate_element_point(pm, er.origin, 'y', ry, er.rescale);
                }
                if rz.abs() > 1e-4 {
                    pm = rotate_element_point(pm, er.origin, 'z', rz, er.rescale);
                }
            } else {
                pm = rotate_element_point(pm, er.origin, er.axis, er.angle, er.rescale);
            }
        }
        pm_corners[i] = apply_blockstate_rot(pm, el.rot_x, el.rot_y, el.rot_z);
    }

    let world_dir = orig_dir.rotate_xyz(el.rot_x, el.rot_y, el.rot_z);
    if el.rotation.is_none() {
        if let Some(ref mut planes) = emitted_planes {
            let plane_coord = match world_dir {
                FaceDir::Up | FaceDir::Down => pm_corners[0][1],
                FaceDir::North | FaceDir::South => pm_corners[0][2],
                FaceDir::East | FaceDir::West => pm_corners[0][0],
            };
            if planes.iter().any(|&(d, c)| d == world_dir && (c - plane_coord).abs() < 1e-4) {
                let off = world_dir.offset();
                let eps = 0.002_f32;
                for pm in &mut pm_corners {
                    pm[0] += off[0] as f32 * eps;
                    pm[1] += off[1] as f32 * eps;
                    pm[2] += off[2] as f32 * eps;
                }
            } else {
                planes.push((world_dir, plane_coord));
            }
        }
    }
    pm_corners
}

fn bake_palette_models(
    grid: &VoxelGrid,
    models: &[ResolvedBlockModel],
    palette_occluder: &[bool],
    atlas: &TextureAtlas,
    animated_mats: &HashMap<TexRequestKey, (String, RenderBucket)>,
) -> Vec<BakedModel> {
    let biome_palette = &grid.biome_palette;
    let num_biomes = biome_palette.len().max(1);

    grid.palette
        .iter()
        .enumerate()
        .map(|(pid, bs)| {
            if bs.is_air {
                return BakedModel::Empty;
            }
            let has_water_overlay = !bs.is_water_tag && is_water_host(bs);
            let els = match models.get(pid) {
                Some(ResolvedBlockModel::Obj(tris)) => return BakedModel::Obj(tris.clone()),
                Some(ResolvedBlockModel::Elements(els)) => els.as_slice(),
                _ if has_water_overlay => &[],
                _ => return BakedModel::Empty,
            };
            let is_water = bs.is_water_tag;
            let is_fluid = is_water || is_lava_name(bs.short_name());
            let self_is_occluder = palette_occluder[pid];
            let track_planes = els.len() > 1;
            let mut emitted_planes: Vec<(FaceDir, f32)> = Vec::new();
            let mut baked_faces = Vec::new();

            for el in els {
                        let f0 = [el.from[0] / 16.0, el.from[1] / 16.0, el.from[2] / 16.0];
                        let f1 = [el.to[0] / 16.0, el.to[1] / 16.0, el.to[2] / 16.0];
                        let f1_full = [f1[0], 1.0, f1[2]];

                        for (&orig_dir, face) in &el.faces {
                            let world_dir = orig_dir.rotate_xyz(el.rot_x, el.rot_y, el.rot_z);
                            let cull_dir = if el.rotation.is_none()
                                && (face.cullface.is_some()
                                    || (self_is_occluder && is_full_face_rect(orig_dir, el.from, el.to)))
                            {
                                Some(
                                    face.cullface
                                        .map(|cd| cd.rotate_xyz(el.rot_x, el.rot_y, el.rot_z))
                                        .unwrap_or(world_dir),
                                )
                            } else {
                                None
                            };
                            let cull_off = cull_dir.map(|d| d.offset()).unwrap_or([0, 0, 0]);

                            let is_zero_thick_face = match orig_dir {
                                FaceDir::Up | FaceDir::Down => (f1[1] - f0[1]).abs() < 1e-4,
                                FaceDir::North | FaceDir::South => (f1[2] - f0[2]).abs() < 1e-4,
                                FaceDir::East | FaceDir::West => (f1[0] - f0[0]).abs() < 1e-4,
                            };
                            let opposite = face_dir_opposite(orig_dir);
                            let is_zero_thick_pair = is_zero_thick_face && el.faces.contains_key(&opposite);

                            // Check material bucket before emitting geometry so $nocull 1 cutout/translucent
                            // zero-thickness planes emit only a single non-flickering quad.
                            let first_biome = biome_palette.first().map(|s| s.as_str()).unwrap_or("minecraft:plains");
                            let first_tint = if is_water {
                                None
                            } else {
                                effective_biome_tint(&bs.name, &face.texture, face.tint_index, first_biome)
                            };
                            let first_key = TexRequestKey {
                                tex_ref: face.texture.clone(),
                                tint_rgb: first_tint,
                            };
                            let first_bucket = if is_water {
                                RenderBucket::TerrainTranslucent
                            } else if let Some((_, ab)) = animated_mats.get(&first_key).or_else(|| {
                                animated_mats.get(&TexRequestKey {
                                    tex_ref: face.texture.clone(),
                                    tint_rgb: None,
                                })
                            }) {
                                *ab
                            } else {
                                atlas.get_or_default(&first_key).bucket
                            };
                            let is_nocull_mat = matches!(
                                first_bucket,
                                RenderBucket::TerrainCutout | RenderBucket::TerrainTranslucent
                            ) || (is_zero_thick_pair
                                && first_bucket == RenderBucket::TerrainCutoutCull);
                            if is_zero_thick_pair
                                && is_nocull_mat
                                && matches!(orig_dir, FaceDir::South | FaceDir::West | FaceDir::Down)
                            {
                                continue;
                            }

                            let mut plane_opt = if track_planes {
                                Some(&mut emitted_planes)
                            } else {
                                None
                            };
                            let corners_mc = bake_element_corners(
                                orig_dir,
                                el,
                                f0,
                                f1,
                                &mut plane_opt,
                                !is_nocull_mat,
                            );
                            let corners_mc_full_top = if is_fluid && (f1[1] - 1.0).abs() > 1e-4 {
                                let mut no_planes: Option<&mut Vec<(FaceDir, f32)>> = None;
                                Some(bake_element_corners(orig_dir, el, f0, f1_full, &mut no_planes, false))
                            } else {
                                None
                            };

                            let p0 = [corners_mc[0][0], -corners_mc[0][2], corners_mc[0][1]];
                            let p1 = [corners_mc[1][0], -corners_mc[1][2], corners_mc[1][1]];
                            let p2 = [corners_mc[2][0], -corners_mc[2][2], corners_mc[2][1]];
                            let norm_src = if is_zero_thick_face
                                && matches!(
                                    first_bucket,
                                    RenderBucket::TerrainCutout | RenderBucket::TerrainCutoutCull
                                ) {
                                [0.0, 0.0, 1.0]
                            } else {
                                compute_quad_normal(p0, p1, p2)
                            };

                            let raw_uv = face
                                .uv
                                .unwrap_or_else(|| default_face_uv(orig_dir, el.from, el.to));
                            let u0 = (raw_uv[0] / 16.0).clamp(0.0, 1.0);
                            let v0 = (raw_uv[1] / 16.0).clamp(0.0, 1.0);
                            let u1 = (raw_uv[2] / 16.0).clamp(0.0, 1.0);
                            let v1 = (raw_uv[3] / 16.0).clamp(0.0, 1.0);
                            let base_uvs = [[u0, v1], [u1, v1], [u1, v0], [u0, v0]];
                            let uvlock_rot = if el.uvlock {
                                match orig_dir {
                                    FaceDir::Up => (360 - ((el.rot_y % 360 + 360) % 360)) % 360,
                                    FaceDir::Down => (el.rot_y % 360 + 360) % 360,
                                    _ if (el.rot_x % 360 + 360) % 360 == 180 => 180,
                                    _ => 0,
                                }
                            } else {
                                0
                            };
                            let face_steps = (((face.rotation % 360) + 360) % 360) / 90;
                            let f0_idx = (face_steps as usize) % 4;
                            let f1_idx = (1 + face_steps as usize) % 4;
                            let f2_idx = (2 + face_steps as usize) % 4;
                            let f3_idx = (3 + face_steps as usize) % 4;
                            let mut face_uvs = [
                                base_uvs[f0_idx],
                                base_uvs[f1_idx],
                                base_uvs[f2_idx],
                                base_uvs[f3_idx],
                            ];
                            if el.uvlock && uvlock_rot != 0 {
                                let uvlock_steps = (((uvlock_rot % 360) + 360) % 360) / 90;
                                for uv in &mut face_uvs {
                                    for _ in 0..uvlock_steps {
                                        let du = uv[0] - 0.5;
                                        let dv = uv[1] - 0.5;
                                        uv[0] = (0.5 + dv).clamp(0.0, 1.0);
                                        uv[1] = (0.5 - du).clamp(0.0, 1.0);
                                    }
                                }
                            }

                            if is_water {
                                let map_standalone = |u: f32, v: f32| -> [f32; 2] { [u, 1.0 - v] };
                                let water_uvs = [
                                    map_standalone(face_uvs[0][0], face_uvs[0][1]),
                                    map_standalone(face_uvs[1][0], face_uvs[1][1]),
                                    map_standalone(face_uvs[2][0], face_uvs[2][1]),
                                    map_standalone(face_uvs[3][0], face_uvs[3][1]),
                                ];
                                baked_faces.push(BakedFace {
                                    world_dir,
                                    cull_dir,
                                    cull_off,
                                    corners_mc,
                                    corners_mc_full_top,
                                    norm_src,
                                    per_biome: vec![BakedBiomeFace {
                                        uvs: water_uvs,
                                        mat_name: Cow::Borrowed("mc_water_flow"),
                                        is_translucent: true,
                                        is_cutout: false,
                                    }],
                                    is_water_overlay: false,
                                });
                            } else {
                                let mut per_biome = Vec::with_capacity(num_biomes);
                                for bid in 0..num_biomes {
                                    let biome_id = biome_palette
                                        .get(bid)
                                        .map(|s| s.as_str())
                                        .unwrap_or("minecraft:plains");
                                    let tint = effective_biome_tint(
                                        &bs.name,
                                        &face.texture,
                                        face.tint_index,
                                        biome_id,
                                    );
                                    let req_key = TexRequestKey {
                                        tex_ref: face.texture.clone(),
                                        tint_rgb: tint,
                                    };
                                    let anim_hit = animated_mats.get(&req_key).or_else(|| {
                                        animated_mats.get(&TexRequestKey {
                                            tex_ref: face.texture.clone(),
                                            tint_rgb: None,
                                        })
                                    });
                                    if let Some((anim_name, anim_bucket)) = anim_hit {
                                        let map_standalone = |u: f32, v: f32| -> [f32; 2] { [u, 1.0 - v] };
                                        per_biome.push(BakedBiomeFace {
                                            uvs: [
                                                map_standalone(face_uvs[0][0], face_uvs[0][1]),
                                                map_standalone(face_uvs[1][0], face_uvs[1][1]),
                                                map_standalone(face_uvs[2][0], face_uvs[2][1]),
                                                map_standalone(face_uvs[3][0], face_uvs[3][1]),
                                            ],
                                            mat_name: Cow::Owned(anim_name.clone()),
                                            is_translucent: *anim_bucket == RenderBucket::TerrainTranslucent,
                                            is_cutout: matches!(
                                                *anim_bucket,
                                                RenderBucket::TerrainCutout | RenderBucket::TerrainCutoutCull
                                            ),
                                        });
                                    } else {
                                        let uv_rect = atlas.get_or_default(&req_key);
                                        let eff_bucket = if is_zero_thick_pair
                                            && uv_rect.bucket == RenderBucket::TerrainCutoutCull
                                        {
                                            RenderBucket::TerrainCutout
                                        } else {
                                            uv_rect.bucket
                                        };
                                        per_biome.push(BakedBiomeFace {
                                            uvs: [
                                                uv_rect.map_uv(face_uvs[0][0], face_uvs[0][1]),
                                                uv_rect.map_uv(face_uvs[1][0], face_uvs[1][1]),
                                                uv_rect.map_uv(face_uvs[2][0], face_uvs[2][1]),
                                                uv_rect.map_uv(face_uvs[3][0], face_uvs[3][1]),
                                            ],
                                            mat_name: Cow::Borrowed(eff_bucket.material_name()),
                                            is_translucent: eff_bucket == RenderBucket::TerrainTranslucent,
                                            is_cutout: matches!(
                                                eff_bucket,
                                                RenderBucket::TerrainCutout | RenderBucket::TerrainCutoutCull
                                            ),
                                        });
                                    }
                                }
                                baked_faces.push(BakedFace {
                                    world_dir,
                                    cull_dir,
                                    cull_off,
                                    corners_mc,
                                    corners_mc_full_top,
                                    norm_src,
                                    per_biome,
                                    is_water_overlay: false,
                                });
                            }
                        }
                    }

                    if has_water_overlay {
                        append_water_overlay_faces(&mut baked_faces);
                    }

                    BakedModel::Faces {
                        faces: baked_faces,
                        is_fluid,
                        is_water,
                        has_water_overlay,
                    }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn mesh_world_chunks_with_chunk_size(
    grid: &VoxelGrid,
    models: &[ResolvedBlockModel],
    atlas: &TextureAtlas,
    sign_atlas: &SignAtlas,
    features: &ExtractedFeatures,
    origin_mc: [f64; 3],
    scale: f64,
    chunk_size: usize,
) -> Vec<ChunkMesh> {
    stream_world_chunks_with_animated(
        grid,
        models,
        atlas,
        sign_atlas,
        features,
        origin_mc,
        scale,
        chunk_size,
        &HashMap::new(),
        Ok,
    )
    .unwrap_or_default()
}

#[inline]
fn fluid_corner_at(corners: [f32; 4], x: f32, z: f32) -> f32 {
    match (x >= 0.5, z >= 0.5) {
        (false, false) => corners[0], // NW
        (true, false) => corners[1],  // NE
        (true, true) => corners[2],   // SE
        (false, true) => corners[3],  // SW
    }
}

fn compute_fluid_block_corners(
    grid: &VoxelGrid,
    cull_info: &[PaletteCullInfo],
    wx: i32,
    wy: i32,
    wz: i32,
    is_water: bool,
) -> [f32; 4] {
    let same_above_flag = if is_water { CULL_WATER_HOST } else { CULL_LAVA };
    let ab_pid = grid.get_block_id(wx, wy + 1, wz) as usize;
    if (cull_info[ab_pid].flags & same_above_flag) != 0 {
        return [1.0, 1.0, 1.0, 1.0];
    }

    let same_pure_flag = if is_water { CULL_WATER } else { CULL_LAVA };
    let corner_offsets = [(0, 0), (1, 0), (1, 1), (0, 1)];
    let mut out = [8.0 / 9.0; 4];

    for (ci, &(ox, oz)) in corner_offsets.iter().enumerate() {
        let vx = wx + ox;
        let vz = wz + oz;
        let mut sum = 0.0f32;
        let mut weight = 0.0f32;
        for &(nx, nz) in &[
            (vx - 1, vz - 1),
            (vx, vz - 1),
            (vx - 1, vz),
            (vx, vz),
        ] {
            let npid = grid.get_block_id(nx, wy, nz) as usize;
            let ninfo = cull_info[npid];
            if (ninfo.flags & same_pure_flag) != 0 {
                let lvl = ninfo.fluid_level;
                let h = if lvl == 0 || lvl >= 8 {
                    8.0 / 9.0
                } else {
                    (8.0 - lvl as f32) / 9.0
                };
                if h >= 0.8 {
                    sum += h * 10.0;
                    weight += 10.0;
                } else {
                    sum += h;
                    weight += 1.0;
                }
            } else if is_water && (ninfo.flags & CULL_WATER_HOST) != 0 {
                let h = 14.2 / 16.0;
                sum += h * 10.0;
                weight += 10.0;
            } else if (ninfo.flags & CULL_OCCLUDER) == 0 {
                let sname = grid.palette[npid].short_name();
                if !sname.ends_with("_slab") && !sname.ends_with("_stairs") {
                    weight += 1.0;
                }
            }
        }
        out[ci] = if weight > 0.0 {
            (sum / weight).clamp(0.05, 1.0)
        } else {
            8.0 / 9.0
        };
    }

    out
}

#[allow(clippy::too_many_arguments)]
pub fn stream_world_chunks_with_animated<T: Send, F>(
    grid: &VoxelGrid,
    models: &[ResolvedBlockModel],
    atlas: &TextureAtlas,
    sign_atlas: &SignAtlas,
    features: &ExtractedFeatures,
    origin_mc: [f64; 3],
    scale: f64,
    chunk_size: usize,
    animated_mats: &HashMap<TexRequestKey, (String, RenderBucket)>,
    emit_chunk: F,
) -> Result<Vec<T>, String>
where
    F: Fn(ChunkMesh) -> Result<T, String> + Sync,
{
    use rayon::prelude::*;

    let cs = chunk_size.max(1);
    let c = grid.coords;
    let sx = c.size_x();
    let sy = c.size_y();
    let sz = c.size_z();
    let ncx = sx.div_ceil(cs);
    let ncz = sz.div_ceil(cs);

    let mut chunk_coords = Vec::with_capacity(ncx * ncz);
    for cz_i in 0..ncz {
        for cx_i in 0..ncx {
            chunk_coords.push((cx_i as i32, cz_i as i32));
        }
    }

    let palette_occluder: Vec<bool> = grid
        .palette
        .iter()
        .enumerate()
        .map(|(pid, bs)| {
            is_full_opaque_occluder(bs)
                && models.get(pid).map(model_has_full_cube).unwrap_or(false)
        })
        .collect();
    let palette_is_sign: Vec<bool> = grid
        .palette
        .iter()
        .map(|bs| bs.short_name().ends_with("_sign"))
        .collect();
    let cull_info = build_palette_cull_info(&grid.palette, &palette_occluder);
    let baked_models = bake_palette_models(grid, models, &palette_occluder, atlas, animated_mats);
    let has_skip_bits = !features.skip_mesh_bits.is_empty();
    let flat_water_bits: Vec<u64> = if !features.flat_water_voxels.is_empty() {
        let mut bits = vec![0u64; grid.blocks.len().div_ceil(64)];
        for &(vx, vy, vz) in &features.flat_water_voxels {
            if let Some(idx) = grid.index(vx, vy, vz) {
                bits[idx >> 6] |= 1u64 << (idx & 63);
            }
        }
        bits
    } else {
        Vec::new()
    };
    let has_flat_water_bits = !flat_water_bits.is_empty();
    let ox = origin_mc[0];
    let oy = origin_mc[1];
    let oz = origin_mc[2];

    chunk_coords
        .into_par_iter()
        .filter_map(|(cx_i, cz_i)| {
            let x0 = (cx_i as usize) * cs;
            let x1 = (x0 + cs).min(sx);
            let z0 = (cz_i as usize) * cs;
            let z1 = (z0 + cs).min(sz);

            let origin_source = compute_chunk_open_air_origin(grid, &palette_occluder, x0, x1, z0, z1, origin_mc, scale);

            let mut solid_tris = Vec::new();
            let mut translucent_tris = Vec::new();
            let mut has_cutout = false;
            let mut bmin = [f64::INFINITY; 3];
            let mut bmax = [f64::NEG_INFINITY; 3];

            for gy in 0..sy {
                let wy = c.min_y + gy as i32;
                for gz in z0..z1 {
                    let wz = c.min_z + gz as i32;
                    let row = (gy * sz + gz) * sx;
                    for gx in x0..x1 {
                        let v_idx = row + gx;
                        let pid = grid.blocks[v_idx] as usize;
                        if pid == grid.air_id as usize || grid.palette[pid].is_air {
                            continue;
                        }
                        if has_skip_bits && ((features.skip_mesh_bits[v_idx >> 6] >> (v_idx & 63)) & 1) != 0 {
                            continue;
                        }
                        let in_flat_water = has_flat_water_bits
                            && ((flat_water_bits[v_idx >> 6] >> (v_idx & 63)) & 1) != 0;
                        let wx = c.min_x + gx as i32;

                        // Special geometry for sign text panels
                        if palette_is_sign[pid] {
                            emit_sign_geometry(
                                &grid.palette[pid],
                                wx,
                                wy,
                                wz,
                                sign_atlas,
                                atlas,
                                origin_mc,
                                scale,
                                &mut solid_tris,
                                &mut bmin,
                                &mut bmax,
                            );
                            continue;
                        }

                        match &baked_models[pid] {
                            BakedModel::Faces {
                                faces,
                                is_fluid,
                                is_water,
                                has_water_overlay,
                            } => {
                                let self_cull = cull_info[pid];
                                let same_fluid_above = if *has_water_overlay {
                                    let ab_pid = grid.get_block_id(wx, wy + 1, wz) as usize;
                                    (cull_info[ab_pid].flags & CULL_WATER_HOST) != 0
                                } else {
                                    false
                                };
                                let self_fluid_corners = if *is_fluid {
                                    Some(compute_fluid_block_corners(
                                        grid,
                                        &cull_info,
                                        wx,
                                        wy,
                                        wz,
                                        *is_water,
                                    ))
                                } else {
                                    None
                                };
                                let bid = if *is_water {
                                    0
                                } else {
                                    grid.get_biome_id(gx, gy, gz)
                                };

                                for bf in faces {
                                    if bf.is_water_overlay && in_flat_water {
                                        continue;
                                    }
                                    if wy == c.min_y && bf.world_dir == FaceDir::Down {
                                        continue;
                                    }
                                    if let Some(cull_dir) = bf.cull_dir {
                                        let nb_pid = grid.get_block_id(
                                            wx + bf.cull_off[0],
                                            wy + bf.cull_off[1],
                                            wz + bf.cull_off[2],
                                        ) as usize;
                                        let nb_cull = cull_info[nb_pid];
                                        if bf.is_water_overlay {
                                            if (nb_cull.flags & CULL_AIR) == 0
                                                && (nb_cull.flags & (CULL_WATER_HOST | CULL_OCCLUDER)) != 0
                                            {
                                                continue;
                                            }
                                        } else if should_cull_pid(self_cull, nb_cull, cull_dir) {
                                            continue;
                                        }
                                    }
                                    let bf_biome = &bf.per_biome[bid.min(bf.per_biome.len() - 1)];
                                    let mut uvs = bf_biome.uvs;
                                    let mut norm_src = bf.norm_src;
                                    let corners = if let Some(sc) = self_fluid_corners {
                                        let mut mc = bf.corners_mc;
                                        match bf.world_dir {
                                            FaceDir::Up => {
                                                for pm in &mut mc {
                                                    pm[1] = fluid_corner_at(sc, pm[0], pm[2]);
                                                }
                                                let p0 = [mc[0][0], -mc[0][2], mc[0][1]];
                                                let p1 = [mc[1][0], -mc[1][2], mc[1][1]];
                                                let p2 = [mc[2][0], -mc[2][2], mc[2][1]];
                                                norm_src = compute_quad_normal(p0, p1, p2);
                                            }
                                            FaceDir::North
                                            | FaceDir::South
                                            | FaceDir::East
                                            | FaceDir::West => {
                                                let nx = wx + bf.cull_off[0];
                                                let nz = wz + bf.cull_off[2];
                                                let nb_pid = grid.get_block_id(nx, wy, nz) as usize;
                                                let nb_cull = cull_info[nb_pid];
                                                let same_pure = if *is_water { CULL_WATER } else { CULL_LAVA };
                                                let nei_corners: Option<[f32; 4]> =
                                                    if (nb_cull.flags & same_pure) != 0 {
                                                        Some(compute_fluid_block_corners(
                                                            grid,
                                                            &cull_info,
                                                            nx,
                                                            wy,
                                                            nz,
                                                            *is_water,
                                                        ))
                                                    } else if *is_water
                                                        && (nb_cull.flags & CULL_WATER_HOST) != 0
                                                    {
                                                        Some([14.2 / 16.0; 4])
                                                    } else {
                                                        None
                                                    };

                                                if let Some(nc) = nei_corners {
                                                    let (sa, sb, na, nb) = match bf.world_dir {
                                                        FaceDir::North => (sc[0], sc[1], nc[3], nc[2]),
                                                        FaceDir::South => (sc[3], sc[2], nc[0], nc[1]),
                                                        FaceDir::East => (sc[1], sc[2], nc[0], nc[3]),
                                                        FaceDir::West => (sc[0], sc[3], nc[1], nc[2]),
                                                        _ => unreachable!(),
                                                    };
                                                    if sa <= na + 1e-4 && sb <= nb + 1e-4 {
                                                        continue;
                                                    }
                                                }

                                                for pm in &mut mc {
                                                    let h_self = fluid_corner_at(sc, pm[0], pm[2]);
                                                    let h_bot = if let Some(nc) = nei_corners {
                                                        let h_nei = fluid_corner_at(
                                                            nc,
                                                            pm[0] - bf.cull_off[0] as f32,
                                                            pm[2] - bf.cull_off[2] as f32,
                                                        );
                                                        h_nei.min(h_self)
                                                    } else {
                                                        0.0
                                                    };
                                                    if pm[1] > 0.01 {
                                                        pm[1] = h_self;
                                                    } else {
                                                        pm[1] = h_bot;
                                                    }
                                                }

                                                let span_a = mc[3][1] - mc[0][1];
                                                let span_b = mc[2][1] - mc[1][1];
                                                if span_a <= 1e-4 && span_b <= 1e-4 {
                                                    continue;
                                                }
                                                let y_top_max = mc[2][1].max(mc[3][1]);
                                                let y_bot_min = mc[0][1].min(mc[1][1]);
                                                if y_top_max - y_bot_min <= 1e-4 {
                                                    continue;
                                                }
                                                if !matches!(
                                                    bf_biome.mat_name.as_ref(),
                                                    "terrain_opaque"
                                                        | "terrain_cutout"
                                                        | "terrain_cutout_cull"
                                                        | "terrain_translucent"
                                                ) {
                                                    for i in 0..4 {
                                                        uvs[i][1] = (1.0 - (y_top_max - mc[i][1]))
                                                            .clamp(0.0, 1.0);
                                                    }
                                                }
                                            }
                                            FaceDir::Down => {}
                                        }
                                        mc
                                    } else if same_fluid_above {
                                        *bf.corners_mc_full_top.as_ref().unwrap_or(&bf.corners_mc)
                                    } else {
                                        bf.corners_mc
                                    };
                                    if bf_biome.is_cutout {
                                        has_cutout = true;
                                    }
                                    let mut pts_src = [[0.0f32; 3]; 4];
                                    let wxf = wx as f64;
                                    let wyf = wy as f64;
                                    let wzf = wz as f64;
                                    for (i, &pm) in corners.iter().enumerate() {
                                        let sx_p = ((wxf + pm[0] as f64 - ox) * scale) as f32;
                                        let sy_p = (-(wzf + pm[2] as f64 - oz) * scale) as f32;
                                        let sz_p = ((wyf + pm[1] as f64 - oy) * scale) as f32;
                                        let p_s = [sx_p, sy_p, sz_p];
                                        expand_bounds(&mut bmin, &mut bmax, p_s);
                                        pts_src[i] = p_s;
                                    }

                                    let tri1 = MeshTri {
                                        material: bf_biome.mat_name.clone(),
                                        pos: [pts_src[0], pts_src[1], pts_src[2]],
                                        norm: [norm_src; 3],
                                        uv: [uvs[0], uvs[1], uvs[2]],
                                    };
                                    let tri2 = MeshTri {
                                        material: bf_biome.mat_name.clone(),
                                        pos: [pts_src[0], pts_src[2], pts_src[3]],
                                        norm: [norm_src; 3],
                                        uv: [uvs[0], uvs[2], uvs[3]],
                                    };
                                    if bf_biome.is_translucent {
                                        translucent_tris.push(tri1);
                                        translucent_tris.push(tri2);
                                    } else {
                                        solid_tris.push(tri1);
                                        solid_tris.push(tri2);
                                    }
                                }
                            }
                            BakedModel::Obj(tris) => {
                                for ot in tris {
                                    let uv_rect = atlas.get_or_default(&TexRequestKey {
                                        tex_ref: ot.texture.clone(),
                                        tint_rgb: None,
                                    });
                                    let mat = Cow::Borrowed(uv_rect.bucket.material_name());
                                    if matches!(
                                        uv_rect.bucket,
                                        RenderBucket::TerrainCutout | RenderBucket::TerrainCutoutCull
                                    ) {
                                        has_cutout = true;
                                    }
                                    let p0 = mc_point_to_source(
                                        [
                                            wx as f64 + ot.positions[0][0] as f64,
                                            wy as f64 + ot.positions[0][1] as f64,
                                            wz as f64 + ot.positions[0][2] as f64,
                                        ],
                                        origin_mc,
                                        scale,
                                    );
                                    let p1 = mc_point_to_source(
                                        [
                                            wx as f64 + ot.positions[1][0] as f64,
                                            wy as f64 + ot.positions[1][1] as f64,
                                            wz as f64 + ot.positions[1][2] as f64,
                                        ],
                                        origin_mc,
                                        scale,
                                    );
                                    let p2 = mc_point_to_source(
                                        [
                                            wx as f64 + ot.positions[2][0] as f64,
                                            wy as f64 + ot.positions[2][1] as f64,
                                            wz as f64 + ot.positions[2][2] as f64,
                                        ],
                                        origin_mc,
                                        scale,
                                    );
                                    expand_bounds(&mut bmin, &mut bmax, p0);
                                    expand_bounds(&mut bmin, &mut bmax, p1);
                                    expand_bounds(&mut bmin, &mut bmax, p2);
                                    let tri = MeshTri {
                                        material: mat,
                                        pos: [p0, p1, p2],
                                        norm: [
                                            mc_normal_to_source(ot.normals[0]),
                                            mc_normal_to_source(ot.normals[1]),
                                            mc_normal_to_source(ot.normals[2]),
                                        ],
                                        uv: [
                                            uv_rect.map_uv(wrap_obj_uv(ot.uvs[0][0]), 1.0 - wrap_obj_uv(ot.uvs[0][1])),
                                            uv_rect.map_uv(wrap_obj_uv(ot.uvs[1][0]), 1.0 - wrap_obj_uv(ot.uvs[1][1])),
                                            uv_rect.map_uv(wrap_obj_uv(ot.uvs[2][0]), 1.0 - wrap_obj_uv(ot.uvs[2][1])),
                                        ],
                                    };
                                    if uv_rect.bucket == RenderBucket::TerrainTranslucent {
                                        translucent_tris.push(tri);
                                    } else {
                                        solid_tris.push(tri);
                                    }
                                }
                            }
                            BakedModel::Empty => {}
                        }
                    }
                }
            }

            if solid_tris.is_empty() && translucent_tris.is_empty() {
                return None;
            }
            if !bmin[0].is_finite() {
                bmin = origin_source;
                bmax = origin_source;
            }
            let cm = ChunkMesh {
                cx: cx_i,
                cz: cz_i,
                name: format!("chunk_{}_{}", cx_i, cz_i),
                origin_source,
                bounds_min: bmin,
                bounds_max: bmax,
                solid_tris,
                translucent_tris,
                has_cutout,
            };
            Some(emit_chunk(cm))
        })
        .collect::<Result<Vec<T>, String>>()
}

fn expand_bounds(bmin: &mut [f64; 3], bmax: &mut [f64; 3], p: [f32; 3]) {
    for i in 0..3 {
        bmin[i] = bmin[i].min(p[i] as f64);
        bmax[i] = bmax[i].max(p[i] as f64);
    }
}

#[inline]
fn face_dir_opposite(d: FaceDir) -> FaceDir {
    match d {
        FaceDir::Up => FaceDir::Down,
        FaceDir::Down => FaceDir::Up,
        FaceDir::North => FaceDir::South,
        FaceDir::South => FaceDir::North,
        FaceDir::East => FaceDir::West,
        FaceDir::West => FaceDir::East,
    }
}

fn is_full_face_rect(dir: FaceDir, from: [f32; 3], to: [f32; 3]) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    match dir {
        FaceDir::Up => near(to[1], 16.0) && near(from[0], 0.0) && near(to[0], 16.0) && near(from[2], 0.0) && near(to[2], 16.0),
        FaceDir::Down => near(from[1], 0.0) && near(from[0], 0.0) && near(to[0], 16.0) && near(from[2], 0.0) && near(to[2], 16.0),
        FaceDir::North => near(from[2], 0.0) && near(from[0], 0.0) && near(to[0], 16.0) && near(from[1], 0.0) && near(to[1], 16.0),
        FaceDir::South => near(to[2], 16.0) && near(from[0], 0.0) && near(to[0], 16.0) && near(from[1], 0.0) && near(to[1], 16.0),
        FaceDir::East => near(to[0], 16.0) && near(from[1], 0.0) && near(to[1], 16.0) && near(from[2], 0.0) && near(to[2], 16.0),
        FaceDir::West => near(from[0], 0.0) && near(from[1], 0.0) && near(to[1], 16.0) && near(from[2], 0.0) && near(to[2], 16.0),
    }
}

fn default_face_uv(dir: FaceDir, from: [f32; 3], to: [f32; 3]) -> [f32; 4] {
    match dir {
        FaceDir::Up => [from[0], from[2], to[0], to[2]],
        FaceDir::Down => [from[0], 16.0 - to[2], to[0], 16.0 - from[2]],
        FaceDir::North => [16.0 - to[0], 16.0 - to[1], 16.0 - from[0], 16.0 - from[1]],
        FaceDir::South => [from[0], 16.0 - to[1], to[0], 16.0 - from[1]],
        FaceDir::East => [16.0 - to[2], 16.0 - to[1], 16.0 - from[2], 16.0 - from[1]],
        FaceDir::West => [from[2], 16.0 - to[1], to[2], 16.0 - from[1]],
    }
}

fn rotate_element_point(p: [f32; 3], origin_px: [f32; 3], axis: char, angle_deg: f32, rescale: bool) -> [f32; 3] {
    let o = [origin_px[0] / 16.0, origin_px[1] / 16.0, origin_px[2] / 16.0];
    let rad = angle_deg.to_radians();
    let (s, c) = rad.sin_cos();
    let mut dx = p[0] - o[0];
    let mut dy = p[1] - o[1];
    let mut dz = p[2] - o[2];
    match axis {
        'x' => {
            let ny = dy * c - dz * s;
            let nz = dy * s + dz * c;
            dy = ny;
            dz = nz;
            if rescale {
                let k = 1.0 / c.abs().max(0.5);
                dy *= k;
                dz *= k;
            }
        }
        'z' => {
            let nx = dx * c - dy * s;
            let ny = dx * s + dy * c;
            dx = nx;
            dy = ny;
            if rescale {
                let k = 1.0 / c.abs().max(0.5);
                dx *= k;
                dy *= k;
            }
        }
        _ => {
            let nx = dx * c + dz * s;
            let nz = -dx * s + dz * c;
            dx = nx;
            dz = nz;
            if rescale {
                let k = 1.0 / c.abs().max(0.5);
                dx *= k;
                dz *= k;
            }
        }
    }
    [o[0] + dx, o[1] + dy, o[2] + dz]
}

fn apply_blockstate_rot(p: [f32; 3], rot_x: i32, rot_y: i32, rot_z: i32) -> [f32; 3] {
    let mut x = p[0] - 0.5;
    let mut y = p[1] - 0.5;
    let mut z = p[2] - 0.5;
    let rx_steps = rot_x.rem_euclid(360) / 90;
    for _ in 0..rx_steps {
        let ny = z;
        let nz = -y;
        y = ny;
        z = nz;
    }
    let ry_steps = rot_y.rem_euclid(360) / 90;
    for _ in 0..ry_steps {
        let nx = -z;
        let nz = x;
        x = nx;
        z = nz;
    }
    let rz_steps = rot_z.rem_euclid(360) / 90;
    for _ in 0..rz_steps {
        let nx = y;
        let ny = -x;
        x = nx;
        y = ny;
    }
    [x + 0.5, y + 0.5, z + 0.5]
}

fn compute_quad_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let nx = u[1] * v[2] - u[2] * v[1];
    let ny = u[2] * v[0] - u[0] * v[2];
    let nz = u[0] * v[1] - u[1] * v[0];
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
    [nx / len, ny / len, nz / len]
}

#[inline]
fn sign_orientation(bs: &BlockState) -> (bool, f32) {
    let sname = bs.short_name();
    let is_wall = sname.contains("wall_sign") || sname.contains("wall_hanging_sign");
    let yaw_deg: f32 = if is_wall {
        match bs.prop("facing").unwrap_or("north") {
            "south" => 0.0,
            "west" => 90.0,
            "north" => 180.0,
            _ => 270.0, // east
        }
    } else {
        let rot: f32 = bs.prop("rotation").and_then(|v| v.parse().ok()).unwrap_or(0.0);
        rot * 22.5
    };
    (is_wall, yaw_deg)
}

#[inline]
fn rotate_sign_local_xz(px: f64, pz: f64, yaw_deg: f64) -> (f64, f64) {
    let rad = yaw_deg.to_radians();
    let (s, c) = rad.sin_cos();
    let dx = px - 0.5;
    let dz = pz - 0.5;
    (dx * c - dz * s + 0.5, dx * s + dz * c + 0.5)
}

/// Builds parkour activation zones and safe respawn anchors from `[mcgm checkpoint]` /
/// `[mcgm finish]` front-side signs using the map's MC -> Source coordinate transform.
pub fn extract_parkour_markers(
    grid: &VoxelGrid,
    origin_mc: [f64; 3],
    scale: f64,
) -> Vec<ParkourMarker> {
    let mut keys: Vec<(i32, i32, i32)> = grid.signs.keys().copied().collect();
    keys.sort();

    let mut markers = Vec::new();
    for (wx, wy, wz) in keys {
        let Some(sign) = grid.signs.get(&(wx, wy, wz)) else {
            continue;
        };
        let Some(parsed) = crate::anvil::parse_parkour_sign(sign) else {
            continue;
        };

        let bs = grid.get_block(wx, wy, wz);
        let (is_wall, yaw_deg) = sign_orientation(bs);
        let yaw_f64 = yaw_deg as f64;
        let rad = yaw_f64.to_radians();
        let fx_src = -rad.sin();
        let fy_src = -rad.cos();
        let src_yaw = fy_src.atan2(fx_src).to_degrees();

        let (px0, px1, py0, py1, pz0, pz1, anchor_x, anchor_y, anchor_z) = if is_wall {
            (-0.15, 1.15, -0.85, 1.35, 0.08, 1.45, 0.5, 0.0, 0.75)
        } else {
            (-0.25, 1.25, -0.85, 1.35, -0.15, 1.35, 0.5, 0.0, 0.5)
        };

        let mut zmin = [f64::INFINITY; 3];
        let mut zmax = [f64::NEG_INFINITY; 3];
        for px in [px0, px1] {
            for pz in [pz0, pz1] {
                let (rx, rz) = rotate_sign_local_xz(px, pz, yaw_f64);
                for py in [py0, py1] {
                    let pt = mc_point_to_source(
                        [wx as f64 + rx, wy as f64 + py, wz as f64 + rz],
                        origin_mc,
                        scale,
                    );
                    zmin[0] = zmin[0].min(pt[0] as f64);
                    zmin[1] = zmin[1].min(pt[1] as f64);
                    zmin[2] = zmin[2].min(pt[2] as f64);
                    zmax[0] = zmax[0].max(pt[0] as f64);
                    zmax[1] = zmax[1].max(pt[1] as f64);
                    zmax[2] = zmax[2].max(pt[2] as f64);
                }
            }
        }

        let (arx, arz) = rotate_sign_local_xz(anchor_x, anchor_z, yaw_f64);
        let anchor_src = mc_point_to_source(
            [wx as f64 + arx, wy as f64 + anchor_y, wz as f64 + arz],
            origin_mc,
            scale,
        );

        markers.push(ParkourMarker {
            id: format!("{}_{}_{}_{}", parsed.kind.as_str(), wx, wy, wz),
            kind: parsed.kind,
            name: parsed.name,
            mc_pos: [wx, wy, wz],
            is_wall,
            zone_min: zmin,
            zone_max: zmax,
            anchor_pos: [
                anchor_src[0] as f64,
                anchor_src[1] as f64,
                anchor_src[2] as f64,
            ],
            forward_dir: [fx_src, fy_src],
            yaw: src_yaw,
        });
    }
    markers
}

#[allow(clippy::too_many_arguments)]
fn emit_sign_geometry(
    bs: &BlockState,
    wx: i32,
    wy: i32,
    wz: i32,
    sign_atlas: &SignAtlas,
    _atlas: &TextureAtlas,
    origin_mc: [f64; 3],
    scale: f64,
    solid_tris: &mut Vec<MeshTri>,
    bmin: &mut [f64; 3],
    bmax: &mut [f64; 3],
) {
    let (is_wall, yaw_deg) = sign_orientation(bs);
    let key_front = format!("{},{},{},front", wx, wy, wz);
    let key_back = format!("{},{},{},back", wx, wy, wz);

    let (y0, y1, z_center) = if is_wall {
        (0.28f32, 0.78f32, 0.04f32)
    } else {
        (0.56f32, 1.06f32, 0.50f32)
    };
    let z_front = z_center + 0.035;
    let z_back = z_center - 0.035;

    let rot_y_pt = |px: f32, py: f32, pz: f32| -> [f32; 3] {
        let rad = yaw_deg.to_radians();
        let (s, c) = rad.sin_cos();
        let dx = px - 0.5;
        let dz = pz - 0.5;
        let rx = dx * c - dz * s + 0.5;
        let rz = dx * s + dz * c + 0.5;
        mc_point_to_source(
            [wx as f64 + rx as f64, wy as f64 + py as f64, wz as f64 + rz as f64],
            origin_mc,
            scale,
        )
    };

    let push_quad = |pts: [[f32; 3]; 4], uvs: [[f32; 2]; 4], tris: &mut Vec<MeshTri>, bmin: &mut [f64; 3], bmax: &mut [f64; 3]| {
        for p in &pts {
            expand_bounds(bmin, bmax, *p);
        }
        let n = compute_quad_normal(pts[0], pts[1], pts[2]);
        tris.push(MeshTri {
            material: Cow::Borrowed("sign_atlas"),
            pos: [pts[0], pts[1], pts[2]],
            norm: [n; 3],
            uv: [uvs[0], uvs[1], uvs[2]],
        });
        tris.push(MeshTri {
            material: Cow::Borrowed("sign_atlas"),
            pos: [pts[0], pts[2], pts[3]],
            norm: [n; 3],
            uv: [uvs[0], uvs[2], uvs[3]],
        });
    };

    let base_key = sign_base_key(&bs.name);
    let base_rect = sign_atlas
        .rects
        .get(&base_key)
        .or_else(|| sign_atlas.rects.values().next());
    let base_quad_uvs = if let Some(uvr) = base_rect {
        [
            uvr.map_uv(0.0, 1.0),
            uvr.map_uv(1.0, 1.0),
            uvr.map_uv(1.0, 0.0),
            uvr.map_uv(0.0, 0.0),
        ]
    } else {
        [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]
    };
    let edge_uvs = if let Some(uvr) = base_rect {
        [
            uvr.map_uv(0.0, 0.25),
            uvr.map_uv(1.0, 0.25),
            uvr.map_uv(1.0, 0.0),
            uvr.map_uv(0.0, 0.0),
        ]
    } else {
        [[0.0, 0.25], [1.0, 0.25], [1.0, 0.0], [0.0, 0.0]]
    };

    let quad_uvs = |rect_opt: Option<&UvRect>| -> [[f32; 2]; 4] {
        if let Some(uvr) = rect_opt {
            [
                uvr.map_uv(0.0, 1.0),
                uvr.map_uv(1.0, 1.0),
                uvr.map_uv(1.0, 0.0),
                uvr.map_uv(0.0, 0.0),
            ]
        } else {
            base_quad_uvs
        }
    };

    // Closed 6-face sign board
    let f_pts = [
        rot_y_pt(0.02, y0, z_front),
        rot_y_pt(0.98, y0, z_front),
        rot_y_pt(0.98, y1, z_front),
        rot_y_pt(0.02, y1, z_front),
    ];
    push_quad(f_pts, quad_uvs(sign_atlas.rects.get(&key_front)), solid_tris, bmin, bmax);

    let b_pts = [
        rot_y_pt(0.98, y0, z_back),
        rot_y_pt(0.02, y0, z_back),
        rot_y_pt(0.02, y1, z_back),
        rot_y_pt(0.98, y1, z_back),
    ];
    push_quad(b_pts, quad_uvs(sign_atlas.rects.get(&key_back)), solid_tris, bmin, bmax);

    let top_pts = [
        rot_y_pt(0.02, y1, z_front),
        rot_y_pt(0.98, y1, z_front),
        rot_y_pt(0.98, y1, z_back),
        rot_y_pt(0.02, y1, z_back),
    ];
    push_quad(top_pts, edge_uvs, solid_tris, bmin, bmax);

    let bot_pts = [
        rot_y_pt(0.02, y0, z_back),
        rot_y_pt(0.98, y0, z_back),
        rot_y_pt(0.98, y0, z_front),
        rot_y_pt(0.02, y0, z_front),
    ];
    push_quad(bot_pts, edge_uvs, solid_tris, bmin, bmax);

    let left_pts = [
        rot_y_pt(0.02, y0, z_back),
        rot_y_pt(0.02, y0, z_front),
        rot_y_pt(0.02, y1, z_front),
        rot_y_pt(0.02, y1, z_back),
    ];
    push_quad(left_pts, edge_uvs, solid_tris, bmin, bmax);

    let right_pts = [
        rot_y_pt(0.98, y0, z_front),
        rot_y_pt(0.98, y0, z_back),
        rot_y_pt(0.98, y1, z_back),
        rot_y_pt(0.98, y1, z_front),
    ];
    push_quad(right_pts, edge_uvs, solid_tris, bmin, bmax);

    // Closed 4-face standing post
    if !is_wall {
        let p_front = [
            rot_y_pt(0.45, 0.0, 0.55),
            rot_y_pt(0.55, 0.0, 0.55),
            rot_y_pt(0.55, 0.56, 0.55),
            rot_y_pt(0.45, 0.56, 0.55),
        ];
        push_quad(p_front, edge_uvs, solid_tris, bmin, bmax);

        let p_back = [
            rot_y_pt(0.55, 0.0, 0.45),
            rot_y_pt(0.45, 0.0, 0.45),
            rot_y_pt(0.45, 0.56, 0.45),
            rot_y_pt(0.55, 0.56, 0.45),
        ];
        push_quad(p_back, edge_uvs, solid_tris, bmin, bmax);

        let p_left = [
            rot_y_pt(0.45, 0.0, 0.45),
            rot_y_pt(0.45, 0.0, 0.55),
            rot_y_pt(0.45, 0.56, 0.55),
            rot_y_pt(0.45, 0.56, 0.45),
        ];
        push_quad(p_left, edge_uvs, solid_tris, bmin, bmax);

        let p_right = [
            rot_y_pt(0.55, 0.0, 0.55),
            rot_y_pt(0.55, 0.0, 0.45),
            rot_y_pt(0.55, 0.56, 0.45),
            rot_y_pt(0.55, 0.56, 0.55),
        ];
        push_quad(p_right, edge_uvs, solid_tris, bmin, bmax);
    }
}
