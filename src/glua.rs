use crate::collision::PhysicsZone;
use crate::mesher::ParkourMarker;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as FmtWrite;
use std::path::Path;

#[cfg(test)]
pub const JUMP_POWER: f64 = 329.36;
#[cfg(test)]
pub const CROUCHED_WALK_SPEED: f64 = 1.30 / 4.317;
#[cfg(test)]
pub const STEP_SIZE: f64 = 0.60 * 39.37;

/// Escapes a string for safe embedding inside a double-quoted Lua string literal,
/// preventing Lua injection / syntax errors from arbitrary map names.
fn lua_escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            _ => out.push(c),
        }
    }
    out
}

/// Filesystem-safe stem for per-map Lua / .ain files. Blocks path separators,
/// Windows-illegal characters and leading-dot traversal prefixes ("..").
/// Exotic map names that change during sanitization will not auto-load their
/// physics config (GMod looks up `<lowercase map name>.lua`), which is the safe
/// failure mode; typical names like `gm_mc_export` pass through unchanged.
pub(crate) fn sanitize_map_stem(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ' ' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return "mcgm_map".to_string();
    }
    let is_reserved_dos = matches!(
        trimmed.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if is_reserved_dos {
        format!("mc_{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

pub fn generate_mcgm_physics_autorun() -> String {
    include_str!("../lua/autorun/mcgm_physics.lua").to_string()
}

pub fn generate_mcgm_smooth_fall_autorun() -> String {
    include_str!("../lua/autorun/mcgm_smooth_fall.lua").to_string()
}

pub fn generate_mcgm_parkour_autorun() -> String {
    include_str!("../lua/autorun/mcgm_parkour.lua").to_string()
}

/// Generates `lua/mcgm_parkour/<map>.lua` containing checkpoint and finish marker zones
/// in Source coordinates (independent of Minecraft movement physics settings).
pub fn generate_map_parkour_lua(
    map_name: &str,
    scale_factor: f64,
    markers: &[ParkourMarker],
) -> String {
    let map_key = lua_escape_str(&map_name.to_lowercase());
    let mut out = String::with_capacity(markers.len() * 192 + 320);
    out.push_str("if not SERVER then return end\n");
    out.push_str("MCGM_PARKOUR = MCGM_PARKOUR or {}\n");
    out.push_str("MCGM_PARKOUR.Maps = MCGM_PARKOUR.Maps or {}\n");
    let _ = writeln!(out, "local mapData = {{\n  scale = {:.2},\n  markers = {{", scale_factor);

    for m in markers {
        let id_esc = lua_escape_str(&m.id);
        let name_esc = lua_escape_str(&m.name);
        let _ = writeln!(
            out,
            "    {{ id = \"{}\", kind = \"{}\", name = \"{}\", is_wall = {}, yaw = {:.2}, mc = {{ {}, {}, {} }}, min = {{ {:.2}, {:.2}, {:.2} }}, max = {{ {:.2}, {:.2}, {:.2} }}, anchor = {{ {:.2}, {:.2}, {:.2} }}, fwd = {{ {:.4}, {:.4} }} }},",
            id_esc,
            m.kind.as_str(),
            name_esc,
            m.is_wall,
            m.yaw,
            m.mc_pos[0],
            m.mc_pos[1],
            m.mc_pos[2],
            m.zone_min[0],
            m.zone_min[1],
            m.zone_min[2],
            m.zone_max[0],
            m.zone_max[1],
            m.zone_max[2],
            m.anchor_pos[0],
            m.anchor_pos[1],
            m.anchor_pos[2],
            m.forward_dir[0],
            m.forward_dir[1],
        );
    }

    out.push_str("  }\n}\n");
    let _ = writeln!(out, "MCGM_PARKOUR.Maps[\"{}\"] = mapData", map_key);
    let _ = writeln!(
        out,
        "if _G.MCGM_RegisterParkourMap then _G.MCGM_RegisterParkourMap(\"{}\", mapData) end",
        map_key
    );
    out
}

pub fn install_parkour_marker_files(
    map_name: &str,
    dest_lua_dirs: &[&Path],
    scale_factor: f64,
    markers: &[ParkourMarker],
) -> Result<(), String> {
    let autorun_src = generate_mcgm_parkour_autorun();
    let map_lua_src = generate_map_parkour_lua(map_name, scale_factor, markers);
    let map_stem = sanitize_map_stem(map_name);
    let map_fname = format!("{}.lua", map_stem);

    for &lua_dir in dest_lua_dirs {
        let autorun_dir = lua_dir.join("autorun");
        let parkour_dir = lua_dir.join("mcgm_parkour");
        std::fs::create_dir_all(&autorun_dir)
            .map_err(|e| format!("Failed creating {}: {}", autorun_dir.display(), e))?;
        std::fs::create_dir_all(&parkour_dir)
            .map_err(|e| format!("Failed creating {}: {}", parkour_dir.display(), e))?;

        std::fs::write(autorun_dir.join("mcgm_parkour.lua"), &autorun_src)
            .map_err(|e| e.to_string())?;
        std::fs::write(parkour_dir.join(&map_fname), &map_lua_src).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn generate_map_physics_lua(
    map_name: &str,
    scale_factor: f64,
    default_mc_physics: bool,
    zones: &[PhysicsZone],
) -> String {
    // Escaped so arbitrary map names can never break out of the Lua string literal.
    let map_key = lua_escape_str(&map_name.to_lowercase());
    let cell_size = 512.0;

    // Spatial bucketing in XY grid cells, grouped by normalized zone kind (Format A).
    let mut buckets: BTreeMap<i32, BTreeMap<i32, BTreeMap<&str, Vec<&PhysicsZone>>>> =
        BTreeMap::new();
    let mut has_kinds: BTreeSet<&str> = BTreeSet::new();
    for z in zones {
        let eff_kind = if z.kind == "ladder" || z.kind == "vine" {
            "climb"
        } else {
            z.kind.as_str()
        };
        has_kinds.insert(eff_kind);
        let cx0 = (z.min[0].min(z.max[0]) / cell_size).floor() as i32;
        let cx1 = (z.min[0].max(z.max[0]) / cell_size).floor() as i32;
        let cy0 = (z.min[1].min(z.max[1]) / cell_size).floor() as i32;
        let cy1 = (z.min[1].max(z.max[1]) / cell_size).floor() as i32;
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                buckets
                    .entry(cx)
                    .or_default()
                    .entry(cy)
                    .or_default()
                    .entry(eff_kind)
                    .or_default()
                    .push(z);
            }
        }
    }

    let mut out = String::with_capacity(zones.len() * 48 + 384);
    out.push_str("if SERVER then AddCSLuaFile() end\n");
    out.push_str("MCGM_PHYSICS = MCGM_PHYSICS or {}\n");
    out.push_str("MCGM_PHYSICS.Maps = MCGM_PHYSICS.Maps or {}\n");
    let _ = writeln!(
        out,
        "local mapData = {{\n  scale = {:.2},\n  enabled_by_default = {},\n  default_mc_physics = {},\n  chunk_size = 512,",
        scale_factor, default_mc_physics, default_mc_physics
    );

    out.push_str("  has_kinds = { ");
    for k in &has_kinds {
        let _ = write!(out, "[\"{}\"] = true, ", k);
    }
    out.push_str("},\n  buckets = {\n");

    for (cx, y_map) in &buckets {
        let _ = writeln!(out, "    [{}] = {{", cx);
        for (cy, kind_map) in y_map {
            let _ = writeln!(out, "      [{}] = {{", cy);
            for (kind, list) in kind_map {
                let _ = write!(out, "        [\"{}\"] = {{ ", kind);
                for z in list.iter() {
                    let _ = write!(
                        out,
                        "{:.1}, {:.1}, {:.1}, {:.1}, {:.1}, {:.1}, ",
                        z.min[0].min(z.max[0]),
                        z.min[1].min(z.max[1]),
                        z.min[2].min(z.max[2]),
                        z.min[0].max(z.max[0]),
                        z.min[1].max(z.max[1]),
                        z.min[2].max(z.max[2]),
                    );
                }
                out.push_str("},\n");
            }
            out.push_str("      },\n");
        }
        out.push_str("    },\n");
    }

    out.push_str("  }\n}\n");
    let _ = writeln!(out, "MCGM_PHYSICS.Maps[\"{}\"] = mapData", map_key);
    let _ = writeln!(
        out,
        "if _G.MCGM_RegisterMap then _G.MCGM_RegisterMap(\"{}\", mapData) end",
        map_key
    );
    out
}

pub fn install_mc_physics_files(
    map_name: &str,
    dest_lua_dirs: &[&Path],
    scale_factor: f64,
    default_mc_physics: bool,
    zones: &[PhysicsZone],
) -> Result<(), String> {
    let autorun_src = generate_mcgm_physics_autorun();
    let smooth_src = generate_mcgm_smooth_fall_autorun();
    let map_lua_src = generate_map_physics_lua(map_name, scale_factor, default_mc_physics, zones);

    for &lua_dir in dest_lua_dirs {
        let autorun_dir = lua_dir.join("autorun");
        let maps_cfg_dir = lua_dir.join("mcgm_physics");
        let maps_alt_dir = lua_dir.join("mcgm_maps");
        std::fs::create_dir_all(&autorun_dir)
            .map_err(|e| format!("Failed creating {}: {}", autorun_dir.display(), e))?;
        std::fs::create_dir_all(&maps_cfg_dir)
            .map_err(|e| format!("Failed creating {}: {}", maps_cfg_dir.display(), e))?;
        std::fs::create_dir_all(&maps_alt_dir)
            .map_err(|e| format!("Failed creating {}: {}", maps_alt_dir.display(), e))?;

        std::fs::write(autorun_dir.join("mcgm_physics.lua"), &autorun_src)
            .map_err(|e| e.to_string())?;
        std::fs::write(autorun_dir.join("mcgm_smooth_fall.lua"), &smooth_src)
            .map_err(|e| e.to_string())?;
        let map_stem = sanitize_map_stem(map_name);
        if map_stem != map_name.to_lowercase() {
            eprintln!(
                "[Glua] Warning: map name '{}' contains unsafe characters; using file stem '{}' \
                 (exotic names will not auto-load their physics config in GMod)",
                map_name, map_stem
            );
        }
        let map_fname = format!("{}.lua", map_stem);
        std::fs::write(maps_cfg_dir.join(&map_fname), &map_lua_src)
            .map_err(|e| e.to_string())?;
        std::fs::write(maps_alt_dir.join(&map_fname), &map_lua_src)
            .map_err(|e| e.to_string())?;

        // Remove any stale .ain node graph from earlier compiles of this map name
        if let Some(root_dir) = lua_dir.parent() {
            let graphs_dir = root_dir.join("maps").join("graphs");
            let stale_ain = graphs_dir.join(format!("{}.ain", map_stem));
            if stale_ain.is_file() {
                let _ = std::fs::remove_file(&stale_ain);
            }
        }
    }
    Ok(())
}
