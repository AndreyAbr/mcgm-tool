use crate::anvil::{BlockState, VoxelGrid};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub enum SurfaceProp {
    #[default]
    Rock,
    Dirt,
    Grass,
    Wood,
    Metal,
    Glass,
    Sand,
    Gravel,
    Snow,
    Carpet,
}

impl SurfaceProp {
    pub const ALL: [SurfaceProp; 10] = [
        SurfaceProp::Rock,
        SurfaceProp::Dirt,
        SurfaceProp::Grass,
        SurfaceProp::Wood,
        SurfaceProp::Metal,
        SurfaceProp::Glass,
        SurfaceProp::Sand,
        SurfaceProp::Gravel,
        SurfaceProp::Snow,
        SurfaceProp::Carpet,
    ];

    pub fn as_source_str(self) -> &'static str {
        match self {
            SurfaceProp::Rock => "rock",
            SurfaceProp::Dirt => "dirt",
            SurfaceProp::Grass => "grass",
            SurfaceProp::Wood => "wood",
            SurfaceProp::Metal => "metal",
            SurfaceProp::Glass => "glass",
            SurfaceProp::Sand => "sand",
            SurfaceProp::Gravel => "gravel",
            SurfaceProp::Snow => "snow",
            SurfaceProp::Carpet => "carpet",
        }
    }
}

pub fn classify_surface_prop(bs: &BlockState) -> SurfaceProp {
    let s = bs.short_name();

    // Check stone/brick variants that contain "sand", "mud", "mossy_", or nylium first.
    if s.contains("sandstone")
        || s == "mud_bricks"
        || s.starts_with("mud_brick")
        || s == "packed_mud"
        || s.starts_with("mossy_")
        || s == "crimson_nylium"
        || s == "warped_nylium"
    {
        return SurfaceProp::Rock;
    }

    // Grass, leaves, moss, foliage before wood species (so oak_leaves -> Grass, not Wood).
    if s.contains("leaves")
        || s.contains("grass")
        || matches!(
            s,
            "moss_block"
                | "moss_carpet"
                | "pale_moss_block"
                | "pale_moss_carpet"
                | "podzol"
                | "mycelium"
                | "azalea"
                | "flowering_azalea"
                | "fern"
                | "large_fern"
                | "vine"
                | "hay_block"
                | "dried_kelp_block"
                | "melon"
                | "pumpkin"
                | "carved_pumpkin"
                | "jack_o_lantern"
                | "cactus"
        )
    {
        return SurfaceProp::Grass;
    }

    // Check muddy_mangrove_roots before wood species (so mangrove -> Wood does not intercept it).
    if s == "muddy_mangrove_roots" {
        return SurfaceProp::Dirt;
    }

    // Check wood species & wooden blocks before generic "_slab" / "_stairs".
    if matches!(
        s,
        "oak_log"
            | "spruce_log"
            | "birch_log"
            | "jungle_log"
            | "acacia_log"
            | "dark_oak_log"
            | "mangrove_log"
            | "cherry_log"
            | "pale_oak_log"
            | "crimson_stem"
            | "warped_stem"
            | "bamboo_block"
            | "stripped_bamboo_block"
            | "bookshelf"
            | "chiseled_bookshelf"
            | "crafting_table"
            | "chest"
            | "trapped_chest"
            | "barrel"
            | "lectern"
            | "composter"
            | "beehive"
            | "bee_nest"
            | "note_block"
            | "jukebox"
            | "scaffolding"
            | "ladder"
            | "loom"
            | "cartography_table"
            | "fletching_table"
            | "smithing_table"
    ) || s.contains("oak")
        || s.contains("spruce")
        || s.contains("birch")
        || s.contains("jungle")
        || s.contains("acacia")
        || s.contains("mangrove")
        || s.contains("cherry")
        || s.contains("bamboo")
        || s.contains("crimson")
        || s.contains("warped")
        || s.contains("planks")
        || s.contains("_log")
        || s.contains("_wood")
        || s.contains("_hyphae")
        || s.contains("_stem")
        || (s.contains("fence") && !s.contains("nether_brick"))
        || (s.contains("door") && !s.contains("iron") && !s.contains("copper"))
        || s.ends_with("_sign")
    {
        return SurfaceProp::Wood;
    }

    // Dirt, mud, clay, soil.
    if matches!(
        s,
        "dirt"
            | "coarse_dirt"
            | "rooted_dirt"
            | "dirt_path"
            | "farmland"
            | "mud"
            | "muddy_mangrove_roots"
            | "clay"
            | "soul_sand"
            | "soul_soil"
    ) {
        return SurfaceProp::Dirt;
    }

    // Sand.
    if s == "sand" || s == "red_sand" || s == "suspicious_sand" || s.ends_with("_concrete_powder") {
        return SurfaceProp::Sand;
    }

    // Gravel.
    if s == "gravel" || s == "suspicious_gravel" {
        return SurfaceProp::Gravel;
    }

    // Snow.
    if s.contains("snow") {
        return SurfaceProp::Snow;
    }

    // Glass & ice & crystal.
    if s.contains("glass")
        || s.contains("ice")
        || s.contains("amethyst")
        || matches!(s, "sea_lantern" | "glowstone" | "beacon" | "redstone_lamp")
    {
        return SurfaceProp::Glass;
    }

    // Ores are stone/rock, even if named iron_ore / gold_ore / copper_ore.
    if s.ends_with("_ore") {
        return SurfaceProp::Rock;
    }

    // Metal.
    if s.contains("iron")
        || s.contains("gold")
        || s.contains("copper")
        || s.contains("netherite")
        || s.contains("anvil")
        || s.contains("rail")
        || matches!(
            s,
            "chain"
                | "hopper"
                | "cauldron"
                | "water_cauldron"
                | "lava_cauldron"
                | "powder_snow_cauldron"
                | "bell"
                | "lantern"
                | "soul_lantern"
                | "lightning_rod"
                | "heavy_weighted_pressure_plate"
                | "light_weighted_pressure_plate"
                | "blast_furnace"
                | "stonecutter"
                | "brewing_stand"
        )
    {
        return SurfaceProp::Metal;
    }

    // Wool / carpet / cloth.
    if s.contains("wool") || s.contains("carpet") || s.contains("_bed") || s == "sponge" || s == "wet_sponge" {
        return SurfaceProp::Carpet;
    }

    SurfaceProp::Rock
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollisionBox {
    pub box_coords: [f64; 6], // [sx0, sy0, sz0, sx1, sy1, sz1] Source world units
    pub block_class: Cow<'static, str>,
    pub is_walkable: bool,
    pub is_opaque_roof: bool,
    #[serde(default)]
    pub surface_prop: SurfaceProp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicsZone {
    pub kind: String,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HazardBox {
    pub kind: String,
    pub box_coords: [f64; 6],
    pub damage: i32,
    pub damage_type: i32,
}

#[derive(Debug, Clone)]
pub struct WorldCollisionResult {
    pub per_chunk: HashMap<(i32, i32), Vec<CollisionBox>>,
    pub all_solid_boxes: Vec<CollisionBox>,
    pub ladder_boxes: Vec<[f64; 6]>,
    pub water_pools: Vec<[f64; 6]>,
    pub hazard_boxes: Vec<HazardBox>,
    pub physics_zones: Vec<PhysicsZone>,
    pub core_occluders: Vec<[f64; 6]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidClass {
    Passable,
    FullOpaque,
    FullLeaves,
    PartialBox,
}

pub fn is_non_solid_foliage_carpet(short_name: &str) -> bool {
    matches!(
        short_name,
        "leaf_litter"
            | "fallen_leaves"
            | "leaf_carpet"
            | "pink_petals"
            | "moss_carpet"
            | "pale_moss_carpet"
            | "wildflowers"
    )
}

pub fn is_leaves_block(state: &BlockState) -> bool {
    let s = state.short_name();
    if is_non_solid_foliage_carpet(s) {
        return false;
    }
    s == "leaves" || s.ends_with("_leaves") || s.contains("leaves")
}

#[derive(Clone, Copy)]
enum NameMatch {
    Exact(&'static str),
    Prefix(&'static str),
    Suffix(&'static str),
    Contains(&'static str),
}

impl NameMatch {
    #[inline]
    fn matches(self, s: &str) -> bool {
        match self {
            Self::Exact(p) => s == p,
            Self::Prefix(p) => s.starts_with(p),
            Self::Suffix(p) => s.ends_with(p),
            Self::Contains(p) => s.contains(p),
        }
    }
}

const PASSABLE_SOLID_OVERRIDES: &[NameMatch] = &[
    NameMatch::Suffix("_coral_block"),
    NameMatch::Suffix("_mushroom_block"),
    NameMatch::Exact("mushroom_stem"),
    NameMatch::Contains("rose_quartz"),
    NameMatch::Exact("campfire"),
    NameMatch::Exact("soul_campfire"),
];

const PASSABLE_DIRECT_RULES: &[NameMatch] = &[
    NameMatch::Exact("lava"),
    NameMatch::Exact("flowing_lava"),
    NameMatch::Exact("cobweb"),
    NameMatch::Exact("sweet_berry_bush"),
    NameMatch::Exact("scaffolding"),
    NameMatch::Exact("powder_snow"),
    NameMatch::Exact("candle"),
    NameMatch::Exact("repeater"),
    NameMatch::Exact("comparator"),
    NameMatch::Exact("lily_pad"),
    NameMatch::Exact("amethyst_cluster"),
    NameMatch::Exact("pointed_dripstone"),
    NameMatch::Exact("lightning_rod"),
    NameMatch::Exact("nether_portal"),
    NameMatch::Exact("end_portal"),
    NameMatch::Exact("light"),
    NameMatch::Exact("structure_void"),
    NameMatch::Exact("rock"),
    NameMatch::Exact("pebble"),
    NameMatch::Exact("small_rock"),
    NameMatch::Exact("stick"),
    NameMatch::Exact("twig"),
    NameMatch::Exact("fallen_twig"),
    NameMatch::Exact("pinecone"),
    NameMatch::Exact("acorn"),
    NameMatch::Exact("seashell"),
    NameMatch::Exact("bush"),
    NameMatch::Exact("firefly_bush"),
    NameMatch::Exact("pale_hanging_moss"),
    NameMatch::Exact("open_eyeblossom"),
    NameMatch::Exact("closed_eyeblossom"),
    NameMatch::Exact("eyeblossom"),
    NameMatch::Exact("resin_clump"),
    NameMatch::Exact("cactus_flower"),
    NameMatch::Exact("frogspawn"),
    NameMatch::Exact("sea_pickle"),
    NameMatch::Exact("pink_petals"),
    NameMatch::Exact("wildflowers"),
    NameMatch::Suffix("_candle"),
    NameMatch::Suffix("_pebble"),
    NameMatch::Suffix("_twig"),
    NameMatch::Suffix("_lightning_rod"),
];

const PASSABLE_PRE_TOKEN_SOLID_OVERRIDES: &[&str] = &[
    "grass_block",
    "muddy_mangrove_roots",
    "moss_block",
    "bamboo",
];

const PASSABLE_CONTAINS_TOKENS: &[&str] = &[
    "grass", "fern", "flower", "poppy", "dandelion", "tulip", "orchid", "allium", "bluet",
    "daisy", "lily", "rose", "lilac", "peony", "sunflower", "sapling", "torch", "rail",
    "redstone_wire", "tripwire", "string", "button", "lever", "pressure_plate", "banner",
    "_sign", "vine", "ladder", "sugar_cane", "wheat", "carrots", "potatoes", "beetroots",
    "nether_wart", "mushroom", "kelp", "seagrass", "coral", "fire", "dead_bush", "bamboo_sapling",
    "hanging_roots", "glow_lichen", "sculk_vein", "spore_blossom", "big_dripleaf_stem",
    "small_dripleaf", "pitcher_plant", "torchflower", "carpet",
];

pub fn is_passable_block(state: &BlockState) -> bool {
    if state.is_air || state.is_water_tag {
        return true;
    }
    let s = state.short_name();
    if PASSABLE_SOLID_OVERRIDES.iter().any(|m| m.matches(s)) {
        return false;
    }
    if PASSABLE_DIRECT_RULES.iter().any(|m| m.matches(s)) || is_non_solid_foliage_carpet(s) {
        return true;
    }
    if is_leaves_block(state) {
        return false;
    }
    if s == "snow" {
        let layers: i32 = state.prop("layers").and_then(|v| v.parse().ok()).unwrap_or(1);
        return layers <= 1;
    }
    if s.ends_with("_door") || s.ends_with("_fence_gate") {
        // Doors and fence gates become interactive func_door_rotating entities
        return true;
    }
    if PASSABLE_PRE_TOKEN_SOLID_OVERRIDES.contains(&s) {
        return false;
    }
    PASSABLE_CONTAINS_TOKENS.iter().any(|t| s.contains(t))
}

/// Returns snow height in normalized block units [0.0 .. 1.0]:
/// layers == 1 -> None (passable); layers >= 8 -> 1.0; layers 2..=7 -> (layers - 1) / 8.0
pub fn snow_collision_height(layers: i32) -> Option<f64> {
    if layers <= 1 {
        None
    } else if layers >= 8 {
        Some(1.0)
    } else {
        Some(((layers - 1) as f64) / 8.0)
    }
}

const PARTIAL_BOX_RULES: &[NameMatch] = &[
    NameMatch::Exact("dirt_path"),
    NameMatch::Exact("farmland"),
    NameMatch::Exact("snow"),
    NameMatch::Suffix("_stairs"),
    NameMatch::Suffix("_fence"),
    NameMatch::Suffix("_wall"),
    NameMatch::Suffix("_pane"),
    NameMatch::Exact("iron_bars"),
    NameMatch::Suffix("_trapdoor"),
    NameMatch::Suffix("_bed"),
    NameMatch::Contains("chest"),
    NameMatch::Exact("lantern"),
    NameMatch::Exact("soul_lantern"),
    NameMatch::Exact("chain"),
    NameMatch::Exact("end_rod"),
    NameMatch::Exact("bamboo"),
    NameMatch::Exact("cactus"),
    NameMatch::Exact("campfire"),
    NameMatch::Exact("soul_campfire"),
    NameMatch::Exact("enchanting_table"),
    NameMatch::Exact("anvil"),
    NameMatch::Exact("chipped_anvil"),
    NameMatch::Exact("damaged_anvil"),
    NameMatch::Exact("brewing_stand"),
    NameMatch::Exact("hopper"),
    NameMatch::Exact("cauldron"),
    NameMatch::Suffix("_cauldron"),
    NameMatch::Exact("lectern"),
    NameMatch::Exact("composter"),
    NameMatch::Exact("bell"),
    NameMatch::Exact("flower_pot"),
    NameMatch::Prefix("potted_"),
    NameMatch::Exact("soul_sand"),
    NameMatch::Exact("mud"),
    NameMatch::Exact("honey_block"),
    NameMatch::Exact("daylight_detector"),
    NameMatch::Exact("stonecutter"),
    NameMatch::Exact("end_portal_frame"),
    NameMatch::Exact("cake"),
    NameMatch::Suffix("_candle_cake"),
    NameMatch::Exact("grindstone"),
    NameMatch::Exact("sculk_sensor"),
    NameMatch::Exact("calibrated_sculk_sensor"),
    NameMatch::Exact("sculk_shrieker"),
    NameMatch::Contains("glass"),
];

const CREATE_PARTIAL_TOKENS: &[&str] = &[
    "cogwheel", "shaft", "belt", "pipe", "track", "girder", "funnel", "chute", "basin", "depot",
    "burner",
];

pub fn classify_solid_block(state: &BlockState) -> SolidClass {
    if is_passable_block(state) {
        return SolidClass::Passable;
    }
    if is_leaves_block(state) {
        return SolidClass::FullLeaves;
    }
    let s = state.short_name();
    if PARTIAL_BOX_RULES.iter().any(|m| m.matches(s)) {
        return SolidClass::PartialBox;
    }
    if s.ends_with("_slab") {
        if state.prop("type") == Some("double") {
            return SolidClass::FullOpaque;
        }
        return SolidClass::PartialBox;
    }
    // Create / modded kinetic & non-full blocks
    if state.name.starts_with("create:") && CREATE_PARTIAL_TOKENS.iter().any(|t| s.contains(t)) {
        return SolidClass::PartialBox;
    }
    SolidClass::FullOpaque
}

/// Classifies uniform horizontal slabs (bottom slab, top slab, dirt_path/farmland)
/// so they can be 2D greedy-merged per Y slice identically to snow sheets.
fn horizontal_slab_profile(state: &BlockState) -> Option<(usize, f64, f64)> {
    if classify_solid_block(state) != SolidClass::PartialBox {
        return None;
    }
    let s = state.short_name();
    if s == "dirt_path" || s == "farmland" {
        return Some((2, 0.0, 15.0 / 16.0));
    }
    if s.ends_with("_slab") && state.prop("type") != Some("double") {
        if state.prop("type") == Some("top") {
            return Some((1, 0.5, 1.0));
        }
        return Some((0, 0.0, 0.5));
    }
    None
}

struct StaticCollisionSpec {
    matcher: NameMatch,
    aabb: [f64; 6],
    block_class: &'static str,
    is_opaque_roof: bool,
}

const STATIC_PARTIAL_BOXES: &[StaticCollisionSpec] = &[
    StaticCollisionSpec {
        matcher: NameMatch::Suffix("_bed"),
        aabb: [0.0, 0.0, 0.0, 1.0, 0.5625, 1.0],
        block_class: "slab",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("cactus"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.9375, 0.9375],
        block_class: "full",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Contains("glass"),
        aabb: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        block_class: "glass",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Contains("chest"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.875, 0.9375],
        block_class: "slab",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("enchanting_table"),
        aabb: [0.0, 0.0, 0.0, 1.0, 0.75, 1.0],
        block_class: "slab",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("soul_sand"),
        aabb: [0.0, 0.0, 0.0, 1.0, 0.875, 1.0],
        block_class: "full",
        is_opaque_roof: true,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("mud"),
        aabb: [0.0, 0.0, 0.0, 1.0, 0.875, 1.0],
        block_class: "full",
        is_opaque_roof: true,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("honey_block"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.9375, 0.9375],
        block_class: "full",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("daylight_detector"),
        aabb: [0.0, 0.0, 0.0, 1.0, 0.375, 1.0],
        block_class: "slab",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("stonecutter"),
        aabb: [0.0, 0.0, 0.0, 1.0, 0.5625, 1.0],
        block_class: "slab",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("end_portal_frame"),
        aabb: [0.0, 0.0, 0.0, 1.0, 0.8125, 1.0],
        block_class: "slab",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("cake"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "slab",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Suffix("_candle_cake"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "slab",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("lantern"),
        aabb: [0.3125, 0.0, 0.3125, 0.6875, 0.5625, 0.6875],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("soul_lantern"),
        aabb: [0.3125, 0.0, 0.3125, 0.6875, 0.5625, 0.6875],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("bamboo"),
        // Minecraft bamboo stalk is 2/16 block wide centered vertically
        aabb: [0.4375, 0.0, 0.4375, 0.5625, 1.0, 0.5625],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("chain"),
        aabb: [0.375, 0.0, 0.375, 0.625, 1.0, 0.625],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("end_rod"),
        aabb: [0.375, 0.0, 0.375, 0.625, 1.0, 0.625],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("flower_pot"),
        aabb: [0.3125, 0.0, 0.3125, 0.6875, 0.375, 0.6875],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Prefix("potted_"),
        aabb: [0.3125, 0.0, 0.3125, 0.6875, 0.375, 0.6875],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("anvil"),
        aabb: [0.125, 0.0, 0.125, 0.875, 1.0, 0.875],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("chipped_anvil"),
        aabb: [0.125, 0.0, 0.125, 0.875, 1.0, 0.875],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("damaged_anvil"),
        aabb: [0.125, 0.0, 0.125, 0.875, 1.0, 0.875],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("brewing_stand"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("lectern"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("grindstone"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("bell"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("sculk_sensor"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("calibrated_sculk_sensor"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "partial",
        is_opaque_roof: false,
    },
    StaticCollisionSpec {
        matcher: NameMatch::Exact("sculk_shrieker"),
        aabb: [0.0625, 0.0, 0.0625, 0.9375, 0.5, 0.9375],
        block_class: "partial",
        is_opaque_roof: false,
    },
];

/// Emits normalized MC block-local AABBs: ([x0, y0, z0, x1, y1, z1], block_class, is_opaque_roof)
pub fn block_local_collision_boxes(state: &BlockState) -> Vec<([f64; 6], &'static str, bool)> {
    let class = classify_solid_block(state);
    match class {
        SolidClass::Passable => Vec::new(),
        SolidClass::FullOpaque => vec![([0.0, 0.0, 0.0, 1.0, 1.0, 1.0], "full", true)],
        SolidClass::FullLeaves => vec![([0.0, 0.0, 0.0, 1.0, 1.0, 1.0], "leaves", false)],
        SolidClass::PartialBox => {
            let s = state.short_name();
            if s == "snow" {
                let layers: i32 = state.prop("layers").and_then(|v| v.parse().ok()).unwrap_or(1);
                if let Some(h) = snow_collision_height(layers) {
                    return vec![([0.0, 0.0, 0.0, 1.0, h, 1.0], "snow", false)];
                }
                return Vec::new();
            }
            if s == "dirt_path" || s == "farmland" {
                return vec![([0.0, 0.0, 0.0, 1.0, 15.0 / 16.0, 1.0], "slab", true)];
            }
            if s.ends_with("_slab") {
                return match state.prop("type") {
                    Some("top") => vec![([0.0, 0.5, 0.0, 1.0, 1.0, 1.0], "slab", true)],
                    Some("double") => vec![([0.0, 0.0, 0.0, 1.0, 1.0, 1.0], "full", true)],
                    _ => vec![([0.0, 0.0, 0.0, 1.0, 0.5, 1.0], "slab", true)],
                };
            }
            if s.ends_with("_fence") {
                let mut out = vec![([0.375, 0.0, 0.375, 0.625, 1.5, 0.625], "fence", false)];
                if state.prop("north") == Some("true") {
                    out.push(([0.4375, 0.0, 0.0, 0.5625, 1.5, 0.5], "fence", false));
                }
                if state.prop("south") == Some("true") {
                    out.push(([0.4375, 0.0, 0.5, 0.5625, 1.5, 1.0], "fence", false));
                }
                if state.prop("west") == Some("true") {
                    out.push(([0.0, 0.0, 0.4375, 0.5, 1.5, 0.5625], "fence", false));
                }
                if state.prop("east") == Some("true") {
                    out.push(([0.5, 0.0, 0.4375, 1.0, 1.5, 0.5625], "fence", false));
                }
                return out;
            }
            if s.ends_with("_wall") {
                let mut out = vec![([0.25, 0.0, 0.25, 0.75, 1.5, 0.75], "wall", false)];
                let connected = |dir: &str| -> bool {
                    matches!(state.prop(dir), Some(v) if !v.is_empty() && v != "none")
                };
                if connected("north") {
                    out.push(([0.3125, 0.0, 0.0, 0.6875, 1.5, 0.5], "wall", false));
                }
                if connected("south") {
                    out.push(([0.3125, 0.0, 0.5, 0.6875, 1.5, 1.0], "wall", false));
                }
                if connected("west") {
                    out.push(([0.0, 0.0, 0.3125, 0.5, 1.5, 0.6875], "wall", false));
                }
                if connected("east") {
                    out.push(([0.5, 0.0, 0.3125, 1.0, 1.5, 0.6875], "wall", false));
                }
                return out;
            }
            if s.ends_with("_stairs") {
                let top = state.prop("half") == Some("top");
                let facing = state.prop("facing").unwrap_or("north");
                let shape = state.prop("shape").unwrap_or("straight");
                let base = if top {
                    [0.0, 0.5, 0.0, 1.0, 1.0, 1.0]
                } else {
                    [0.0, 0.0, 0.0, 1.0, 0.5, 1.0]
                };
                let (sy0, sy1) = if top { (0.0, 0.5) } else { (0.5, 1.0) };
                let nw = [0.0, sy0, 0.0, 0.5, sy1, 0.5];
                let ne = [0.5, sy0, 0.0, 1.0, sy1, 0.5];
                let sw = [0.0, sy0, 0.5, 0.5, sy1, 1.0];
                let se = [0.5, sy0, 0.5, 1.0, sy1, 1.0];
                let north = [0.0, sy0, 0.0, 1.0, sy1, 0.5];
                let south = [0.0, sy0, 0.5, 1.0, sy1, 1.0];
                let east = [0.5, sy0, 0.0, 1.0, sy1, 1.0];
                let west = [0.0, sy0, 0.0, 0.5, sy1, 1.0];

                let mut out = vec![(base, "stairs", true)];
                match (facing, shape) {
                    ("north", "outer_left") => out.push((nw, "stairs", true)),
                    ("north", "outer_right") => out.push((ne, "stairs", true)),
                    ("north", "inner_left") => {
                        out.push((north, "stairs", true));
                        out.push((sw, "stairs", true));
                    }
                    ("north", "inner_right") => {
                        out.push((north, "stairs", true));
                        out.push((se, "stairs", true));
                    }
                    ("north", _) => out.push((north, "stairs", true)),

                    ("south", "outer_left") => out.push((se, "stairs", true)),
                    ("south", "outer_right") => out.push((sw, "stairs", true)),
                    ("south", "inner_left") => {
                        out.push((south, "stairs", true));
                        out.push((ne, "stairs", true));
                    }
                    ("south", "inner_right") => {
                        out.push((south, "stairs", true));
                        out.push((nw, "stairs", true));
                    }
                    ("south", _) => out.push((south, "stairs", true)),

                    ("east", "outer_left") => out.push((ne, "stairs", true)),
                    ("east", "outer_right") => out.push((se, "stairs", true)),
                    ("east", "inner_left") => {
                        out.push((east, "stairs", true));
                        out.push((nw, "stairs", true));
                    }
                    ("east", "inner_right") => {
                        out.push((east, "stairs", true));
                        out.push((sw, "stairs", true));
                    }
                    ("east", _) => out.push((east, "stairs", true)),

                    ("west", "outer_left") => out.push((sw, "stairs", true)),
                    ("west", "outer_right") => out.push((nw, "stairs", true)),
                    ("west", "inner_left") => {
                        out.push((west, "stairs", true));
                        out.push((se, "stairs", true));
                    }
                    ("west", "inner_right") => {
                        out.push((west, "stairs", true));
                        out.push((ne, "stairs", true));
                    }
                    ("west", _) => out.push((west, "stairs", true)),

                    (_, _) => out.push((north, "stairs", true)),
                }
                return out;
            }
            if s.ends_with("_trapdoor") {
                let open = state.prop("open") == Some("true");
                if open {
                    let b = match state.prop("facing").unwrap_or("north") {
                        "south" => [0.0, 0.0, 0.0, 1.0, 1.0, 0.1875],
                        "east" => [0.0, 0.0, 0.0, 0.1875, 1.0, 1.0],
                        "west" => [0.8125, 0.0, 0.0, 1.0, 1.0, 1.0],
                        _ => [0.0, 0.0, 0.8125, 1.0, 1.0, 1.0],
                    };
                    return vec![(b, "trapdoor", false)];
                } else if state.prop("half") == Some("top") {
                    return vec![([0.0, 0.8125, 0.0, 1.0, 1.0, 1.0], "trapdoor", false)];
                } else {
                    return vec![([0.0, 0.0, 0.0, 1.0, 0.1875, 1.0], "trapdoor", false)];
                }
            }
            if s.ends_with("_pane") || s == "iron_bars" {
                let mut out = vec![([0.4375, 0.0, 0.4375, 0.5625, 1.0, 0.5625], "pane", false)];
                if state.prop("north") == Some("true") {
                    out.push(([0.4375, 0.0, 0.0, 0.5625, 1.0, 0.4375], "pane", false));
                }
                if state.prop("south") == Some("true") {
                    out.push(([0.4375, 0.0, 0.5625, 0.5625, 1.0, 1.0], "pane", false));
                }
                if state.prop("west") == Some("true") {
                    out.push(([0.0, 0.0, 0.4375, 0.4375, 1.0, 0.5625], "pane", false));
                }
                if state.prop("east") == Some("true") {
                    out.push(([0.5625, 0.0, 0.4375, 1.0, 1.0, 0.5625], "pane", false));
                }
                return out;
            }
            if let Some(spec) = STATIC_PARTIAL_BOXES.iter().find(|spec| spec.matcher.matches(s)) {
                return vec![(spec.aabb, spec.block_class, spec.is_opaque_roof)];
            }
            vec![([0.0, 0.0, 0.0, 1.0, 1.0, 1.0], "full", false)]
        }
    }
}

pub fn surface_zone_kind(state: &BlockState) -> Option<&'static str> {
    if state.is_water_tag || state.waterlogged {
        return Some("water");
    }
    let s = state.short_name();
    match s {
        "lava" | "flowing_lava" => Some("lava"),
        "cobweb" => Some("cobweb"),
        "slime_block" => Some("slime"),
        "honey_block" => Some("honey"),
        "ice" | "frosted_ice" => Some("ice"),
        "packed_ice" => Some("packed_ice"),
        "blue_ice" => Some("blue_ice"),
        "soul_sand" => Some("soul_sand"),
        "magma_block" => Some("magma"),
        "hay_block" => Some("hay"),
        "ladder" => Some("ladder"),
        "vine" | "weeping_vines" | "weeping_vines_plant" | "twisting_vines" | "twisting_vines_plant" | "scaffolding" => Some("vine"),
        _ if s.ends_with("_bed") => Some("bed"),
        _ => None,
    }
}

#[inline]
pub fn mc_box_to_source(
    mc_x: f64,
    mc_y: f64,
    mc_z: f64,
    local: [f64; 6],
    origin_mc: [f64; 3],
    scale: f64,
) -> [f64; 6] {
    let mx0 = mc_x + local[0];
    let mx1 = mc_x + local[3];
    let my0 = mc_y + local[1];
    let my1 = mc_y + local[4];
    let mz0 = mc_z + local[2];
    let mz1 = mc_z + local[5];

    let sx0 = (mx0 - origin_mc[0]) * scale;
    let sx1 = (mx1 - origin_mc[0]) * scale;
    let sy0 = -(mz1 - origin_mc[2]) * scale;
    let sy1 = -(mz0 - origin_mc[2]) * scale;
    let sz0 = (my0 - origin_mc[1]) * scale;
    let sz1 = (my1 - origin_mc[1]) * scale;
    [
        sx0.min(sx1),
        sy0.min(sy1),
        sz0.min(sz1),
        sx0.max(sx1),
        sy0.max(sy1),
        sz0.max(sz1),
    ]
}

/// Filters out completely buried interior full-solid cubes (Hollow Shell),
/// then performs 3D Greedy Meshing over remaining exposed full cubes per chunk.
#[cfg(test)]
pub fn build_world_collision(
    grid: &VoxelGrid,
    origin_mc: [f64; 3],
    scale: f64,
) -> WorldCollisionResult {
    build_world_collision_with_chunk_size(grid, origin_mc, scale, 64)
}

const FLAG_FULL_OPAQUE: u8 = 1 << 0;
const FLAG_LEAVES: u8 = 1 << 1;
const FLAG_GLASS_CUBE: u8 = 1 << 2;
const FLAG_SHELL_OPAQUE: u8 = 1 << 3;
const FLAG_PASSABLE: u8 = 1 << 4;
const FLAG_WATER: u8 = 1 << 5;
const FLAG_LAVA: u8 = 1 << 6;
const FLAG_WATER_BRUSH: u8 = 1 << 7;

pub fn is_water_brush_eligible(state: &BlockState) -> bool {
    let s = state.short_name();
    if s == "water" && state.prop("level").unwrap_or("0") == "0" {
        return true;
    }
    state.waterlogged && is_passable_block(state)
}

const MERGEABLE_ZONE_KINDS: [&str; 11] = [
    "cobweb",
    "slime",
    "honey",
    "ice",
    "packed_ice",
    "blue_ice",
    "soul_sand",
    "magma",
    "hay",
    "vine",
    "bed",
];

#[derive(Clone)]
struct PaletteColMeta {
    solid_class: SolidClass,
    is_passable: bool,
    is_glass_cube: bool,
    snow_layers: u8,
    slab_kind: u8,
    stair_info: Option<(u8, u8)>,
    wall_fence_kind: u8, // 0=none, 1=wall, 2=fence
    wall_fence_conn: u8, // bit 0=N, 1=S, 2=W, 3=E, 4=skip_post
    is_water: bool,
    is_water_brush: bool,
    is_lava: bool,
    hazard_kind: u8, // 0=none, 1=fire, 2=soul_fire, 3=lit_campfire, 4=magma_block, 5=cactus
    zone_kind_idx: u8, // 0..11 into MERGEABLE_ZONE_KINDS, or 255=none
    ladder_facing: u8, // 0=north (+Z), 1=south (-Z), 2=east (-X), 3=west (+X), 255=none
    vine_wall_mask: u8, // bit 0=south(+Z), 1=north(-Z), 2=west(-X), 3=east(+X)
    is_unmerged_partial: bool,
    unmerged_boxes: Vec<([f64; 6], &'static str, bool)>,
    surface_prop: SurfaceProp,
}

fn rotate_axis_pt(mut x: f64, mut y: f64, mut z: f64, axis: char, angle_deg: f64) -> [f64; 3] {
    let rad = angle_deg.to_radians();
    let (s, c) = rad.sin_cos();
    match axis {
        'x' | 'X' => {
            let ny = y * c - z * s;
            let nz = y * s + z * c;
            y = ny;
            z = nz;
        }
        'y' | 'Y' => {
            let nx = x * c + z * s;
            let nz = -x * s + z * c;
            x = nx;
            z = nz;
        }
        'z' | 'Z' => {
            let nx = x * c - y * s;
            let ny = x * s + y * c;
            x = nx;
            y = ny;
        }
        _ => {}
    }
    [x, y, z]
}

fn rotate_model_pt(
    mut p: [f64; 3],
    rot_x: i32,
    rot_y: i32,
    rot_z: i32,
    rot: Option<&crate::assets::ModelRotation>,
) -> [f64; 3] {
    if let Some(r) = rot {
        let ox = r.origin[0] as f64 / 16.0;
        let oy = r.origin[1] as f64 / 16.0;
        let oz = r.origin[2] as f64 / 16.0;
        let mut x = p[0] - ox;
        let mut y = p[1] - oy;
        let mut z = p[2] - oz;
        if let Some([rx, ry, rz]) = r.euler_xyz {
            if rx.abs() > 1e-4 {
                let v = rotate_axis_pt(x, y, z, 'x', rx as f64);
                x = v[0];
                y = v[1];
                z = v[2];
            }
            if ry.abs() > 1e-4 {
                let v = rotate_axis_pt(x, y, z, 'y', ry as f64);
                x = v[0];
                y = v[1];
                z = v[2];
            }
            if rz.abs() > 1e-4 {
                let v = rotate_axis_pt(x, y, z, 'z', rz as f64);
                x = v[0];
                y = v[1];
                z = v[2];
            }
        } else {
            let v = rotate_axis_pt(x, y, z, r.axis, r.angle as f64);
            x = v[0];
            y = v[1];
            z = v[2];
        }
        p = [x + ox, y + oy, z + oz];
    }
    let rx_steps = rot_x.rem_euclid(360) / 90;
    for _ in 0..rx_steps {
        let y = p[1] - 0.5;
        let z = p[2] - 0.5;
        p[1] = z + 0.5;
        p[2] = -y + 0.5;
    }
    let ry_steps = rot_y.rem_euclid(360) / 90;
    for _ in 0..ry_steps {
        let x = p[0] - 0.5;
        let z = p[2] - 0.5;
        p[0] = -z + 0.5;
        p[2] = x + 0.5;
    }
    let rz_steps = rot_z.rem_euclid(360) / 90;
    for _ in 0..rz_steps {
        let x = p[0] - 0.5;
        let y = p[1] - 0.5;
        p[0] = y + 0.5;
        p[1] = -x + 0.5;
    }
    p
}

fn compute_model_normalized_aabb(model: &crate::assets::ResolvedBlockModel) -> Option<[f64; 6]> {
    match model {
        crate::assets::ResolvedBlockModel::Empty => None,
        crate::assets::ResolvedBlockModel::Obj(_) => Some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
        crate::assets::ResolvedBlockModel::Elements(els) => {
            if els.is_empty() {
                return None;
            }
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            for el in els {
                let f = [
                    el.from[0] as f64 / 16.0,
                    el.from[1] as f64 / 16.0,
                    el.from[2] as f64 / 16.0,
                ];
                let t = [
                    el.to[0] as f64 / 16.0,
                    el.to[1] as f64 / 16.0,
                    el.to[2] as f64 / 16.0,
                ];
                for ix in 0..2 {
                    for iy in 0..2 {
                        for iz in 0..2 {
                            let pt = [
                                if ix == 0 { f[0] } else { t[0] },
                                if iy == 0 { f[1] } else { t[1] },
                                if iz == 0 { f[2] } else { t[2] },
                            ];
                            let rp = rotate_model_pt(pt, el.rot_x, el.rot_y, el.rot_z, el.rotation.as_ref());
                            for k in 0..3 {
                                min[k] = min[k].min(rp[k]);
                                max[k] = max[k].max(rp[k]);
                            }
                        }
                    }
                }
            }
            if !min[0].is_finite() {
                return None;
            }
            Some([
                min[0].clamp(0.0, 1.0),
                min[1].clamp(0.0, 1.0),
                min[2].clamp(0.0, 1.0),
                max[0].clamp(0.0, 1.0),
                max[1].clamp(0.0, 1.0),
                max[2].clamp(0.0, 1.0),
            ])
        }
    }
}

/// Filters out completely buried interior full-solid cubes,
/// then performs 3D Greedy Meshing over remaining exposed full cubes per `chunk_size x chunk_size` chunk.
#[cfg(test)]
pub fn build_world_collision_with_chunk_size(
    grid: &VoxelGrid,
    origin_mc: [f64; 3],
    scale: f64,
    chunk_size: usize,
) -> WorldCollisionResult {
    build_world_collision_with_models(grid, None, origin_mc, scale, chunk_size)
}

pub fn build_world_collision_with_models(
    grid: &VoxelGrid,
    resolved_models: Option<&[crate::assets::ResolvedBlockModel]>,
    origin_mc: [f64; 3],
    scale: f64,
    chunk_size: usize,
) -> WorldCollisionResult {
    let chunk_size = chunk_size.max(1);
    let c = grid.coords;
    let sx = c.size_x();
    let sy = c.size_y();
    let sz = c.size_z();
    let total_voxels = grid.blocks.len();

    // Precompute all collision/zone/hazard metadata once per unique palette entry
    let mut any_leaves = false;
    let mut any_glass = false;
    let mut any_snow = false;
    let mut any_slab = false;
    let mut any_stairs = false;
    let mut any_walls = false;
    let mut any_fences = false;
    let mut any_water = false;
    let mut any_water_brush = false;
    let mut any_non_brush_water = false;
    let mut any_lava = false;
    let mut any_partial = false;
    let mut any_hazard = false;
    let mut zone_kinds_mask: u16 = 0;
    let mut ladder_sides_mask: u8 = 0;
    let mut vine_sides_mask: u8 = 0;

    let pal_meta: Vec<PaletteColMeta> = grid
        .palette
        .iter()
        .enumerate()
        .map(|(pid, bs)| {
            let sname = bs.short_name();
            let mut solid_class = classify_solid_block(bs);
            let mut is_passable = is_passable_block(bs);
            let is_glass_cube = solid_class == SolidClass::PartialBox
                && sname.contains("glass")
                && !sname.ends_with("_pane");

            let snow_layers = if sname == "snow" || sname == "snow_layer" {
                let l: u8 = bs.prop("layers").and_then(|v| v.parse().ok()).unwrap_or(1);
                if (2..=8).contains(&l) {
                    any_snow = true;
                    l
                } else {
                    0
                }
            } else {
                0
            };

            let slab_kind = if let Some((k, _, _)) = horizontal_slab_profile(bs) {
                any_slab = true;
                k as u8
            } else {
                255
            };

            let stair_info = if solid_class == SolidClass::PartialBox
                && sname.ends_with("_stairs")
                && matches!(bs.prop("shape"), None | Some("straight"))
            {
                any_stairs = true;
                let top = u8::from(bs.prop("half") == Some("top"));
                let facing = match bs.prop("facing").unwrap_or("north") {
                    "south" => 1,
                    "east" => 2,
                    "west" => 3,
                    _ => 0,
                };
                Some((top, facing))
            } else {
                None
            };

            let (wall_fence_kind, wall_fence_conn) = if solid_class == SolidClass::PartialBox {
                if sname.ends_with("_wall") {
                    any_walls = true;
                    let conn = |dir: &str| matches!(bs.prop(dir), Some(v) if !v.is_empty() && v != "none");
                    let n = conn("north");
                    let so = conn("south");
                    let w = conn("west");
                    let e = conn("east");
                    let skip_post = bs.prop("up") == Some("false")
                        && ((w && e && !n && !so) || (n && so && !w && !e));
                    let bits = (n as u8)
                        | ((so as u8) << 1)
                        | ((w as u8) << 2)
                        | ((e as u8) << 3)
                        | ((skip_post as u8) << 4);
                    (1, bits)
                } else if sname.ends_with("_fence") {
                    any_fences = true;
                    let n = bs.prop("north") == Some("true");
                    let so = bs.prop("south") == Some("true");
                    let w = bs.prop("west") == Some("true");
                    let e = bs.prop("east") == Some("true");
                    let bits = (n as u8) | ((so as u8) << 1) | ((w as u8) << 2) | ((e as u8) << 3);
                    (2, bits)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            };

            if solid_class == SolidClass::FullLeaves {
                any_leaves = true;
            }
            if is_glass_cube {
                any_glass = true;
            }

            let is_water = bs.is_water_tag || bs.waterlogged;
            let is_water_brush = is_water && is_water_brush_eligible(bs);
            let is_lava = !is_water && (sname == "lava" || sname == "flowing_lava");
            if is_water {
                any_water = true;
                if is_water_brush {
                    any_water_brush = true;
                } else {
                    any_non_brush_water = true;
                }
            }
            if is_lava {
                any_lava = true;
            }

            let mut is_unmerged_partial = solid_class == SolidClass::PartialBox
                && snow_layers == 0
                && slab_kind == 255
                && stair_info.is_none()
                && wall_fence_kind == 0
                && !is_glass_cube;
            let mut unmerged_boxes = if is_unmerged_partial {
                any_partial = true;
                block_local_collision_boxes(bs)
            } else {
                Vec::new()
            };

            if solid_class == SolidClass::FullOpaque {
                if let Some(models) = resolved_models {
                    if let Some(model) = models.get(pid) {
                        match compute_model_normalized_aabb(model) {
                            None => {
                                solid_class = SolidClass::Passable;
                                is_passable = true;
                            }
                            Some(aabb) => {
                                let dx = (aabb[3] - aabb[0]).max(0.0);
                                let dy = (aabb[4] - aabb[1]).max(0.0);
                                let dz = (aabb[5] - aabb[2]).max(0.0);
                                if dy <= (5.0 / 16.0) && (dx * dz) <= 0.45 {
                                    solid_class = SolidClass::Passable;
                                    is_passable = true;
                                } else if dx * dy * dz < 0.85
                                    && (dx < 0.90 || dy < 0.90 || dz < 0.90)
                                    && dx > 0.02
                                    && dy > 0.02
                                    && dz > 0.02
                                {
                                    solid_class = SolidClass::PartialBox;
                                    is_unmerged_partial = true;
                                    any_partial = true;
                                    unmerged_boxes = vec![(aabb, "partial", false)];
                                }
                            }
                        }
                    }
                }
            }

            let hazard_kind = if sname == "fire" {
                any_hazard = true;
                1
            } else if sname == "soul_fire" {
                any_hazard = true;
                2
            } else if (sname == "campfire" || sname == "soul_campfire") && bs.prop("lit") != Some("false") {
                any_hazard = true;
                3
            } else if sname == "magma_block" {
                any_hazard = true;
                4
            } else if sname == "cactus" {
                any_hazard = true;
                5
            } else {
                0
            };

            let ladder_facing = if sname == "ladder" {
                let f = match bs.prop("facing").unwrap_or("north") {
                    "south" => 1,
                    "east" => 2,
                    "west" => 3,
                    _ => 0,
                };
                ladder_sides_mask |= 1u8 << f;
                f
            } else {
                255
            };

            let vine_wall_mask = if sname == "vine" {
                let m = u8::from(bs.prop("south") == Some("true"))
                    | (u8::from(bs.prop("north") == Some("true")) << 1)
                    | (u8::from(bs.prop("west") == Some("true")) << 2)
                    | (u8::from(bs.prop("east") == Some("true")) << 3);
                vine_sides_mask |= m;
                m
            } else {
                0
            };

            let zone_kind_idx = match surface_zone_kind(bs) {
                Some(zk) if zk != "water" && zk != "lava" && zk != "ladder" => {
                    if zk == "vine" && vine_wall_mask != 0 {
                        255
                    } else if let Some(pos) = MERGEABLE_ZONE_KINDS.iter().position(|&k| k == zk) {
                        zone_kinds_mask |= 1u16 << pos;
                        pos as u8
                    } else {
                        255
                    }
                }
                _ => 255,
            };

            PaletteColMeta {
                solid_class,
                is_passable,
                is_glass_cube,
                snow_layers,
                slab_kind,
                stair_info,
                wall_fence_kind,
                wall_fence_conn,
                is_water,
                is_water_brush,
                is_lava,
                hazard_kind,
                zone_kind_idx,
                ladder_facing,
                vine_wall_mask,
                is_unmerged_partial,
                unmerged_boxes,
                surface_prop: classify_surface_prop(bs),
            }
        })
        .collect();

    // 1. Build compact 1-byte/voxel flags array in parallel
    let mut flags = vec![0u8; total_voxels];
    let slice_sz = sx * sz;
    if slice_sz > 0 {
        flags
            .par_chunks_mut(slice_sz)
            .enumerate()
            .for_each(|(gy, dst_slice)| {
                let src_slice = &grid.blocks[gy * slice_sz..(gy + 1) * slice_sz];
                for i in 0..slice_sz {
                    let m = &pal_meta[src_slice[i] as usize];
                    let mut f = 0u8;
                    match m.solid_class {
                        SolidClass::FullOpaque => f |= FLAG_FULL_OPAQUE,
                        SolidClass::FullLeaves => f |= FLAG_LEAVES,
                        _ => {}
                    }
                    if m.is_glass_cube {
                        f |= FLAG_GLASS_CUBE;
                    }
                    if m.is_passable {
                        f |= FLAG_PASSABLE;
                    }
                    if m.is_water {
                        f |= FLAG_WATER;
                    }
                    if m.is_water_brush {
                        f |= FLAG_WATER_BRUSH;
                    }
                    if m.is_lava {
                        f |= FLAG_LAVA;
                    }
                    dst_slice[i] = f;
                }
            });
    }

    // Exposed solid voxel mask: mark exposed full-opaque voxels in parallel without a separate Vec<bool>
    let shell_flags: Vec<u8> = (0..sy)
        .into_par_iter()
        .flat_map_iter(|gy| {
            let y_base = gy * slice_sz;
            let mut out = vec![0u8; slice_sz];
            for gz in 0..sz {
                let row = gz * sx;
                for gx in 0..sx {
                    let idx = y_base + row + gx;
                    if (flags[idx] & FLAG_FULL_OPAQUE) == 0 {
                        continue;
                    }
                    let enclosed = gx > 0
                        && gx + 1 < sx
                        && gy > 0
                        && gy + 1 < sy
                        && gz > 0
                        && gz + 1 < sz
                        && (flags[idx - 1] & FLAG_FULL_OPAQUE) != 0
                        && (flags[idx + 1] & FLAG_FULL_OPAQUE) != 0
                        && (flags[idx - sx] & FLAG_FULL_OPAQUE) != 0
                        && (flags[idx + sx] & FLAG_FULL_OPAQUE) != 0
                        && (flags[idx - slice_sz] & FLAG_FULL_OPAQUE) != 0
                        && (flags[idx + slice_sz] & FLAG_FULL_OPAQUE) != 0;
                    if !enclosed {
                        out[row + gx] = FLAG_SHELL_OPAQUE;
                    }
                }
            }
            out
        })
        .collect();
    for (f, sf) in flags.iter_mut().zip(shell_flags) {
        *f |= sf;
    }

    let is_top_walkable = |gx0: usize, gx1: usize, top_gy: usize, gz0: usize, gz1: usize| -> bool {
        if top_gy >= sy {
            return true;
        }
        for gz in gz0..gz1 {
            let row = (top_gy * sz + gz) * sx;
            for gx in gx0..gx1 {
                if (flags[row + gx] & FLAG_PASSABLE) != 0 {
                    return true;
                }
            }
        }
        false
    };

    // 2. Process all macro chunks in parallel via rayon
    let ncx = sx.div_ceil(chunk_size);
    let ncz = sz.div_ceil(chunk_size);
    let mut chunk_coords = Vec::with_capacity(ncx * ncz);
    for cz_i in 0..ncz {
        for cx_i in 0..ncx {
            chunk_coords.push((cx_i, cz_i));
        }
    }

    struct ChunkColOut {
        key: (i32, i32),
        boxes: Vec<CollisionBox>,
        hazards: Vec<HazardBox>,
    }

    let chunk_outs: Vec<ChunkColOut> = chunk_coords
        .into_par_iter()
        .map(|(cx_i, cz_i)| {
            let x0 = cx_i * chunk_size;
            let x1 = (x0 + chunk_size).min(sx);
            let z0 = cz_i * chunk_size;
            let z1 = (z0 + chunk_size).min(sz);
            let cw = x1 - x0;
            let cd = z1 - z0;
            let mut boxes = Vec::new();
            let mut hazards = Vec::new();
            let mut local_mask = vec![false; cw * sy * cd];

            let mut extract_greedy_flag = |flag_bit: u8, block_class: &'static str, is_opaque_roof: bool| {
                for sp in SurfaceProp::ALL {
                    let mut any_bit = false;
                    for gy in 0..sy {
                        for lz in 0..cd {
                            let gz = z0 + lz;
                            let src_off = (gy * sz + gz) * sx + x0;
                            let dst_off = (gy * cd + lz) * cw;
                            for lx in 0..cw {
                                let hit = (flags[src_off + lx] & flag_bit) != 0
                                    && pal_meta[grid.blocks[src_off + lx] as usize].surface_prop == sp;
                                local_mask[dst_off + lx] = hit;
                                any_bit |= hit;
                            }
                        }
                    }
                    if !any_bit {
                        continue;
                    }
                    for [bx0, by0, bz0, bx1, by1, bz1] in greedy_mesh_3d(&mut local_mask, cw, sy, cd) {
                        let wx = (c.min_x + (x0 + bx0) as i32) as f64;
                        let wy = (c.min_y + by0 as i32) as f64;
                        let wz = (c.min_z + (z0 + bz0) as i32) as f64;
                        let sb = mc_box_to_source(
                            wx,
                            wy,
                            wz,
                            [
                                0.0,
                                0.0,
                                0.0,
                                (bx1 - bx0) as f64,
                                (by1 - by0) as f64,
                                (bz1 - bz0) as f64,
                            ],
                            origin_mc,
                            scale,
                        );
                        boxes.push(CollisionBox {
                            box_coords: sb,
                            block_class: Cow::Borrowed(block_class),
                            is_walkable: is_top_walkable(x0 + bx0, x0 + bx1, by1, z0 + bz0, z0 + bz1),
                            is_opaque_roof,
                            surface_prop: sp,
                        });
                    }
                }
            };

            extract_greedy_flag(FLAG_SHELL_OPAQUE, "full", true);
            if any_leaves {
                extract_greedy_flag(FLAG_LEAVES, "leaves", false);
            }
            if any_glass {
                extract_greedy_flag(FLAG_GLASS_CUBE, "glass", false);
            }

            // Snow[layers=2..=8]: single scan per Y slice into lazy layer masks
            if any_snow {
                for gy in 0..sy {
                    let wy = (c.min_y + gy as i32) as f64;
                    let mut layer_masks: [Vec<bool>; 7] = Default::default();
                    for lz in 0..cd {
                        let gz = z0 + lz;
                        let row = (gy * sz + gz) * sx + x0;
                        for lx in 0..cw {
                            let l = pal_meta[grid.blocks[row + lx] as usize].snow_layers;
                            if (2..=8).contains(&l) {
                                let li = (l - 2) as usize;
                                if layer_masks[li].is_empty() {
                                    layer_masks[li] = vec![false; cw * cd];
                                }
                                layer_masks[li][lz * cw + lx] = true;
                            }
                        }
                    }
                    for (li, mask) in layer_masks.iter_mut().enumerate() {
                        if mask.is_empty() {
                            continue;
                        }
                        let layers_val = (li + 2) as u8;
                        let h = if layers_val >= 8 {
                            1.0
                        } else {
                            (layers_val as f64 - 1.0) / 8.0
                        };
                        for [bx0, _, bz0, bx1, _, bz1] in greedy_mesh_3d(mask, cw, 1, cd) {
                            let wx = (c.min_x + (x0 + bx0) as i32) as f64;
                            let wz = (c.min_z + (z0 + bz0) as i32) as f64;
                            let sb = mc_box_to_source(
                                wx,
                                wy,
                                wz,
                                [0.0, 0.0, 0.0, (bx1 - bx0) as f64, h, (bz1 - bz0) as f64],
                                origin_mc,
                                scale,
                            );
                            boxes.push(CollisionBox {
                                box_coords: sb,
                                block_class: Cow::Borrowed("snow"),
                                is_walkable: is_top_walkable(x0 + bx0, x0 + bx1, gy + 1, z0 + bz0, z0 + bz1),
                                is_opaque_roof: false,
                                surface_prop: SurfaceProp::Snow,
                            });
                        }
                    }
                }
            }

            // Horizontal slabs
            if any_slab {
                const SLAB_HEIGHTS: [(f64, f64); 3] = [(0.0, 0.5), (0.5, 1.0), (0.0, 15.0 / 16.0)];
                for gy in 0..sy {
                    let wy = (c.min_y + gy as i32) as f64;
                    let mut slab_masks: [[Vec<bool>; 3]; SurfaceProp::ALL.len()] = Default::default();
                    for lz in 0..cd {
                        let gz = z0 + lz;
                        let row = (gy * sz + gz) * sx + x0;
                        for lx in 0..cw {
                            let m = &pal_meta[grid.blocks[row + lx] as usize];
                            let kind_idx = m.slab_kind;
                            if kind_idx < 3 {
                                let sp_i = m.surface_prop as usize;
                                let ki = kind_idx as usize;
                                if slab_masks[sp_i][ki].is_empty() {
                                    slab_masks[sp_i][ki] = vec![false; cw * cd];
                                }
                                slab_masks[sp_i][ki][lz * cw + lx] = true;
                            }
                        }
                    }
                    for (sp_i, kind_masks) in slab_masks.iter_mut().enumerate() {
                        let sp = SurfaceProp::ALL[sp_i];
                        for (kind_idx, mask) in kind_masks.iter_mut().enumerate() {
                            if mask.is_empty() {
                                continue;
                            }
                            let (sy0, sy1) = SLAB_HEIGHTS[kind_idx];
                            for [bx0, _, bz0, bx1, _, bz1] in greedy_mesh_3d(mask, cw, 1, cd) {
                                let wx = (c.min_x + (x0 + bx0) as i32) as f64;
                                let wz = (c.min_z + (z0 + bz0) as i32) as f64;
                                let sb = mc_box_to_source(
                                    wx,
                                    wy,
                                    wz,
                                    [0.0, sy0, 0.0, (bx1 - bx0) as f64, sy1, (bz1 - bz0) as f64],
                                    origin_mc,
                                    scale,
                                );
                                boxes.push(CollisionBox {
                                    box_coords: sb,
                                    block_class: Cow::Borrowed("slab"),
                                    is_walkable: is_top_walkable(x0 + bx0, x0 + bx1, gy + 1, z0 + bz0, z0 + bz1),
                                    is_opaque_roof: true,
                                    surface_prop: sp,
                                });
                            }
                        }
                    }
                }
            }

            // Stairs
            if any_stairs {
                for gy in 0..sy {
                    let wy = (c.min_y + gy as i32) as f64;
                    let mut base_masks: [[Vec<bool>; 2]; SurfaceProp::ALL.len()] = Default::default();
                    let mut step_masks: Vec<[[Vec<bool>; 4]; 2]> = vec![Default::default(); SurfaceProp::ALL.len()];
                    for lz in 0..cd {
                        let gz = z0 + lz;
                        let row = (gy * sz + gz) * sx + x0;
                        for lx in 0..cw {
                            let m = &pal_meta[grid.blocks[row + lx] as usize];
                            if let Some((top_u, facing_u)) = m.stair_info {
                                let sp_i = m.surface_prop as usize;
                                let top = top_u as usize;
                                let facing = facing_u as usize;
                                if base_masks[sp_i][top].is_empty() {
                                    base_masks[sp_i][top] = vec![false; cw * cd];
                                }
                                base_masks[sp_i][top][lz * cw + lx] = true;
                                if step_masks[sp_i][top][facing].is_empty() {
                                    step_masks[sp_i][top][facing] = vec![false; cw * cd];
                                }
                                step_masks[sp_i][top][facing][lz * cw + lx] = true;
                            }
                        }
                    }
                    for (sp_i, sp_bases) in base_masks.iter_mut().enumerate() {
                        let sp = SurfaceProp::ALL[sp_i];
                        for (top, mask) in sp_bases.iter_mut().enumerate() {
                            if mask.is_empty() {
                                continue;
                            }
                            let (by0, by1) = if top == 1 { (0.5, 1.0) } else { (0.0, 0.5) };
                            for [bx0, _, bz0, bx1, _, bz1] in greedy_mesh_3d(mask, cw, 1, cd) {
                                let wx = (c.min_x + (x0 + bx0) as i32) as f64;
                                let wz = (c.min_z + (z0 + bz0) as i32) as f64;
                                let sb = mc_box_to_source(
                                    wx,
                                    wy,
                                    wz,
                                    [0.0, by0, 0.0, (bx1 - bx0) as f64, by1, (bz1 - bz0) as f64],
                                    origin_mc,
                                    scale,
                                );
                                boxes.push(CollisionBox {
                                    box_coords: sb,
                                    block_class: Cow::Borrowed("stairs"),
                                    is_walkable: is_top_walkable(x0 + bx0, x0 + bx1, gy + 1, z0 + bz0, z0 + bz1),
                                    is_opaque_roof: true,
                                    surface_prop: sp,
                                });
                            }
                        }
                    }
                    for (sp_i, sp_steps) in step_masks.iter().enumerate() {
                        let sp = SurfaceProp::ALL[sp_i];
                        for (top, facing_masks) in sp_steps.iter().enumerate() {
                            let (sy0, sy1) = if top == 1 { (0.0, 0.5) } else { (0.5, 1.0) };
                            for (facing, mask) in facing_masks.iter().enumerate() {
                                if mask.is_empty() {
                                    continue;
                                }
                                if facing == 0 || facing == 1 {
                                    let (sz0, sz1) = if facing == 1 { (0.5, 1.0) } else { (0.0, 0.5) };
                                    for lz in 0..cd {
                                        let wz = (c.min_z + (z0 + lz) as i32) as f64;
                                        let mut lx = 0;
                                        while lx < cw {
                                            if !mask[lz * cw + lx] {
                                                lx += 1;
                                                continue;
                                            }
                                            let mut w = 1;
                                            while lx + w < cw && mask[lz * cw + lx + w] {
                                                w += 1;
                                            }
                                            let wx = (c.min_x + (x0 + lx) as i32) as f64;
                                            let sb = mc_box_to_source(
                                                wx,
                                                wy,
                                                wz,
                                                [0.0, sy0, sz0, w as f64, sy1, sz1],
                                                origin_mc,
                                                scale,
                                            );
                                            boxes.push(CollisionBox {
                                                box_coords: sb,
                                                block_class: Cow::Borrowed("stairs"),
                                                is_walkable: is_top_walkable(x0 + lx, x0 + lx + w, gy + 1, z0 + lz, z0 + lz + 1),
                                                is_opaque_roof: true,
                                                surface_prop: sp,
                                            });
                                            lx += w;
                                        }
                                    }
                                } else {
                                    let (sx0, sx1) = if facing == 2 { (0.5, 1.0) } else { (0.0, 0.5) };
                                    for lx in 0..cw {
                                        let wx = (c.min_x + (x0 + lx) as i32) as f64;
                                        let mut lz = 0;
                                        while lz < cd {
                                            if !mask[lz * cw + lx] {
                                                lz += 1;
                                                continue;
                                            }
                                            let mut d = 1;
                                            while lz + d < cd && mask[(lz + d) * cw + lx] {
                                                d += 1;
                                            }
                                            let wz = (c.min_z + (z0 + lz) as i32) as f64;
                                            let sb = mc_box_to_source(
                                                wx,
                                                wy,
                                                wz,
                                                [sx0, sy0, 0.0, sx1, sy1, d as f64],
                                                origin_mc,
                                                scale,
                                            );
                                            boxes.push(CollisionBox {
                                                box_coords: sb,
                                                block_class: Cow::Borrowed("stairs"),
                                                is_walkable: is_top_walkable(x0 + lx, x0 + lx + 1, gy + 1, z0 + lz, z0 + lz + d),
                                                is_opaque_roof: true,
                                                surface_prop: sp,
                                            });
                                            lz += d;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Walls & Fences
            for (target_kind, active) in [(1u8, any_walls), (2u8, any_fences)] {
                if !active {
                    continue;
                }
                let bclass = if target_kind == 1 { "wall" } else { "fence" };
                let (arm_lo, arm_hi, post_lo, post_hi) = if target_kind == 1 {
                    (0.3125, 0.6875, 0.25, 0.75)
                } else {
                    (0.4375, 0.5625, 0.375, 0.625)
                };
                let mut post_mask = vec![false; cw * sy * cd];
                let mut x_arm_mask = vec![false; (cw * 2) * sy * cd];
                let mut z_arm_mask = vec![false; cw * sy * (cd * 2)];

                for sp in SurfaceProp::ALL {
                    post_mask.fill(false);
                    x_arm_mask.fill(false);
                    z_arm_mask.fill(false);
                    let mut any_found = false;

                    for gy in 0..sy {
                        for lz in 0..cd {
                            let gz = z0 + lz;
                            let row = (gy * sz + gz) * sx + x0;
                            for lx in 0..cw {
                                let m = &pal_meta[grid.blocks[row + lx] as usize];
                                if m.wall_fence_kind != target_kind || m.surface_prop != sp {
                                    continue;
                                }
                                any_found = true;
                                let conn = m.wall_fence_conn;
                                if (conn & 4) != 0 {
                                    x_arm_mask[(gy * cd + lz) * (cw * 2) + (lx * 2)] = true;
                                }
                                if (conn & 8) != 0 {
                                    x_arm_mask[(gy * cd + lz) * (cw * 2) + (lx * 2 + 1)] = true;
                                }
                                if (conn & 1) != 0 {
                                    z_arm_mask[(gy * (cd * 2) + (lz * 2)) * cw + lx] = true;
                                }
                                if (conn & 2) != 0 {
                                    z_arm_mask[(gy * (cd * 2) + (lz * 2 + 1)) * cw + lx] = true;
                                }
                                if (conn & 16) == 0 {
                                    post_mask[(gy * cd + lz) * cw + lx] = true;
                                }
                            }
                        }
                    }
                    if !any_found {
                        continue;
                    }

                    // Merge X-arms along X then Y
                    for gy in 0..sy {
                        for lz in 0..cd {
                            let wz = (c.min_z + (z0 + lz) as i32) as f64;
                            let mut hx = 0;
                            while hx < cw * 2 {
                                if !x_arm_mask[(gy * cd + lz) * (cw * 2) + hx] {
                                    hx += 1;
                                    continue;
                                }
                                let mut w = 1;
                                while hx + w < cw * 2 && x_arm_mask[(gy * cd + lz) * (cw * 2) + hx + w] {
                                    w += 1;
                                }
                                let mut h = 1;
                                'y_xarm: while gy + h < sy {
                                    for dx in 0..w {
                                        if !x_arm_mask[((gy + h) * cd + lz) * (cw * 2) + hx + dx] {
                                            break 'y_xarm;
                                        }
                                    }
                                    h += 1;
                                }
                                for dy in 0..h {
                                    for dx in 0..w {
                                        x_arm_mask[((gy + dy) * cd + lz) * (cw * 2) + hx + dx] = false;
                                    }
                                }
                                let wx = (c.min_x + (x0 as i32)) as f64 + (hx as f64) * 0.5;
                                let wy = (c.min_y + gy as i32) as f64;
                                let gx0 = x0 + hx / 2;
                                let gx1 = x0 + (hx + w - 1) / 2 + 1;
                                let all_above_passable = gy + h >= sy
                                    || (gx0..gx1).all(|gx| {
                                        (flags[((gy + h) * sz + (z0 + lz)) * sx + gx] & FLAG_PASSABLE) != 0
                                    });
                                let extra_h = if all_above_passable { 0.5 } else { 0.0 };
                                let sb = mc_box_to_source(
                                    wx,
                                    wy,
                                    wz,
                                    [0.0, 0.0, arm_lo, (w as f64) * 0.5, (h as f64) + extra_h, arm_hi],
                                    origin_mc,
                                    scale,
                                );
                                boxes.push(CollisionBox {
                                    box_coords: sb,
                                    block_class: Cow::Borrowed(bclass),
                                    is_walkable: is_top_walkable(gx0, gx1, gy + h, z0 + lz, z0 + lz + 1),
                                    is_opaque_roof: false,
                                    surface_prop: sp,
                                });
                                hx += w;
                            }
                        }
                    }
                    // Merge Z-arms along Z then Y
                    for gy in 0..sy {
                        let wy = (c.min_y + gy as i32) as f64;
                        for lx in 0..cw {
                            let wx = (c.min_x + (x0 + lx) as i32) as f64;
                            let mut hz = 0;
                            while hz < cd * 2 {
                                if !z_arm_mask[(gy * (cd * 2) + hz) * cw + lx] {
                                    hz += 1;
                                    continue;
                                }
                                let mut d = 1;
                                while hz + d < cd * 2 && z_arm_mask[(gy * (cd * 2) + hz + d) * cw + lx] {
                                    d += 1;
                                }
                                let mut h = 1;
                                'y_zarm: while gy + h < sy {
                                    for dz in 0..d {
                                        if !z_arm_mask[((gy + h) * (cd * 2) + hz + dz) * cw + lx] {
                                            break 'y_zarm;
                                        }
                                    }
                                    h += 1;
                                }
                                for dy in 0..h {
                                    for dz in 0..d {
                                        z_arm_mask[((gy + dy) * (cd * 2) + hz + dz) * cw + lx] = false;
                                    }
                                }
                                let wz = (c.min_z + (z0 as i32)) as f64 + (hz as f64) * 0.5;
                                let gz0 = z0 + hz / 2;
                                let gz1 = z0 + (hz + d - 1) / 2 + 1;
                                let all_above_passable = gy + h >= sy
                                    || (gz0..gz1).all(|gz| {
                                        (flags[((gy + h) * sz + gz) * sx + (x0 + lx)] & FLAG_PASSABLE) != 0
                                    });
                                let extra_h = if all_above_passable { 0.5 } else { 0.0 };
                                let sb = mc_box_to_source(
                                    wx,
                                    wy,
                                    wz,
                                    [arm_lo, 0.0, 0.0, arm_hi, (h as f64) + extra_h, (d as f64) * 0.5],
                                    origin_mc,
                                    scale,
                                );
                                boxes.push(CollisionBox {
                                    box_coords: sb,
                                    block_class: Cow::Borrowed(bclass),
                                    is_walkable: is_top_walkable(x0 + lx, x0 + lx + 1, gy + h, gz0, gz1),
                                    is_opaque_roof: false,
                                    surface_prop: sp,
                                });
                                hz += d;
                            }
                        }
                    }
                    // Merge center posts vertically along Y
                    for gy in 0..sy {
                        let wy = (c.min_y + gy as i32) as f64;
                        for lz in 0..cd {
                            let wz = (c.min_z + (z0 + lz) as i32) as f64;
                            for lx in 0..cw {
                                if !post_mask[(gy * cd + lz) * cw + lx] {
                                    continue;
                                }
                                let mut h = 1;
                                while gy + h < sy && post_mask[((gy + h) * cd + lz) * cw + lx] {
                                    h += 1;
                                }
                                for dy in 0..h {
                                    post_mask[((gy + dy) * cd + lz) * cw + lx] = false;
                                }
                                let wx = (c.min_x + (x0 + lx) as i32) as f64;
                                let above_passable = gy + h >= sy
                                    || (flags[((gy + h) * sz + (z0 + lz)) * sx + (x0 + lx)] & FLAG_PASSABLE) != 0;
                                let extra_h = if above_passable { 0.5 } else { 0.0 };
                                let sb = mc_box_to_source(
                                    wx,
                                    wy,
                                    wz,
                                    [post_lo, 0.0, post_lo, post_hi, (h as f64) + extra_h, post_hi],
                                    origin_mc,
                                    scale,
                                );
                                boxes.push(CollisionBox {
                                    box_coords: sb,
                                    block_class: Cow::Borrowed(bclass),
                                    is_walkable: is_top_walkable(x0 + lx, x0 + lx + 1, gy + h, z0 + lz, z0 + lz + 1),
                                    is_opaque_roof: false,
                                    surface_prop: sp,
                                });
                            }
                        }
                    }
                }
            }

            // Unmerged partial boxes & hazards inside this chunk
            if any_partial || any_hazard {
                for gy in 0..sy {
                    let wy = c.min_y + gy as i32;
                    for lz in 0..cd {
                        let gz = z0 + lz;
                        let wz = c.min_z + gz as i32;
                        let row = (gy * sz + gz) * sx + x0;
                        for lx in 0..cw {
                            let gx = x0 + lx;
                            let wx = c.min_x + gx as i32;
                            let m = &pal_meta[grid.blocks[row + lx] as usize];
                            if m.is_unmerged_partial {
                                let walkable = is_top_walkable(gx, gx + 1, gy + 1, gz, gz + 1);
                                for &(local, bclass, is_roof) in &m.unmerged_boxes {
                                    let sb = mc_box_to_source(wx as f64, wy as f64, wz as f64, local, origin_mc, scale);
                                    boxes.push(CollisionBox {
                                        box_coords: sb,
                                        block_class: Cow::Borrowed(bclass),
                                        is_walkable: walkable,
                                        is_opaque_roof: is_roof,
                                        surface_prop: m.surface_prop,
                                    });
                                }
                            }
                            match m.hazard_kind {
                                1 => hazards.push(HazardBox {
                                    kind: "fire".to_string(),
                                    box_coords: mc_box_to_source(wx as f64, wy as f64, wz as f64, [0.1, 0.0, 0.1, 0.9, 0.85, 0.9], origin_mc, scale),
                                    damage: 12,
                                    damage_type: 8,
                                }),
                                2 => hazards.push(HazardBox {
                                    kind: "soul_fire".to_string(),
                                    box_coords: mc_box_to_source(wx as f64, wy as f64, wz as f64, [0.1, 0.0, 0.1, 0.9, 0.85, 0.9], origin_mc, scale),
                                    damage: 16,
                                    damage_type: 8,
                                }),
                                3 => hazards.push(HazardBox {
                                    kind: "campfire".to_string(),
                                    box_coords: mc_box_to_source(wx as f64, wy as f64, wz as f64, [0.1, 0.0, 0.1, 0.9, 0.85, 0.9], origin_mc, scale),
                                    damage: 12,
                                    damage_type: 8,
                                }),
                                4 => hazards.push(HazardBox {
                                    kind: "magma".to_string(),
                                    box_coords: mc_box_to_source(wx as f64, wy as f64, wz as f64, [0.0, 0.95, 0.0, 1.0, 1.15, 1.0], origin_mc, scale),
                                    damage: 10,
                                    damage_type: 8,
                                }),
                                5 => hazards.push(HazardBox {
                                    kind: "cactus".to_string(),
                                    box_coords: mc_box_to_source(wx as f64, wy as f64, wz as f64, [0.0, 0.0, 0.0, 1.0, 1.0, 1.0], origin_mc, scale),
                                    damage: 8,
                                    damage_type: 16,
                                }),
                                _ => {}
                            }
                        }
                    }
                }
            }

            ChunkColOut {
                key: (cx_i as i32, cz_i as i32),
                boxes,
                hazards,
            }
        })
        .collect();

    let total_boxes: usize = chunk_outs.iter().map(|co| co.boxes.len()).sum();
    let mut per_chunk: HashMap<(i32, i32), Vec<CollisionBox>> = HashMap::with_capacity(chunk_outs.len());
    let mut all_solid_boxes: Vec<CollisionBox> = Vec::with_capacity(total_boxes);
    let mut hazard_boxes: Vec<HazardBox> = Vec::new();

    for co in chunk_outs {
        all_solid_boxes.extend(co.boxes.iter().cloned());
        per_chunk.insert(co.key, co.boxes);
        hazard_boxes.extend(co.hazards);
    }

    // 3. Buried opaque core occluders on 4x4x4 macro grid (parallel, zero full-volume erosion buffers)
    // A voxel survives 2 passes of 6-neighbor erosion iff all voxels within Manhattan distance <= 2 are FULL_OPAQUE.
    let mut core_occluders: Vec<[f64; 6]> = Vec::new();
    if sx >= 8 && sy >= 8 && sz >= 8 {
        let csx = sx / 4;
        let csy = sy / 4;
        let csz = sz / 4;
        let core_rows: Vec<(usize, Vec<bool>)> = (1..csy.saturating_sub(1))
            .into_par_iter()
            .map(|cy| {
                let mut plane = vec![false; csx * csz];
                for cz in 1..csz.saturating_sub(1) {
                    for cx in 1..csx.saturating_sub(1) {
                        let gx0 = cx * 4;
                        let gy0 = cy * 4;
                        let gz0 = cz * 4;
                        if gx0 < 2 || gx0 + 5 >= sx || gy0 < 2 || gy0 + 5 >= sy || gz0 < 2 || gz0 + 5 >= sz {
                            continue;
                        }
                        // Check the 4x4x4 core plus its 2-voxel Manhattan halo (early-out on first non-opaque)
                        let mut all_solid = true;
                        'cell: for dy in -2isize..=5 {
                            let wy = (gy0 as isize + dy) as usize;
                            let y_dist = if dy < 0 { -dy } else if dy > 3 { dy - 3 } else { 0 };
                            for dz in -2isize..=5 {
                                let z_dist = if dz < 0 { -dz } else if dz > 3 { dz - 3 } else { 0 };
                                if y_dist + z_dist > 2 {
                                    continue;
                                }
                                let wz = (gz0 as isize + dz) as usize;
                                let row = (wy * sz + wz) * sx;
                                let max_x_dist = 2 - y_dist - z_dist;
                                let dx_min = -max_x_dist;
                                let dx_max = 3 + max_x_dist;
                                for dx in dx_min..=dx_max {
                                    let wx = (gx0 as isize + dx) as usize;
                                    if (flags[row + wx] & FLAG_FULL_OPAQUE) == 0 {
                                        all_solid = false;
                                        break 'cell;
                                    }
                                }
                            }
                        }
                        if all_solid {
                            plane[cz * csx + cx] = true;
                        }
                    }
                }
                (cy, plane)
            })
            .collect();

        let mut core_mask = vec![false; csx * csy * csz];
        for (cy, plane) in core_rows {
            let off = cy * csx * csz;
            core_mask[off..off + csx * csz].copy_from_slice(&plane);
        }
        for [bx0, by0, bz0, bx1, by1, bz1] in select_top_core_occluders(
            greedy_mesh_3d(&mut core_mask, csx, csy, csz),
            MAX_CORE_OCCLUDERS,
            MIN_CORE_OCCLUDER_CELLS,
        ) {
            let wx = (c.min_x + (bx0 * 4) as i32) as f64;
            let wy = (c.min_y + (by0 * 4) as i32) as f64;
            let wz = (c.min_z + (bz0 * 4) as i32) as f64;
            let sb = mc_box_to_source(
                wx,
                wy,
                wz,
                [
                    0.0,
                    0.0,
                    0.0,
                    ((bx1 - bx0) * 4) as f64,
                    ((by1 - by0) * 4) as f64,
                    ((bz1 - bz0) * 4) as f64,
                ],
                origin_mc,
                scale,
            );
            core_occluders.push(sb);
        }
    }

    // 4. Greedy-merge water, lava, ladders, and all surface physics zones using one reusable mask buffer
    let active_zone_indices: Vec<usize> = (0..MERGEABLE_ZONE_KINDS.len())
        .filter(|&i| (zone_kinds_mask & (1u16 << i)) != 0)
        .collect();

    let mut mask = if any_water || any_lava || !active_zone_indices.is_empty() {
        vec![false; total_voxels]
    } else {
        Vec::new()
    };

    let water_pools = if any_water_brush {
        if slice_sz > 0 {
            mask.par_chunks_mut(slice_sz)
                .enumerate()
                .for_each(|(gy, dst)| {
                    let src = &flags[gy * slice_sz..(gy + 1) * slice_sz];
                    for i in 0..slice_sz {
                        dst[i] = (src[i] & FLAG_WATER_BRUSH) != 0;
                    }
                });
        }
        greedy_mesh_3d(&mut mask, sx, sy, sz)
            .into_iter()
            .map(|[bx0, by0, bz0, bx1, by1, bz1]| {
                mc_box_to_source(
                    (c.min_x + bx0 as i32) as f64,
                    (c.min_y + by0 as i32) as f64,
                    (c.min_z + bz0 as i32) as f64,
                    [
                        0.0,
                        0.0,
                        0.0,
                        (bx1 - bx0) as f64,
                        ((by1 - by0) as f64) - 0.11,
                        (bz1 - bz0) as f64,
                    ],
                    origin_mc,
                    scale,
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let water_physics_extra = if any_water && any_non_brush_water {
        if slice_sz > 0 {
            mask.par_chunks_mut(slice_sz)
                .enumerate()
                .for_each(|(gy, dst)| {
                    let src = &flags[gy * slice_sz..(gy + 1) * slice_sz];
                    for i in 0..slice_sz {
                        dst[i] = (src[i] & FLAG_WATER) != 0;
                    }
                });
        }
        Some(
            greedy_mesh_3d(&mut mask, sx, sy, sz)
                .into_iter()
                .map(|[bx0, by0, bz0, bx1, by1, bz1]| {
                    mc_box_to_source(
                        (c.min_x + bx0 as i32) as f64,
                        (c.min_y + by0 as i32) as f64,
                        (c.min_z + bz0 as i32) as f64,
                        [
                            0.0,
                            0.0,
                            0.0,
                            (bx1 - bx0) as f64,
                            ((by1 - by0) as f64) - 0.11,
                            (bz1 - bz0) as f64,
                        ],
                        origin_mc,
                        scale,
                    )
                })
                .collect::<Vec<[f64; 6]>>(),
        )
    } else {
        None
    };

    let lava_results = if any_lava {
        if slice_sz > 0 {
            mask.par_chunks_mut(slice_sz)
                .enumerate()
                .for_each(|(gy, dst)| {
                    let src = &flags[gy * slice_sz..(gy + 1) * slice_sz];
                    for i in 0..slice_sz {
                        dst[i] = (src[i] & FLAG_LAVA) != 0;
                    }
                });
        }
        greedy_mesh_3d(&mut mask, sx, sy, sz)
            .into_iter()
            .map(|[bx0, by0, bz0, bx1, by1, bz1]| {
                mc_box_to_source(
                    (c.min_x + bx0 as i32) as f64,
                    (c.min_y + by0 as i32) as f64,
                    (c.min_z + bz0 as i32) as f64,
                    [
                        0.0,
                        0.0,
                        0.0,
                        (bx1 - bx0) as f64,
                        ((by1 - by0) as f64) - 0.12,
                        (bz1 - bz0) as f64,
                    ],
                    origin_mc,
                    scale,
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    drop(flags);

    let mut zone_results: Vec<(&'static str, Vec<[f64; 6]>)> = Vec::with_capacity(active_zone_indices.len());
    for z_idx in active_zone_indices {
        let zk = MERGEABLE_ZONE_KINDS[z_idx];
        let top_y = if matches!(
            zk,
            "slime" | "honey" | "ice" | "packed_ice" | "blue_ice" | "soul_sand" | "magma" | "hay" | "bed"
        ) {
            0.18
        } else {
            0.0
        };
        if slice_sz > 0 {
            mask.par_chunks_mut(slice_sz)
                .enumerate()
                .for_each(|(gy, dst)| {
                    let src = &grid.blocks[gy * slice_sz..(gy + 1) * slice_sz];
                    for i in 0..slice_sz {
                        dst[i] = pal_meta[src[i] as usize].zone_kind_idx as usize == z_idx;
                    }
                });
        }
        let boxes: Vec<[f64; 6]> = greedy_mesh_3d(&mut mask, sx, sy, sz)
            .into_iter()
            .map(|[bx0, by0, bz0, bx1, by1, bz1]| {
                mc_box_to_source(
                    (c.min_x + bx0 as i32) as f64,
                    (c.min_y + by0 as i32) as f64,
                    (c.min_z + bz0 as i32) as f64,
                    [
                        0.0,
                        0.0,
                        0.0,
                        (bx1 - bx0) as f64,
                        (by1 - by0) as f64 + top_y,
                        (bz1 - bz0) as f64,
                    ],
                    origin_mc,
                    scale,
                )
            })
            .collect();
        zone_results.push((zk, boxes));
    }
    drop(mask);

    let water_zone_boxes = water_physics_extra.as_deref().unwrap_or(&water_pools);
    let mut ladder_boxes: Vec<[f64; 6]> = Vec::new();
    let mut physics_zones: Vec<PhysicsZone> = Vec::with_capacity(water_zone_boxes.len() + lava_results.len());

    for &sb in water_zone_boxes {
        physics_zones.push(PhysicsZone {
            kind: "water".to_string(),
            min: [sb[0], sb[1], sb[2]],
            max: [sb[3], sb[4], sb[5]],
        });
    }
    for sb in lava_results {
        hazard_boxes.push(HazardBox {
            kind: "lava".to_string(),
            box_coords: sb,
            damage: 40,
            damage_type: 8,
        });
        physics_zones.push(PhysicsZone {
            kind: "lava".to_string(),
            min: [sb[0], sb[1], sb[2]],
            max: [sb[3], sb[4], sb[5]],
        });
    }
    for (zk, boxes) in zone_results {
        let is_climb = zk == "vine";
        for sb in boxes {
            if is_climb {
                ladder_boxes.push(sb);
            }
            physics_zones.push(PhysicsZone {
                kind: zk.to_string(),
                min: [sb[0], sb[1], sb[2]],
                max: [sb[3], sb[4], sb[5]],
            });
        }
    }

    if ladder_sides_mask != 0 || vine_sides_mask != 0 {
        let emit_thin_wall_climb = |kind: &str,
                                    sides_mask: u8,
                                    match_fn: &dyn Fn(&PaletteColMeta, u8) -> bool,
                                    ladder_boxes: &mut Vec<[f64; 6]>,
                                    physics_zones: &mut Vec<PhysicsZone>| {
            let wall_local_box = |side: u8, span_u: f64, span_y: f64, thickness: f64| -> [f64; 6] {
                match side {
                    0 => [0.0, 0.0, 1.0 - thickness, span_u, span_y, 1.0],
                    1 => [0.0, 0.0, 0.0, span_u, span_y, thickness],
                    2 => [0.0, 0.0, 0.0, thickness, span_y, span_u],
                    _ => [1.0 - thickness, 0.0, 0.0, 1.0, span_y, span_u],
                }
            };
            let mut push_wall_rect = |side: u8, wx: f64, wy: f64, wz: f64, span_u: f64, span_y: f64| {
                let lb = mc_box_to_source(
                    wx,
                    wy,
                    wz,
                    wall_local_box(side, span_u, span_y, 0.125),
                    origin_mc,
                    scale,
                );
                ladder_boxes.push(lb);
                let pz = mc_box_to_source(
                    wx,
                    wy,
                    wz,
                    wall_local_box(side, span_u, span_y, 0.1875),
                    origin_mc,
                    scale,
                );
                physics_zones.push(PhysicsZone {
                    kind: kind.to_string(),
                    min: [pz[0], pz[1], pz[2]],
                    max: [pz[3], pz[4], pz[5]],
                });
            };

            if (sides_mask & 0b0011) != 0 && sx > 0 && sy > 0 {
                let mut z_masks = [
                    if (sides_mask & 1) != 0 { vec![false; sx * sy] } else { Vec::new() },
                    if (sides_mask & 2) != 0 { vec![false; sx * sy] } else { Vec::new() },
                ];
                for gz in 0..sz {
                    let mut hits = [false; 2];
                    for gy in 0..sy {
                        let src_row = (gy * sz + gz) * sx;
                        let dst_row = gy * sx;
                        for gx in 0..sx {
                            let m = &pal_meta[grid.blocks[src_row + gx] as usize];
                            for side in 0..2u8 {
                                if !z_masks[side as usize].is_empty() && match_fn(m, side) {
                                    z_masks[side as usize][dst_row + gx] = true;
                                    hits[side as usize] = true;
                                }
                            }
                        }
                    }
                    let wz = (c.min_z + gz as i32) as f64;
                    for side in 0..2u8 {
                        if !hits[side as usize] {
                            continue;
                        }
                        for [bx0, by0, _, bx1, by1, _] in
                            greedy_mesh_3d(&mut z_masks[side as usize], sx, sy, 1)
                        {
                            push_wall_rect(
                                side,
                                (c.min_x + bx0 as i32) as f64,
                                (c.min_y + by0 as i32) as f64,
                                wz,
                                (bx1 - bx0) as f64,
                                (by1 - by0) as f64,
                            );
                        }
                    }
                }
            }

            if (sides_mask & 0b1100) != 0 && sz > 0 && sy > 0 {
                let mut x_masks = [
                    if (sides_mask & 4) != 0 { vec![false; sz * sy] } else { Vec::new() },
                    if (sides_mask & 8) != 0 { vec![false; sz * sy] } else { Vec::new() },
                ];
                for gx in 0..sx {
                    let mut hits = [false; 2];
                    for gy in 0..sy {
                        let dst_row = gy * sz;
                        for gz in 0..sz {
                            let m = &pal_meta[grid.blocks[(gy * sz + gz) * sx + gx] as usize];
                            for idx in 0..2usize {
                                let side = (idx + 2) as u8;
                                if !x_masks[idx].is_empty() && match_fn(m, side) {
                                    x_masks[idx][dst_row + gz] = true;
                                    hits[idx] = true;
                                }
                            }
                        }
                    }
                    let wx = (c.min_x + gx as i32) as f64;
                    for idx in 0..2usize {
                        if !hits[idx] {
                            continue;
                        }
                        let side = (idx + 2) as u8;
                        for [bz0, by0, _, bz1, by1, _] in
                            greedy_mesh_3d(&mut x_masks[idx], sz, sy, 1)
                        {
                            push_wall_rect(
                                side,
                                wx,
                                (c.min_y + by0 as i32) as f64,
                                (c.min_z + bz0 as i32) as f64,
                                (bz1 - bz0) as f64,
                                (by1 - by0) as f64,
                            );
                        }
                    }
                }
            }
        };

        if ladder_sides_mask != 0 {
            emit_thin_wall_climb(
                "ladder",
                ladder_sides_mask,
                &|m, side| m.ladder_facing == side,
                &mut ladder_boxes,
                &mut physics_zones,
            );
        }
        if vine_sides_mask != 0 {
            emit_thin_wall_climb(
                "vine",
                vine_sides_mask,
                &|m, side| (m.vine_wall_mask & (1u8 << side)) != 0,
                &mut ladder_boxes,
                &mut physics_zones,
            );
        }
    }

    WorldCollisionResult {
        per_chunk,
        all_solid_boxes,
        ladder_boxes,
        water_pools,
        hazard_boxes,
        physics_zones,
        core_occluders,
    }
}

pub const MAX_CORE_OCCLUDERS: usize = 48;
pub const MIN_CORE_OCCLUDER_CELLS: usize = 4;

/// Filters out tiny 4x4x4-block core occluder slivers and keeps the largest-volume
/// occluders so VBSP/VVIS gets strong visibility blocking without portal bloat.
pub fn select_top_core_occluders(
    boxes: Vec<[usize; 6]>,
    max_count: usize,
    min_cells: usize,
) -> Vec<[usize; 6]> {
    let cell_vol = |b: &[usize; 6]| -> usize {
        b[3].saturating_sub(b[0])
            .saturating_mul(b[4].saturating_sub(b[1]))
            .saturating_mul(b[5].saturating_sub(b[2]))
    };
    let mut filtered: Vec<[usize; 6]> = boxes
        .into_iter()
        .filter(|b| cell_vol(b) >= min_cells)
        .collect();
    filtered.sort_by(|a, b| cell_vol(b).cmp(&cell_vol(a)).then_with(|| a.cmp(b)));
    filtered.truncate(max_count);
    filtered
}

/// 3D Greedy Meshing over a `bool` grid indexed as `(y * sz + z) * sx + x`.
/// Clears consumed bits in `mask` and returns `[x0, y0, z0, x1, y1, z1]`.
pub fn greedy_mesh_3d(mask: &mut [bool], sx: usize, sy: usize, sz: usize) -> Vec<[usize; 6]> {
    let mut out = Vec::new();
    if sx == 0 || sy == 0 || sz == 0 {
        return out;
    }
    let idx = |x: usize, y: usize, z: usize| -> usize { (y * sz + z) * sx + x };

    for y in 0..sy {
        for z in 0..sz {
            let mut x = 0;
            while x < sx {
                if !mask[idx(x, y, z)] {
                    x += 1;
                    continue;
                }
                // Expand along X
                let mut w = 1;
                while x + w < sx && mask[idx(x + w, y, z)] {
                    w += 1;
                }
                // Expand along Z
                let mut d = 1;
                'z_loop: while z + d < sz {
                    for dx in 0..w {
                        if !mask[idx(x + dx, y, z + d)] {
                            break 'z_loop;
                        }
                    }
                    d += 1;
                }
                // Expand along Y
                let mut h = 1;
                'y_loop: while y + h < sy {
                    for dz in 0..d {
                        for dx in 0..w {
                            if !mask[idx(x + dx, y + h, z + dz)] {
                                break 'y_loop;
                            }
                        }
                    }
                    h += 1;
                }
                // Consume volume
                for dy in 0..h {
                    for dz in 0..d {
                        for dx in 0..w {
                            mask[idx(x + dx, y + dy, z + dz)] = false;
                        }
                    }
                }
                out.push([x, y, z, x + w, y + h, z + d]);
                x += w;
            }
        }
    }
    out
}

#[cfg(test)]
pub fn build_hollow_shell_for_test(grid: &VoxelGrid) -> Vec<(i32, i32, i32)> {
    let c = grid.coords;
    let sx = c.size_x();
    let sy = c.size_y();
    let sz = c.size_z();
    let mut is_full = vec![false; grid.blocks.len()];
    for idx in 0..grid.blocks.len() {
        let cl = classify_solid_block(&grid.palette[grid.blocks[idx] as usize]);
        is_full[idx] = cl == SolidClass::FullOpaque || cl == SolidClass::FullLeaves;
    }
    let mut kept = Vec::new();
    for gy in 0..sy {
        for gz in 0..sz {
            for gx in 0..sx {
                let idx = (gy * sz + gz) * sx + gx;
                if !is_full[idx] {
                    continue;
                }
                let enclosed = gx > 0
                    && gx + 1 < sx
                    && gy > 0
                    && gy + 1 < sy
                    && gz > 0
                    && gz + 1 < sz
                    && is_full[idx - 1]
                    && is_full[idx + 1]
                    && is_full[idx - sx]
                    && is_full[idx + sx]
                    && is_full[idx - sx * sz]
                    && is_full[idx + sx * sz];
                if !enclosed {
                    kept.push((c.min_x + gx as i32, c.min_y + gy as i32, c.min_z + gz as i32));
                }
            }
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anvil::WorldCoords;
    use std::collections::BTreeMap;

    fn mk(name: &str, pairs: &[(&str, &str)]) -> BlockState {
        let mut props = BTreeMap::new();
        for (k, v) in pairs {
            props.insert((*k).to_string(), (*v).to_string());
        }
        BlockState::new(name.to_string(), props)
    }

    #[test]
    fn test_wall_arm_boxes_and_flowering_leaves() {
        let wall = mk(
            "minecraft:stone_brick_wall",
            &[("north", "low"), ("south", "none"), ("east", "tall"), ("west", "")],
        );
        let boxes = block_local_collision_boxes(&wall);
        assert_eq!(boxes.len(), 3, "center post + north + east arms");
        assert_eq!(boxes[0].0, [0.25, 0.0, 0.25, 0.75, 1.5, 0.75]);
        assert_eq!(boxes[1].0, [0.3125, 0.0, 0.0, 0.6875, 1.5, 0.5]);
        assert_eq!(boxes[2].0, [0.5, 0.0, 0.3125, 1.0, 1.5, 0.6875]);

        let fl_leaves = mk("minecraft:flowering_azalea_leaves", &[]);
        assert!(is_leaves_block(&fl_leaves));
        assert!(!is_passable_block(&fl_leaves));
        assert_eq!(classify_solid_block(&fl_leaves), SolidClass::FullLeaves);
    }

    #[test]
    fn test_wall_stair_and_leaves_merging() {
        let air = mk("minecraft:air", &[]);
        let wall = mk(
            "minecraft:cobblestone_wall",
            &[("up", "false"), ("east", "low"), ("west", "low"), ("north", "none"), ("south", "none")],
        );
        let stair = mk("minecraft:stone_brick_stairs", &[("half", "bottom"), ("facing", "north")]);
        let leaves = mk("minecraft:oak_leaves", &[]);

        // 4x3x3 grid:
        // y=0: 4 straight walls along x at z=0
        // y=1: 4 north stairs along x at z=1
        // y=0..3 at x=0..3, z=2: wait, let's test 3x3x3 leaves in separate grid
        let mut blocks = vec![0u16; 4 * 2 * 2];
        for x in 0..4usize {
            blocks[x] = 1; // y=0, z=0: wall
            blocks[(2 + 1) * 4 + x] = 2; // y=1, z=1: stair
        }
        let grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 0, min_z: 0, max_x: 3, max_y: 1, max_z: 1 },
            palette: vec![air.clone(), wall, stair],
            air_id: 0,
            blocks,
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 16],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let col = build_world_collision(&grid, [0.0, 0.0, 0.0], 1.0);
        let ch = col.per_chunk.get(&(0, 0)).unwrap();
        let wall_boxes: Vec<_> = ch.iter().filter(|b| b.block_class == "wall").collect();
        let stair_boxes: Vec<_> = ch.iter().filter(|b| b.block_class == "stairs").collect();
        assert_eq!(wall_boxes.len(), 1, "4 straight walls with up=false merge into 1 box");
        assert_eq!(stair_boxes.len(), 2, "4 contiguous north stairs merge into 1 base + 1 step");

        // 3x3x3 solid leaves should greedy-mesh into 1 box (not hollowed into 6)
        let leaf_grid = VoxelGrid {
            coords: WorldCoords { min_x: 0, min_y: 0, min_z: 0, max_x: 2, max_y: 2, max_z: 2 },
            palette: vec![air, leaves],
            air_id: 0,
            blocks: vec![1u16; 27],
            biome_palette: vec!["minecraft:plains".to_string()],
            biomes: vec![0; 27],
            signs: HashMap::new(),
            player_spawn_mc: None,
        };
        let leaf_col = build_world_collision(&leaf_grid, [0.0, 0.0, 0.0], 1.0);
        let leaf_ch = leaf_col.per_chunk.get(&(0, 0)).unwrap();
        assert_eq!(leaf_ch.len(), 1, "3x3x3 solid leaves must merge into 1 box without hollowing");
        assert_eq!(leaf_ch[0].block_class, "leaves");
    }
}
