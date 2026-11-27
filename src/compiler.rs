use crate::atlas::RenderBucket;
use crate::smd_writer::CompiledChunkArtifact;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const DEFAULT_MAX_CONVEX_PIECES: usize = 2048;

#[derive(Debug, Clone)]
pub struct ToolPaths {
    pub studiomdl: PathBuf,
    pub vtfcmd: PathBuf,
    pub vbsp: PathBuf,
    pub vvis: PathBuf,
    pub vrad: PathBuf,
    pub gmad: Option<PathBuf>,
    pub game_dir: PathBuf,
    pub vanilla_jar: Option<PathBuf>,
    pub hammer: Option<PathBuf>,
    pub world_path: Option<PathBuf>,
    pub scale_factor: f64,
    pub max_convex_pieces: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VtfMode {
    Skybox,
    AtlasDxt5,
    SignBgra8888,
    DoorDxt5NoMip,
}

#[cfg(windows)]
mod win_reg {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    pub const HKEY_CURRENT_USER: isize = -0x7fff_ffff; // 0x80000001u32 as i32 as isize
    pub const HKEY_LOCAL_MACHINE: isize = -0x7fff_fffe; // 0x80000002u32 as i32 as isize
    const KEY_READ: u32 = 0x20019;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            hkey: isize,
            lp_sub_key: *const u16,
            ul_options: u32,
            sam_desired: u32,
            phk_result: *mut isize,
        ) -> i32;
        fn RegQueryValueExW(
            hkey: isize,
            lp_value_name: *const u16,
            lp_reserved: *mut u32,
            lp_type: *mut u32,
            lp_data: *mut u8,
            lpcb_data: *mut u32,
        ) -> i32;
        fn RegCloseKey(hkey: isize) -> i32;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub fn read_reg_string(root: isize, subkey: &str, value_name: &str) -> Option<String> {
        let sub_w = to_wide(subkey);
        let val_w = to_wide(value_name);
        let mut hkey: isize = 0;
        let rc = unsafe { RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_READ, &mut hkey) };
        if rc != 0 || hkey == 0 {
            return None;
        }
        let mut reg_type: u32 = 0;
        let mut byte_len: u32 = 0;
        let rc_size = unsafe {
            RegQueryValueExW(
                hkey,
                val_w.as_ptr(),
                std::ptr::null_mut(),
                &mut reg_type,
                std::ptr::null_mut(),
                &mut byte_len,
            )
        };
        if rc_size != 0 || byte_len == 0 || (reg_type != REG_SZ && reg_type != REG_EXPAND_SZ) {
            unsafe { RegCloseKey(hkey) };
            return None;
        }
        let mut buf = vec![0u8; byte_len as usize + 2];
        let mut actual_len = buf.len() as u32;
        let rc_read = unsafe {
            RegQueryValueExW(
                hkey,
                val_w.as_ptr(),
                std::ptr::null_mut(),
                &mut reg_type,
                buf.as_mut_ptr(),
                &mut actual_len,
            )
        };
        unsafe { RegCloseKey(hkey) };
        if rc_read != 0 {
            return None;
        }
        let u16_len = (actual_len as usize) / 2;
        let mut wchars = Vec::with_capacity(u16_len);
        for chunk in buf[..u16_len * 2].as_chunks::<2>().0 {
            let code = u16::from_le_bytes([chunk[0], chunk[1]]);
            if code == 0 {
                break;
            }
            wchars.push(code);
        }
        let s = String::from_utf16_lossy(&wchars);
        let trimmed = s.trim().trim_matches('"').to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }
}

pub fn read_registry_string_hkcu_or_hklm(subkey: &str, value_name: &str) -> Option<String> {
    win_reg::read_reg_string(win_reg::HKEY_CURRENT_USER, subkey, value_name)
        .or_else(|| win_reg::read_reg_string(win_reg::HKEY_LOCAL_MACHINE, subkey, value_name))
}

pub fn get_logical_drives() -> Vec<PathBuf> {
    static DRIVES_CACHE: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DRIVES_CACHE
        .get_or_init(|| {
            let mut drives = Vec::new();
            let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
            if mask != 0 {
                for idx in 0..26u32 {
                    if (mask & (1 << idx)) != 0 {
                        let letter = (b'A' + idx as u8) as char;
                        let p = PathBuf::from(format!("{}:\\", letter));
                        if p.exists() {
                            drives.push(p);
                        }
                    }
                }
            }
            if drives.is_empty() {
                for letter in ['C', 'D', 'E', 'F', 'G', 'H', 'I'] {
                    let p = PathBuf::from(format!("{}:\\", letter));
                    if p.exists() {
                        drives.push(p);
                    }
                }
            }
            drives
        })
        .clone()
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        for name in names {
            let cand = dir.join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

pub fn get_steam_libraries() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(windows)]
    {
        for (root, sub, val) in [
            (win_reg::HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
            (win_reg::HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
            (win_reg::HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        ] {
            if let Some(p) = win_reg::read_reg_string(root, sub, val) {
                candidates.push(PathBuf::from(p));
            }
        }
    }

    for drive in get_logical_drives() {
        candidates.push(drive.join("Program Files (x86)").join("Steam"));
        candidates.push(drive.join("Program Files").join("Steam"));
        candidates.push(drive.join("Steam"));
        candidates.push(drive.join("SteamLibrary"));
        candidates.push(drive.join("Games").join("Steam"));
    }

    let mut libraries = Vec::new();
    let mut seen = HashSet::new();

    let mut add_lib = |p: PathBuf| {
        if !p.is_dir() {
            return;
        }
        let canon = std::fs::canonicalize(&p).unwrap_or(p);
        let key = canon.to_string_lossy().to_ascii_lowercase();
        if seen.insert(key) {
            libraries.push(canon);
        }
    };

    let mut initial = Vec::new();
    for cand in candidates {
        if cand.is_dir() {
            let canon = std::fs::canonicalize(&cand).unwrap_or(cand);
            initial.push(canon);
        }
    }

    for lib in initial {
        add_lib(lib.clone());
        for vdf_rel in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
            let vdf = lib.join(vdf_rel);
            if let Ok(content) = std::fs::read_to_string(&vdf) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("\"path\"") {
                        let rest = rest.trim();
                        if let Some(quoted) = rest.strip_prefix('"') {
                            if let Some(end_idx) = quoted.find('"') {
                                let raw_path = &quoted[..end_idx];
                                let unescaped = raw_path.replace("\\\\", "\\");
                                add_lib(PathBuf::from(unescaped));
                            }
                        }
                    }
                }
            }
        }
    }

    libraries
}

pub fn detect_garrysmod() -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let mut gmod_roots: Vec<PathBuf> = Vec::new();
    let mut seen_roots = HashSet::new();

    let mut push_root = |p: PathBuf| {
        if p.is_dir() {
            let canon = std::fs::canonicalize(&p).unwrap_or(p);
            let k = canon.to_string_lossy().to_ascii_lowercase();
            if seen_roots.insert(k) {
                gmod_roots.push(canon);
            }
        }
    };

    for steam_lib in get_steam_libraries() {
        push_root(steam_lib.join("steamapps").join("common").join("GarrysMod"));
    }

    for drive in get_logical_drives() {
        for cand in [
            drive.join("Steam").join("steamapps").join("common").join("GarrysMod"),
            drive.join("Games").join("Steam").join("steamapps").join("common").join("GarrysMod"),
            drive.join("Games").join("GarrysMod"),
            drive.join("GarrysMod"),
            drive.join("Program Files (x86)").join("Steam").join("steamapps").join("common").join("GarrysMod"),
            drive.join("Program Files").join("Steam").join("steamapps").join("common").join("GarrysMod"),
        ] {
            push_root(cand);
        }
    }

    for root in &gmod_roots {
        let game_dir = root.join("garrysmod");
        if !game_dir.is_dir() {
            continue;
        }
        let studiomdl = [
            root.join("bin").join("studiomdl.exe"),
            root.join("bin").join("win64").join("studiomdl.exe"),
        ]
        .into_iter()
        .find(|p| p.is_file());

        let hammer = [
            root.join("bin").join("hammer.exe"),
            root.join("bin").join("win64").join("hammer.exe"),
        ]
        .into_iter()
        .find(|p| p.is_file());

        if studiomdl.is_some() {
            return (studiomdl, Some(game_dir), hammer);
        }
    }

    // Fallback: partial GMod root with game_dir even if studiomdl wasn't found, or PATH
    let partial_game = gmod_roots
        .iter()
        .map(|r| r.join("garrysmod"))
        .find(|g| g.is_dir());
    let path_studiomdl = find_in_path(&["studiomdl.exe", "studiomdl"]);
    let path_hammer = find_in_path(&["hammer.exe", "hammer"]);

    (path_studiomdl, partial_game, path_hammer)
}

fn scan_dir_for_exe(dir: &Path, target_lower: &str, max_depth: usize) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((cur, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&cur) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Some(fname) = p.file_name().and_then(|s| s.to_str()) {
                    if fname.eq_ignore_ascii_case(target_lower) {
                        return Some(p);
                    }
                }
            } else if p.is_dir() && depth < max_depth {
                stack.push((p, depth + 1));
            }
        }
    }
    None
}

pub fn detect_vtfcmd() -> Option<PathBuf> {
    // 1. Local workspace / exe directory
    let mut local_roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        local_roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            local_roots.push(dir.to_path_buf());
        }
    }
    for r in local_roots {
        for rel in [
            "tools/VTFLib/bin/x86/VTFCmd.exe",
            "tools/VTFLib/bin/x64/VTFCmd.exe",
            "tools/VTFLib/VTFCmd.exe",
            "tools/VTFCmd.exe",
            "bin/VTFCmd.exe",
            "VTFCmd.exe",
        ] {
            let cand = r.join(rel);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }

    // 2. PATH
    if let Some(p) = find_in_path(&["VTFCmd.exe", "vtfcmd.exe", "VTFCmd", "vtfcmd"]) {
        return Some(p);
    }

    // 3. Common directories across all logical drives
    for drive in get_logical_drives() {
        for rel in [
            "Tools/VTFLib/bin/x86/VTFCmd.exe",
            "Tools/VTFLib/bin/x64/VTFCmd.exe",
            "Tools/VTFLib/VTFCmd.exe",
            "VTFLib/bin/x86/VTFCmd.exe",
            "VTFLib/bin/x64/VTFCmd.exe",
            "VTFLib/VTFCmd.exe",
            "Program Files/VTFLib/bin/x86/VTFCmd.exe",
            "Program Files (x86)/VTFLib/bin/x86/VTFCmd.exe",
            "Program Files/Nem's Tools/VTFLib/bin/x86/VTFCmd.exe",
            "Program Files (x86)/Nem's Tools/VTFLib/bin/x86/VTFCmd.exe",
        ] {
            let cand = drive.join(rel);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }

    // 4. User Downloads / Desktop
    if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        for sub in ["Downloads", "Desktop"] {
            if let Some(found) = scan_dir_for_exe(&profile.join(sub), "vtfcmd.exe", 4) {
                return Some(found);
            }
        }
    }

    None
}

pub fn is_valid_vanilla_jar(path: &Path) -> bool {
    crate::assets::is_valid_vanilla_jar(path)
}

fn collect_jars_recursive(dir: &Path, max_depth: usize, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((cur, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&cur) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("jar")).unwrap_or(false) {
                    out.push(p);
                }
            } else if p.is_dir() && depth < max_depth {
                stack.push((p, depth + 1));
            }
        }
    }
}

pub fn detect_minecraft_vanilla_jar() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        let mc_dir = appdata.join(".minecraft");
        let versions_dir = mc_dir.join("versions");
        if let Ok(rd) = std::fs::read_dir(&versions_dir) {
            for ent in rd.flatten() {
                let d = ent.path();
                if d.is_dir() {
                    if let Some(name) = d.file_name().and_then(|s| s.to_str()) {
                        let jar = d.join(format!("{}.jar", name));
                        if jar.is_file() {
                            candidates.push(jar);
                        }
                    }
                }
            }
        }
        collect_jars_recursive(&mc_dir.join("libraries").join("com").join("mojang").join("minecraft"), 4, &mut candidates);
        collect_jars_recursive(
            &appdata.join("PrismLauncher").join("libraries").join("com").join("mojang").join("minecraft"),
            4,
            &mut candidates,
        );
    }

    if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        let cf_versions = profile.join("curseforge").join("minecraft").join("Install").join("versions");
        if let Ok(rd) = std::fs::read_dir(&cf_versions) {
            for ent in rd.flatten() {
                let d = ent.path();
                if d.is_dir() {
                    if let Some(name) = d.file_name().and_then(|s| s.to_str()) {
                        let jar = d.join(format!("{}.jar", name));
                        if jar.is_file() {
                            candidates.push(jar);
                        }
                    }
                }
            }
        }
    }

    for drive in get_logical_drives() {
        for launcher in [
            drive.join("Games").join("PrismLauncher"),
            drive.join("PrismLauncher"),
            drive.join("MultiMC"),
            drive.join("Games").join("MultiMC"),
        ] {
            collect_jars_recursive(&launcher.join("libraries").join("com").join("mojang").join("minecraft"), 4, &mut candidates);
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        collect_jars_recursive(&cwd.join("assets"), 1, &mut candidates);
        collect_jars_recursive(&cwd, 0, &mut candidates);
    }

    // Deduplicate and rank
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for c in candidates {
        let canon = std::fs::canonicalize(&c).unwrap_or(c);
        let key = canon.to_string_lossy().to_ascii_lowercase();
        if seen.insert(key) {
            unique.push(canon);
        }
    }

    unique.sort_by_key(|p| {
        let full = p.to_string_lossy().to_ascii_lowercase();
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let loader_penalty: u8 = if ["fabric", "forge", "neoforge", "quilt"]
            .iter()
            .any(|k| full.contains(k))
        {
            1
        } else {
            0
        };
        let ver_rank = crate::assets::parse_mc_version_tuple(&full)
            .map(|(maj, min, pat)| (2u8, (maj, min, pat)))
            .unwrap_or_else(|| {
                if name.contains("client") {
                    (1u8, (0, 0, 0))
                } else {
                    (0u8, (0, 0, 0))
                }
            });
        (
            loader_penalty,
            std::cmp::Reverse(ver_rank),
            std::cmp::Reverse(name),
        )
    });

    unique.into_iter().find(|p| is_valid_vanilla_jar(p))
}

pub fn detect_minecraft_saves() -> Vec<PathBuf> {
    let mut saves = Vec::new();
    let mut seen = HashSet::new();

    let mut push_dir = |p: PathBuf| {
        if p.is_dir() {
            let clean = crate::instances::strip_windows_verbatim_prefix(&p);
            let canon = std::fs::canonicalize(&clean).unwrap_or_else(|_| clean.clone());
            let key = canon.to_string_lossy().to_ascii_lowercase();
            if seen.insert(key) {
                saves.push(clean);
            }
        }
    };

    for inst in crate::instances::discover_all_instances(None, &[]) {
        push_dir(inst.saves_dir);
    }

    if let Ok(cwd) = std::env::current_dir() {
        push_dir(cwd.join("world"));
        push_dir(cwd.join("saves").join("world"));
        push_dir(cwd.join("input").join("world"));
    }

    saves
}

fn detect_tools_pp_dir(studiomdl: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(sm) = studiomdl {
        if let Some(p) = sm.parent().filter(|p| !p.as_os_str().is_empty()) {
            candidates.push(p.to_path_buf());
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("tools++"));
            candidates.push(dir.join("Tools++"));
            candidates.push(dir.join("Tools++").join("tools++_windows"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("tools++"));
        candidates.push(cwd.join("Tools++"));
        candidates.push(cwd.join("Tools++").join("tools++_windows"));
    }
    for base in [
        PathBuf::from(r"C:\Tools\Tools++\tools++_windows"),
        PathBuf::from(r"C:\Tools\Tools++"),
        PathBuf::from(r"C:\Tools\tools++"),
    ] {
        candidates.push(base);
    }
    for c in candidates {
        for sub in [c.clone(), c.join("bin"), c.join("x64")] {
            if sub.join("vbsp++.exe").is_file() || sub.join("vrad++.exe").is_file() {
                return Some(sub);
            }
        }
    }
    None
}

pub fn load_tool_paths(config_path: Option<&Path>) -> ToolPaths {
    let mut cfg_studiomdl: Option<PathBuf> = None;
    let mut cfg_vbsp: Option<PathBuf> = None;
    let mut cfg_vvis: Option<PathBuf> = None;
    let mut cfg_vrad: Option<PathBuf> = None;
    let mut cfg_vtfcmd: Option<PathBuf> = None;
    let mut cfg_game_dir: Option<PathBuf> = None;
    let mut cfg_hammer: Option<PathBuf> = None;
    let mut cfg_vanilla_jar: Option<PathBuf> = None;
    let mut cfg_world_path: Option<PathBuf> = None;
    let mut scale_factor: f64 = 39.37;
    let mut max_convex_pieces: usize = DEFAULT_MAX_CONVEX_PIECES;

    if let Some(cp) = config_path {
        if let Ok(text) = std::fs::read_to_string(cp) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(s) = val.get("studiomdl_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && p.is_file() {
                        cfg_studiomdl = Some(p);
                    }
                }
                if let Some(s) = val.get("vbsp_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && p.is_file() {
                        cfg_vbsp = Some(p);
                    }
                }
                if let Some(s) = val.get("vvis_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && p.is_file() {
                        cfg_vvis = Some(p);
                    }
                }
                if let Some(s) = val.get("vrad_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && p.is_file() {
                        cfg_vrad = Some(p);
                    }
                }
                if let Some(s) = val.get("vtfcmd_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && p.is_file() {
                        cfg_vtfcmd = Some(p);
                    }
                }
                if let Some(s) = val.get("game_dir").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && p.is_dir() {
                        cfg_game_dir = Some(p);
                    }
                }
                if let Some(s) = val.get("hammer_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && p.is_file() {
                        cfg_hammer = Some(p);
                    }
                }
                if let Some(s) = val.get("vanilla_jar_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && is_valid_vanilla_jar(&p) {
                        cfg_vanilla_jar = Some(p);
                    }
                }
                if let Some(s) = val.get("world_path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(s);
                    if !s.is_empty() && p.is_dir() {
                        cfg_world_path = Some(p);
                    }
                }
                if let Some(sf) = val.get("scale_factor").and_then(|v| v.as_f64()) {
                    if sf.is_finite() && sf > 0.0 {
                        scale_factor = sf;
                    }
                }
                if let Some(mc) = val.get("max_convex_pieces").and_then(|v| v.as_u64()) {
                    if mc > 0 {
                        max_convex_pieces = mc as usize;
                    }
                }
            }
        }
    }

    if cfg_studiomdl.is_none() || cfg_game_dir.is_none() || cfg_hammer.is_none() {
        let (det_studio, det_game, det_hammer) = detect_garrysmod();
        if cfg_studiomdl.is_none() {
            cfg_studiomdl = det_studio;
        }
        if cfg_game_dir.is_none() {
            cfg_game_dir = det_game;
        }
        if cfg_hammer.is_none() {
            cfg_hammer = det_hammer;
        }
    }

    if cfg_vtfcmd.is_none() {
        cfg_vtfcmd = detect_vtfcmd();
    }
    if cfg_vanilla_jar.is_none() {
        cfg_vanilla_jar = detect_minecraft_vanilla_jar();
    }
    if cfg_world_path.is_none() {
        cfg_world_path = detect_minecraft_saves().into_iter().next();
    }

    let pp_dir = detect_tools_pp_dir(cfg_studiomdl.as_deref());
    let studiomdl = cfg_studiomdl.unwrap_or_default();
    let vtfcmd = cfg_vtfcmd.unwrap_or_default();
    let game_dir = cfg_game_dir.unwrap_or_default();

    let resolve_bin_tool = |exe_name: &str| -> PathBuf {
        if let Some(bin_dir) = studiomdl.parent().filter(|p| !p.as_os_str().is_empty()) {
            let direct = bin_dir.join(exe_name);
            if direct.is_file() {
                return direct;
            }
            // If studiomdl is in bin/win64/, vbsp/vvis/vrad may live in bin/
            if let Some(parent_bin) = bin_dir.parent() {
                let up = parent_bin.join(exe_name);
                if up.is_file() {
                    return up;
                }
            }
            return direct;
        }
        if !game_dir.as_os_str().is_empty() {
            if let Some(root) = game_dir.parent() {
                let bin_cand = root.join("bin").join(exe_name);
                if bin_cand.is_file() {
                    return bin_cand;
                }
            }
        }
        PathBuf::new()
    };

    let vbsp = cfg_vbsp
        .or_else(|| {
            pp_dir
                .as_ref()
                .map(|d| d.join("vbsp++.exe"))
                .filter(|p| p.is_file())
        })
        .unwrap_or_else(|| resolve_bin_tool("vbsp.exe"));
    let vvis = cfg_vvis
        .or_else(|| {
            pp_dir
                .as_ref()
                .map(|d| d.join("vvis++.exe"))
                .filter(|p| p.is_file())
        })
        .unwrap_or_else(|| resolve_bin_tool("vvis.exe"));
    let vrad = cfg_vrad
        .or_else(|| {
            pp_dir
                .as_ref()
                .map(|d| d.join("vrad++.exe"))
                .filter(|p| p.is_file())
        })
        .unwrap_or_else(|| resolve_bin_tool("vrad.exe"));

    let gmad = studiomdl
        .parent()
        .map(|d| d.join("gmad.exe"))
        .filter(|p| p.is_file())
        .or_else(|| {
            if !game_dir.as_os_str().is_empty() {
                game_dir.parent().and_then(|root| {
                    let c1 = root.join("bin").join("gmad.exe");
                    if c1.is_file() {
                        Some(c1)
                    } else {
                        let c2 = root.join("bin").join("win64").join("gmad.exe");
                        c2.is_file().then_some(c2)
                    }
                })
            } else {
                None
            }
        });

    ToolPaths {
        studiomdl,
        vtfcmd,
        vbsp,
        vvis,
        vrad,
        gmad,
        game_dir,
        vanilla_jar: cfg_vanilla_jar,
        hammer: cfg_hammer,
        world_path: cfg_world_path,
        scale_factor,
        max_convex_pieces,
    }
}

pub fn detect_tools_and_save(config_path: &Path) -> Result<ToolPaths, String> {
    let tools = load_tool_paths(config_path.is_file().then_some(config_path));

    let mut root_obj = if config_path.is_file() {
        std::fs::read_to_string(config_path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    } else {
        serde_json::Map::new()
    };

    for legacy_key in ["blender_path", "mineways_path", "jdk_path"] {
        root_obj.remove(legacy_key);
    }

    let mut set_path = |key: &str, p: &Path| {
        if !p.as_os_str().is_empty() && p.exists() {
            root_obj.insert(
                key.to_string(),
                serde_json::Value::String(p.to_string_lossy().to_string()),
            );
        }
    };
    set_path("studiomdl_path", &tools.studiomdl);
    set_path("vbsp_path", &tools.vbsp);
    set_path("vvis_path", &tools.vvis);
    set_path("vrad_path", &tools.vrad);
    set_path("vtfcmd_path", &tools.vtfcmd);
    set_path("game_dir", &tools.game_dir);
    if let Some(ref h) = tools.hammer {
        set_path("hammer_path", h);
    }
    if let Some(ref vj) = tools.vanilla_jar {
        set_path("vanilla_jar_path", vj);
    }
    if let Some(ref wp) = tools.world_path {
        set_path("world_path", wp);
    }
    root_obj
        .entry("scale_factor".to_string())
        .or_insert_with(|| serde_json::json!(tools.scale_factor));
    root_obj
        .entry("max_convex_pieces".to_string())
        .or_insert_with(|| serde_json::json!(tools.max_convex_pieces));

    if let Some(parent) = config_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = config_path.with_extension("json.tmp");
    let serialized = serde_json::to_string_pretty(&serde_json::Value::Object(root_obj))
        .map_err(|e| e.to_string())?;
    std::fs::write(&tmp, &serialized)
        .and_then(|_| std::fs::rename(&tmp, config_path))
        .or_else(|_| std::fs::write(config_path, &serialized))
        .map_err(|e| e.to_string())?;

    Ok(tools)
}

#[derive(serde::Serialize)]
struct EnvCheckItem {
    key: &'static str,
    name: &'static str,
    category: &'static str,
    is_installed: bool,
    status_label: &'static str,
    found_path: Option<String>,
    config_key: Option<&'static str>,
}

pub fn run_env_check(
    config_path: Option<&Path>,
    simulate_clean: bool,
    lang: &str,
    output_json: bool,
) -> bool {
    let tools = if simulate_clean {
        ToolPaths {
            studiomdl: PathBuf::new(),
            vtfcmd: PathBuf::new(),
            vbsp: PathBuf::new(),
            vvis: PathBuf::new(),
            vrad: PathBuf::new(),
            gmad: None,
            game_dir: PathBuf::new(),
            vanilla_jar: None,
            hammer: None,
            world_path: None,
            scale_factor: 39.37,
            max_convex_pieces: DEFAULT_MAX_CONVEX_PIECES,
        }
    } else {
        load_tool_paths(config_path)
    };

    let make_item = |key: &'static str,
                     name: &'static str,
                     required: bool,
                     ok: bool,
                     p: Option<&Path>,
                     config_key: Option<&'static str>| {
        EnvCheckItem {
            key,
            name,
            category: if required { "required" } else { "optional" },
            is_installed: ok,
            status_label: if ok {
                "OK"
            } else if required {
                "MISSING"
            } else {
                "OPTIONAL_MISSING"
            },
            found_path: if ok {
                p.map(|x| x.to_string_lossy().to_string())
            } else {
                None
            },
            config_key,
        }
    };

    let items = vec![
        make_item(
            "studiomdl",
            "Garry's Mod StudioMDL Compiler (studiomdl.exe)",
            true,
            tools.studiomdl.is_file(),
            Some(&tools.studiomdl),
            Some("studiomdl_path"),
        ),
        make_item(
            "game_dir",
            "Garry's Mod Game Directory (garrysmod/)",
            true,
            tools.game_dir.is_dir(),
            Some(&tools.game_dir),
            Some("game_dir"),
        ),
        make_item(
            "vbsp",
            "Source Engine BSP Compiler (vbsp.exe)",
            true,
            tools.vbsp.is_file(),
            Some(&tools.vbsp),
            None,
        ),
        make_item(
            "vvis",
            "Source Engine VVIS (vvis.exe)",
            false,
            tools.vvis.is_file(),
            Some(&tools.vvis),
            None,
        ),
        make_item(
            "vrad",
            "Source Engine VRAD Lighting (vrad.exe)",
            true,
            tools.vrad.is_file(),
            Some(&tools.vrad),
            None,
        ),
        make_item(
            "vtfcmd",
            "VTFLib VTFCmd (VTFCmd.exe)",
            true,
            tools.vtfcmd.is_file(),
            Some(&tools.vtfcmd),
            Some("vtfcmd_path"),
        ),
        make_item(
            "hammer",
            "Valve Hammer Editor (hammer.exe)",
            false,
            tools.hammer.as_ref().map(|p| p.is_file()).unwrap_or(false),
            tools.hammer.as_deref(),
            Some("hammer_path"),
        ),
        make_item(
            "gmad",
            "Garry's Mod Addon Tool (gmad.exe)",
            false,
            tools.gmad.as_ref().map(|p| p.is_file()).unwrap_or(false),
            tools.gmad.as_deref(),
            None,
        ),
        make_item(
            "vanilla_jar",
            "Minecraft Vanilla Client JAR",
            false,
            tools
                .vanilla_jar
                .as_ref()
                .map(|p| is_valid_vanilla_jar(p))
                .unwrap_or(false),
            tools.vanilla_jar.as_deref(),
            Some("vanilla_jar_path"),
        ),
        make_item(
            "world_path",
            "Minecraft Saves Folder",
            false,
            tools.world_path.as_ref().map(|p| p.is_dir()).unwrap_or(false),
            tools.world_path.as_deref(),
            Some("world_path"),
        ),
    ];

    let req_total = items.iter().filter(|i| i.category == "required").count();
    let req_ok = items
        .iter()
        .filter(|i| i.category == "required" && i.is_installed)
        .count();
    let opt_total = items.iter().filter(|i| i.category == "optional").count();
    let opt_ok = items
        .iter()
        .filter(|i| i.category == "optional" && i.is_installed)
        .count();
    let is_ready = req_ok == req_total;

    if output_json {
        let payload = serde_json::json!({
            "simulate_clean": simulate_clean,
            "is_ready": is_ready,
            "required_count": req_total,
            "required_installed": req_ok,
            "optional_count": opt_total,
            "optional_installed": opt_ok,
            "results": items,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        let is_ru = lang.eq_ignore_ascii_case("ru");
        println!("{}", "=".repeat(78));
        if is_ru {
            println!("  MCGM — ПРОВЕРКА СИСТЕМНОГО ОКРУЖЕНИЯ И ТРЕБУЕМЫХ УТИЛИТ");
        } else {
            println!("  MCGM — SYSTEM ENVIRONMENT & DEPENDENCIES CHECK");
        }
        println!("{}", "=".repeat(78));
        for item in &items {
            let mark = if item.is_installed { "[OK]" } else if item.category == "required" { "[MISSING]" } else { "[OPTIONAL]" };
            println!(
                "  {:<10} {:<48} {}",
                mark,
                item.name,
                item.found_path.as_deref().unwrap_or("<not found>")
            );
        }
        println!("{}", "-".repeat(78));
        if is_ru {
            println!("  Обязательные: {}/{} | Дополнительные: {}/{}", req_ok, req_total, opt_ok, opt_total);
            println!("  Вердикт: {}", if is_ready { "ГОТОВО К РАБОТЕ" } else { "ОТСУТСТВУЮТ ОБЯЗАТЕЛЬНЫЕ УТИЛИТЫ" });
        } else {
            println!("  Required: {}/{} | Optional: {}/{}", req_ok, req_total, opt_ok, opt_total);
            println!("  Verdict: {}", if is_ready { "READY TO RUN" } else { "MISSING REQUIRED TOOLS" });
        }
        println!("{}", "=".repeat(78));
    }

    is_ready
}

#[allow(clippy::too_many_arguments)]
pub fn generate_vmt(
    output_path: &Path,
    shader: &str,
    basetexture: &str,
    surfaceprop: &str,
    alphatest: bool,
    translucent: bool,
    nocull: bool,
    nofog: bool,
    ignorez: bool,
) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let clean_tex = basetexture
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    let mut lines = vec![
        format!("\"{}\"", shader),
        "{".to_string(),
        format!("\t\"$basetexture\" \"{}\"", clean_tex),
    ];
    if shader.eq_ignore_ascii_case("VertexLitGeneric") {
        lines.push("\t\"$model\" \"1\"".to_string());
    }
    if !surfaceprop.is_empty() && !shader.eq_ignore_ascii_case("UnlitGeneric") {
        lines.push(format!("\t\"$surfaceprop\" \"{}\"", surfaceprop));
    }
    if alphatest {
        lines.push("\t\"$alphatest\" \"1\"".to_string());
        lines.push("\t\"$alphatestreference\" \"0.5\"".to_string());
    }
    if translucent {
        lines.push("\t\"$translucent\" \"1\"".to_string());
    }
    if nocull {
        lines.push("\t\"$nocull\" \"1\"".to_string());
    }
    if nofog {
        lines.push("\t\"$nofog\" \"1\"".to_string());
    }
    if ignorez {
        lines.push("\t\"$ignorez\" \"1\"".to_string());
    }
    lines.push("}".to_string());
    std::fs::write(output_path, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

fn convert_png_to_vtf_with_mode(
    vtfcmd: &Path,
    png_path: &Path,
    out_dir: &Path,
    mode: VtfMode,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    if !vtfcmd.is_file() {
        return Err(format!("VTFCmd.exe not found: {}", vtfcmd.display()));
    }
    let stem = png_path.file_stem().and_then(|s| s.to_str()).unwrap_or("tex");
    let out_vtf = out_dir.join(format!("{}.vtf", stem));

    let run_once = |m: VtfMode| -> Result<PathBuf, String> {
        let _ = std::fs::remove_file(&out_vtf);
        let mut cmd = Command::new(vtfcmd);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.arg("-file").arg(png_path);
        cmd.arg("-output").arg(out_dir);
        match m {
            VtfMode::Skybox => {
                cmd.args([
                    "-format",
                    "BGR888",
                    "-alphaformat",
                    "BGR888",
                    "-nomipmaps",
                    "-flag",
                    "CLAMPS",
                    "-flag",
                    "CLAMPT",
                    "-flag",
                    "POINTSAMPLE",
                ]);
            }
            VtfMode::AtlasDxt5 => {
                cmd.args([
                    "-format",
                    "DXT5",
                    "-alphaformat",
                    "DXT5",
                    "-mfilter",
                    "BOX",
                    "-flag",
                    "POINTSAMPLE",
                ]);
            }
            VtfMode::SignBgra8888 => {
                cmd.args([
                    "-format",
                    "BGRA8888",
                    "-alphaformat",
                    "BGRA8888",
                    "-mfilter",
                    "BOX",
                    "-flag",
                    "POINTSAMPLE",
                ]);
            }
            VtfMode::DoorDxt5NoMip => {
                cmd.args([
                    "-format",
                    "DXT5",
                    "-alphaformat",
                    "DXT5",
                    "-nomipmaps",
                    "-flag",
                    "POINTSAMPLE",
                ]);
            }
        }
        let out = cmd
            .output()
            .map_err(|e| format!("Failed spawning VTFCmd: {}", e))?;
        let produced =
            out_vtf.is_file() && out_vtf.metadata().map(|meta| meta.len() > 0).unwrap_or(false);
        if !produced || !out.status.success() {
            return Err(format!(
                "VTFCmd failed for {} (code {:?}):\nSTDOUT: {}\nSTDERR: {}",
                png_path.display(),
                out.status.code(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        if let Some(src_hash) = compute_file_fnv1a_hex(png_path) {
            let _ = std::fs::write(out_vtf.with_extension("vtf.src_hash"), src_hash);
        }
        Ok(out_vtf.clone())
    };

    match run_once(mode) {
        Ok(p) => Ok(p),
        Err(first_err) if mode == VtfMode::SignBgra8888 => {
            // Fallback to DXT5 if BGRA8888 fails
            run_once(VtfMode::AtlasDxt5).map_err(|fallback_err| {
                format!("{}\nFallback DXT5 also failed:\n{}", first_err, fallback_err)
            })
        }
        Err(e) => Err(e),
    }
}

pub fn scoped_map_category(category: &str, map_name: &str) -> String {
    let clean_cat = crate::cli::sanitize_safe_category(category);
    let clean_map = crate::cli::sanitize_safe_name(map_name, "gm_mc_export").to_ascii_lowercase();
    let last_seg = clean_cat.rsplit('/').next().unwrap_or("");
    if last_seg.eq_ignore_ascii_case(&clean_map) {
        clean_cat
    } else {
        format!("{}/{}", clean_cat, clean_map)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compile_materials(
    tools: &ToolPaths,
    category: &str,
    atlas_png: &Path,
    sign_atlas_png: &Path,
    door_pngs: &HashMap<String, PathBuf>,
    skybox_pngs: &[(String, PathBuf)],
    addon_dir: &Path,
    import_to_gmod: bool,
    reuse_cache: bool,
) -> Result<(), String> {
    let clean_cat = category.trim_matches(|c| c == '/' || c == '\\');
    let mut mat_dests = vec![addon_dir.join("materials")];
    if import_to_gmod && !tools.game_dir.as_os_str().is_empty() {
        mat_dests.push(tools.game_dir.join("materials"));
    }

    let tmp_vtf_dir = atlas_png.parent().unwrap_or(addon_dir).join("vtf_stage");
    let tmp_sky_dir = tmp_vtf_dir.join("skybox");
    std::fs::create_dir_all(&tmp_vtf_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&tmp_sky_dir).map_err(|e| e.to_string())?;

    let has_cached_vtf = |src_png: &Path, out_d: &Path| -> bool {
        if !reuse_cache {
            return false;
        }
        let Some(stem) = src_png.file_stem().and_then(|s| s.to_str()) else {
            return false;
        };
        let vtf = out_d.join(format!("{}.vtf", stem));
        let Ok(vtf_meta) = vtf.metadata() else {
            return false;
        };
        if !vtf_meta.is_file() || vtf_meta.len() == 0 {
            return false;
        }
        let hash_path = vtf.with_extension("vtf.src_hash");
        if hash_path.is_file() {
            return std::fs::read_to_string(&hash_path)
                .ok()
                .zip(compute_file_fnv1a_hex(src_png))
                .is_some_and(|(saved, cur)| saved.trim() == cur);
        }
        match (
            vtf_meta.modified().ok(),
            src_png.metadata().and_then(|m| m.modified()).ok(),
        ) {
            (Some(vtf_mtime), Some(png_mtime)) => vtf_mtime >= png_mtime,
            _ => false,
        }
    };

    // 1. Convert main atlas, sign atlas, doors, and skybox faces in parallel
    let mut jobs: Vec<(&Path, &Path, VtfMode)> = Vec::new();
    if !has_cached_vtf(atlas_png, tmp_vtf_dir.as_path()) {
        jobs.push((atlas_png, tmp_vtf_dir.as_path(), VtfMode::AtlasDxt5));
    }
    jobs.push((sign_atlas_png, tmp_vtf_dir.as_path(), VtfMode::SignBgra8888));
    for p in door_pngs.values() {
        jobs.push((p.as_path(), tmp_vtf_dir.as_path(), VtfMode::DoorDxt5NoMip));
    }
    for (_, p) in skybox_pngs {
        if !has_cached_vtf(p.as_path(), tmp_sky_dir.as_path()) {
            jobs.push((p.as_path(), tmp_sky_dir.as_path(), VtfMode::Skybox));
        }
    }

    jobs.par_iter().try_for_each(|&(src_png, out_d, mode)| {
        convert_png_to_vtf_with_mode(&tools.vtfcmd, src_png, out_d, mode).map(|_| ())
    })?;

    let atlas_stem = atlas_png
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("atlas");
    let sign_stem = sign_atlas_png
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sign_atlas");

    // 2. Write VMTs and copy VTFs to addon + game_dir
    for base_mat_dir in &mat_dests {
        let model_mat_dir = base_mat_dir.join("models").join(clean_cat);
        let brush_mat_dir = base_mat_dir.join(clean_cat);
        let sky_mat_dir = base_mat_dir.join("skybox");
        std::fs::create_dir_all(&model_mat_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&brush_mat_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&sky_mat_dir).map_err(|e| e.to_string())?;

        // Copy atlas VTF + sign atlas VTF into materials/models/<cat>/
        for stem in [atlas_stem, sign_stem] {
            let src = tmp_vtf_dir.join(format!("{}.vtf", stem));
            if src.is_file() {
                std::fs::copy(&src, model_mat_dir.join(format!("{}.vtf", stem)))
                    .map_err(|e| e.to_string())?;
            }
        }

        // Generate atlas_brush.vmt in materials/<cat>/ for brush interactables,
        // referencing models/<cat>/<atlas_stem> to avoid duplicating the large atlas VTF.
        generate_vmt(
            &brush_mat_dir.join("atlas_brush.vmt"),
            "LightmappedGeneric",
            &format!("models/{}/{}", clean_cat, atlas_stem),
            "dirt",
            true,
            false,
            true,
            false,
            false,
        )?;

        // Generate per-surfaceprop invisible clip VMTs in materials/<cat>/col_<surf>.vmt
        // for func_detail collision brushes so each block type has accurate footsteps/impacts.
        for sp in crate::collision::SurfaceProp::ALL {
            let surf = sp.as_source_str();
            let vmt_body = format!(
                "\"LightmappedGeneric\"\n{{\n    \"$basetexture\" \"tools/toolsinvisible\"\n    \"$surfaceprop\" \"{}\"\n    \"%compilenodraw\" 1\n    \"%compilepassbullets\" 0\n}}\n",
                surf
            );
            std::fs::write(brush_mat_dir.join(format!("col_{}.vmt", surf)), vmt_body)
                .map_err(|e| e.to_string())?;
        }

        // Bucket VMTs pointing to models/<cat>/<atlas_stem>
        let btex = format!("models/{}/{}", clean_cat, atlas_stem);
        generate_vmt(
            &model_mat_dir.join(format!("{}.vmt", RenderBucket::TerrainOpaque.material_name())),
            "VertexLitGeneric",
            &btex,
            "rock",
            false,
            false,
            false,
            false,
            false,
        )?;
        generate_vmt(
            &model_mat_dir.join(format!("{}.vmt", RenderBucket::TerrainCutout.material_name())),
            "VertexLitGeneric",
            &btex,
            "grass",
            true,
            false,
            true,
            false,
            false,
        )?;
        generate_vmt(
            &model_mat_dir.join(format!(
                "{}.vmt",
                RenderBucket::TerrainCutoutCull.material_name()
            )),
            "VertexLitGeneric",
            &btex,
            "grass",
            true,
            false,
            false,
            false,
            false,
        )?;
        generate_vmt(
            &model_mat_dir.join(format!(
                "{}.vmt",
                RenderBucket::TerrainTranslucent.material_name()
            )),
            "VertexLitGeneric",
            &btex,
            "glass",
            false,
            true,
            false,
            false,
            false,
        )?;

        let sign_btex = format!("models/{}/{}", clean_cat, sign_stem);
        generate_vmt(
            &model_mat_dir.join("sign_atlas.vmt"),
            "VertexLitGeneric",
            &sign_btex,
            "wood",
            false,
            false,
            false,
            false,
            false,
        )?;

        // Doors (LightmappedGeneric for func_door_rotating brushes + VertexLitGeneric for models,
        // sharing the single VTF in materials/models/<cat>/)
        for mat_stem in door_pngs.keys() {
            let vtf_src = tmp_vtf_dir.join(format!("{}.vtf", mat_stem));
            if vtf_src.is_file() {
                std::fs::copy(&vtf_src, model_mat_dir.join(format!("{}.vtf", mat_stem)))
                    .map_err(|e| e.to_string())?;
            }
            let stem_lower = mat_stem.to_ascii_lowercase();
            let door_surface = if stem_lower.contains("iron") || stem_lower.contains("anvil") {
                "metal"
            } else if stem_lower.contains("stone")
                || stem_lower.contains("blackstone")
                || stem_lower.contains("andesite")
                || stem_lower.contains("cobble")
                || stem_lower.contains("lever")
            {
                "rock"
            } else {
                "wood"
            };
            let needs_alphatest = stem_lower.ends_with("_door");
            let d_tex = format!("models/{}/{}", clean_cat, mat_stem);
            generate_vmt(
                &brush_mat_dir.join(format!("{}.vmt", mat_stem)),
                "LightmappedGeneric",
                &d_tex,
                door_surface,
                needs_alphatest,
                false,
                false,
                false,
                false,
            )?;
            generate_vmt(
                &model_mat_dir.join(format!("{}.vmt", mat_stem)),
                "VertexLitGeneric",
                &d_tex,
                door_surface,
                needs_alphatest,
                false,
                false,
                false,
                false,
            )?;
        }

        // Skybox faces
        for (sky_face_name, _) in skybox_pngs {
            let vtf_src = tmp_sky_dir.join(format!("{}.vtf", sky_face_name));
            if vtf_src.is_file() {
                std::fs::copy(&vtf_src, sky_mat_dir.join(format!("{}.vtf", sky_face_name)))
                    .map_err(|e| e.to_string())?;
            }
            generate_vmt(
                &sky_mat_dir.join(format!("{}.vmt", sky_face_name)),
                "UnlitGeneric",
                &format!("skybox/{}", sky_face_name),
                "",
                false,
                false,
                false,
                true,
                true,
            )?;
        }
    }

    Ok(())
}

/// Processes a single Minecraft water frame into a 64x64 tinted RGBA tile.
fn process_water_frame(src: &image::RgbaImage, is_flow: bool, tint: [f32; 3]) -> image::RgbaImage {
    let base_side = src.width().min(src.height()).max(16);
    // Vanilla water_flow.png is 32x32 per frame (2x2 tiles of 16x16); crop one 16x16 tile so
    // flowing water matches the 16x16/block texel density of water_still and solid blocks.
    let tile_side = if is_flow && base_side >= 32 {
        (base_side / 2).max(16)
    } else {
        base_side
    };
    let cropped = image::imageops::crop_imm(src, 0, 0, tile_side, tile_side).to_image();
    let mut up = image::imageops::resize(
        &cropped,
        64,
        64,
        image::imageops::FilterType::Nearest,
    );
    for px in up.pixels_mut() {
        px[0] = ((px[0] as f32 * tint[0]).round()).clamp(0.0, 255.0) as u8;
        px[1] = ((px[1] as f32 * tint[1]).round()).clamp(0.0, 255.0) as u8;
        px[2] = ((px[2] as f32 * tint[2]).round()).clamp(0.0, 255.0) as u8;
        px[3] = 215;
    }
    up
}

/// Exports 64x64 Minecraft `water_still` and `water_flow` PNGs (plus multi-frame VTFs when
/// animated strips are present and `mc_water_fps.txt`) tinted with authentic Minecraft water
/// blue `[63, 118, 228]` for Source Engine `AnimatedTexture` flipbook water materials.
pub fn export_animated_water_textures(
    assets: &crate::assets::AssetIndex,
    out_dir: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let still_path = out_dir.join("mc_water.png");
    let flow_path = out_dir.join("mc_water_flow.png");
    let tint = [63.0f32 / 255.0, 118.0 / 255.0, 228.0 / 255.0];

    let export_water_tex =
        |tex_ref: &str, is_flow: bool, png_path: &Path, default_fps: f32| -> Result<f32, String> {
            if let Some(strip) = assets
                .load_animated_texture_frames(tex_ref)
                .filter(|s| s.frames.len() >= 2)
            {
                let frames: Vec<image::RgbaImage> = strip
                    .frames
                    .iter()
                    .map(|f| process_water_frame(f, is_flow, tint))
                    .collect();
                crate::atlas::save_png_fast(&frames[0], png_path)?;
                write_multiframe_vtf(&png_path.with_extension("vtf"), &frames)?;
                return Ok(strip.fps);
            }
            let img = if let Some(src) = assets.load_texture_image(tex_ref) {
                process_water_frame(&src, is_flow, tint)
            } else {
                let mut img = image::ImageBuffer::new(64, 64);
                for y in 0..64u32 {
                    for x in 0..64u32 {
                        let bx = x / 4;
                        let by = y / 4;
                        let wave = if is_flow {
                            ((bx * 7 + by * 3) % 5) as i32 - 2
                        } else {
                            (((bx * 5 + by * 11) ^ (bx * 3)) % 5) as i32 - 2
                        };
                        let r = (48 + wave * 4).clamp(24, 96) as u8;
                        let g = (106 + wave * 6).clamp(72, 150) as u8;
                        let b = (218 + wave * 6).clamp(180, 245) as u8;
                        img.put_pixel(x, y, image::Rgba([r, g, b, 215]));
                    }
                }
                img
            };
            crate::atlas::save_png_fast(&img, png_path)?;
            Ok(default_fps)
        };

    let still_fps = export_water_tex("minecraft:block/water_still", false, &still_path, 10.0)?;
    let flow_fps = export_water_tex("minecraft:block/water_flow", true, &flow_path, 20.0)?;
    std::fs::write(
        out_dir.join("mc_water_fps.txt"),
        format!("{:.2}\n{:.2}\n", still_fps, flow_fps),
    )
    .map_err(|e| e.to_string())?;
    Ok((still_path, flow_path))
}

/// Compiles flipbook-animated Minecraft water VTFs & VMTs (`AnimatedTexture` proxy):
/// - `materials/<cat>/mc_water.vmt` & `mc_water_beneath.vmt` (`LightmappedGeneric` + `%compilewater 1`,
///   avoids Source reflective water multi-height black bug while preserving native `CONTENTS_WATER` swimming)
/// - `materials/models/<cat>/mc_water_flow.vmt` (`VertexLitGeneric` flipbook for waterfalls)
pub fn compile_water_materials(
    tools: &ToolPaths,
    category: &str,
    water_still_png: &Path,
    water_flow_png: &Path,
    addon_dir: &Path,
    import_to_gmod: bool,
) -> Result<(), String> {
    let clean_cat = category.trim_matches(|c| c == '/' || c == '\\');
    let tmp_dir = water_still_png.parent().unwrap_or(addon_dir).join("vtf_water");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;

    let resolve_water_vtf = |png_path: &Path| -> Result<PathBuf, String> {
        let prebuilt = png_path.with_extension("vtf");
        if prebuilt.is_file() {
            Ok(prebuilt)
        } else {
            convert_png_to_vtf_with_mode(&tools.vtfcmd, png_path, &tmp_dir, VtfMode::AtlasDxt5)
        }
    };
    let still_vtf = resolve_water_vtf(water_still_png)?;
    let flow_vtf = resolve_water_vtf(water_flow_png)?;

    let (still_fps, flow_fps) = water_still_png
        .parent()
        .and_then(|p| std::fs::read_to_string(p.join("mc_water_fps.txt")).ok())
        .map(|txt| {
            let mut lines = txt.lines();
            let s = lines
                .next()
                .and_then(|l| l.trim().parse::<f32>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(10.0);
            let f = lines
                .next()
                .and_then(|l| l.trim().parse::<f32>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(20.0);
            (s, f)
        })
        .unwrap_or((10.0f32, 20.0f32));

    let animated_proxy = |fps: f32| -> String {
        format!(
            "\t\"Proxies\"\n\
             \t{{\n\
             \t\t\"AnimatedTexture\"\n\
             \t\t{{\n\
             \t\t\t\"animatedTextureVar\" \"$basetexture\"\n\
             \t\t\t\"animatedTextureFrameNumVar\" \"$frame\"\n\
             \t\t\t\"animatedTextureFrameRate\" \"{:.2}\"\n\
             \t\t}}\n\
             \t}}\n",
            fps
        )
    };
    let still_proxy = animated_proxy(still_fps);
    let flow_proxy = animated_proxy(flow_fps);

    let brush_vmt = format!(
        "\"LightmappedGeneric\"\n{{\n\
         \t\"$basetexture\" \"{cat}/mc_water\"\n\
         \t\"$translucent\" \"1\"\n\
         \t\"$alpha\" \"0.78\"\n\
         \t\"$surfaceprop\" \"water\"\n\
         \t\"%compilewater\" \"1\"\n\
         \t\"$abovewater\" \"1\"\n\
         \t\"$bottommaterial\" \"{cat}/mc_water_beneath\"\n\
         \t\"$fogenable\" \"1\"\n\
         \t\"$fogcolor\" \"{{32 68 158}}\"\n\
         \t\"$fogstart\" \"0\"\n\
         \t\"$fogend\" \"450\"\n\
         {proxy}}}\n",
        cat = clean_cat,
        proxy = still_proxy
    );

    let beneath_vmt = format!(
        "\"LightmappedGeneric\"\n{{\n\
         \t\"$basetexture\" \"{cat}/mc_water\"\n\
         \t\"$translucent\" \"1\"\n\
         \t\"$alpha\" \"0.68\"\n\
         \t\"$surfaceprop\" \"water\"\n\
         \t\"$abovewater\" \"0\"\n\
         \t\"$fogenable\" \"1\"\n\
         \t\"$fogcolor\" \"{{32 68 158}}\"\n\
         \t\"$fogstart\" \"0\"\n\
         \t\"$fogend\" \"450\"\n\
         {proxy}}}\n",
        cat = clean_cat,
        proxy = still_proxy
    );

    let flow_vmt = format!(
        "\"VertexLitGeneric\"\n{{\n\
         \t\"$basetexture\" \"models/{cat}/mc_water_flow\"\n\
         \t\"$model\" \"1\"\n\
         \t\"$translucent\" \"1\"\n\
         \t\"$alpha\" \"0.78\"\n\
         \t\"$nocull\" \"1\"\n\
         \t\"$surfaceprop\" \"water\"\n\
         {proxy}}}\n",
        cat = clean_cat,
        proxy = flow_proxy
    );

    let mut mat_dests = vec![addon_dir.join("materials")];
    if import_to_gmod && !tools.game_dir.as_os_str().is_empty() {
        mat_dests.push(tools.game_dir.join("materials"));
    }

    for base_mat_dir in &mat_dests {
        let brush_dir = base_mat_dir.join(clean_cat);
        let model_dir = base_mat_dir.join("models").join(clean_cat);
        std::fs::create_dir_all(&brush_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&model_dir).map_err(|e| e.to_string())?;

        std::fs::copy(&still_vtf, brush_dir.join("mc_water.vtf")).map_err(|e| e.to_string())?;
        std::fs::write(brush_dir.join("mc_water.vmt"), &brush_vmt).map_err(|e| e.to_string())?;
        std::fs::write(brush_dir.join("mc_water_beneath.vmt"), &beneath_vmt)
            .map_err(|e| e.to_string())?;

        std::fs::copy(&flow_vtf, model_dir.join("mc_water_flow.vtf"))
            .map_err(|e| e.to_string())?;
        std::fs::write(model_dir.join("mc_water_flow.vmt"), &flow_vmt)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Writes an uncompressed BGRA8888 multi-frame VTF v7.2 with full mipmap chain.
#[allow(clippy::chunks_exact_to_as_chunks)]
pub fn write_multiframe_vtf(path: &Path, frames: &[image::RgbaImage]) -> Result<(), String> {
    if frames.is_empty() {
        return Err("write_multiframe_vtf called with 0 frames".to_string());
    }
    let w = frames[0].width();
    let h = frames[0].height();
    if w == 0 || h == 0 || !w.is_power_of_two() || !h.is_power_of_two() {
        return Err(format!("VTF dimensions must be non-zero powers of 2, got {}x{}", w, h));
    }
    for (idx, f) in frames.iter().enumerate() {
        if f.width() != w || f.height() != h {
            return Err(format!(
                "Frame {} dimensions {}x{} do not match frame 0 {}x{}",
                idx,
                f.width(),
                f.height(),
                w,
                h
            ));
        }
    }

    let max_dim = w.max(h);
    let mip_count = (max_dim.trailing_zeros() + 1) as u8;
    let num_frames = frames.len() as u16;

    // Average reflectivity from frame 0
    let mut sum_r = 0.0f64;
    let mut sum_g = 0.0f64;
    let mut sum_b = 0.0f64;
    let px_count = (w * h) as f64;
    for px in frames[0].pixels() {
        sum_r += (px[0] as f64) / 255.0;
        sum_g += (px[1] as f64) / 255.0;
        sum_b += (px[2] as f64) / 255.0;
    }
    let refl = [
        (sum_r / px_count) as f32,
        (sum_g / px_count) as f32,
        (sum_b / px_count) as f32,
    ];

    // Build mipmaps [frame][mip_level_0_largest .. mip_level_last_1x1]
    let downsample_box = |src: &image::RgbaImage| -> image::RgbaImage {
        let (sw, sh) = src.dimensions();
        let dw = (sw / 2).max(1);
        let dh = (sh / 2).max(1);
        let mut dst = image::RgbaImage::new(dw, dh);
        for dy in 0..dh {
            for dx in 0..dw {
                let sx0 = (dx * 2).min(sw - 1);
                let sx1 = (dx * 2 + 1).min(sw - 1);
                let sy0 = (dy * 2).min(sh - 1);
                let sy1 = (dy * 2 + 1).min(sh - 1);
                let pts = [
                    src.get_pixel(sx0, sy0),
                    src.get_pixel(sx1, sy0),
                    src.get_pixel(sx0, sy1),
                    src.get_pixel(sx1, sy1),
                ];
                let r = ((pts[0][0] as u32 + pts[1][0] as u32 + pts[2][0] as u32 + pts[3][0] as u32 + 2) / 4) as u8;
                let g = ((pts[0][1] as u32 + pts[1][1] as u32 + pts[2][1] as u32 + pts[3][1] as u32 + 2) / 4) as u8;
                let b = ((pts[0][2] as u32 + pts[1][2] as u32 + pts[2][2] as u32 + pts[3][2] as u32 + 2) / 4) as u8;
                let a = ((pts[0][3] as u32 + pts[1][3] as u32 + pts[2][3] as u32 + pts[3][3] as u32 + 2) / 4) as u8;
                dst.put_pixel(dx, dy, image::Rgba([r, g, b, a]));
            }
        }
        dst
    };

    let mut all_mips: Vec<Vec<image::RgbaImage>> = Vec::with_capacity(frames.len());
    for f in frames {
        let mut chain = Vec::with_capacity(mip_count as usize);
        chain.push(f.clone());
        for _ in 1..mip_count {
            let prev = chain.last().unwrap();
            chain.push(downsample_box(prev));
        }
        all_mips.push(chain);
    }

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::with_capacity(64 * 1024, file);
    use std::io::Write as _;

    // VTF v7.2 80-byte header
    let mut hdr = [0u8; 80];
    hdr[0..4].copy_from_slice(b"VTF\0");
    hdr[4..8].copy_from_slice(&7u32.to_le_bytes());
    hdr[8..12].copy_from_slice(&2u32.to_le_bytes());
    hdr[12..16].copy_from_slice(&80u32.to_le_bytes());
    hdr[16..18].copy_from_slice(&(w as u16).to_le_bytes());
    hdr[18..20].copy_from_slice(&(h as u16).to_le_bytes());
    let flags: u32 = 0x0001 | 0x2000; // POINTSAMPLE | EIGHTBITALPHA
    hdr[20..24].copy_from_slice(&flags.to_le_bytes());
    hdr[24..26].copy_from_slice(&num_frames.to_le_bytes());
    hdr[26..28].copy_from_slice(&0u16.to_le_bytes()); // firstFrame
    hdr[32..36].copy_from_slice(&refl[0].to_le_bytes());
    hdr[36..40].copy_from_slice(&refl[1].to_le_bytes());
    hdr[40..44].copy_from_slice(&refl[2].to_le_bytes());
    hdr[48..52].copy_from_slice(&1.0f32.to_le_bytes()); // bumpmapScale
    hdr[52..56].copy_from_slice(&12u32.to_le_bytes()); // IMAGE_FORMAT_BGRA8888
    hdr[56] = mip_count;
    hdr[57..61].copy_from_slice(&(-1i32).to_le_bytes()); // IMAGE_FORMAT_NONE
    hdr[61] = 0; // lowResImageWidth
    hdr[62] = 0; // lowResImageHeight
    hdr[63..65].copy_from_slice(&1u16.to_le_bytes()); // depth = 1
    out.write_all(&hdr).map_err(|e| e.to_string())?;

    // Mips are stored smallest-to-largest; within each mip, frames 0..N-1 in BGRA order.
    let mut bgra_buf = Vec::with_capacity((w * h * 4) as usize);
    for mip_idx in (0..mip_count as usize).rev() {
        for f_chain in &all_mips {
            let img = &f_chain[mip_idx];
            bgra_buf.clear();
            for px in img.as_raw().chunks_exact(4) {
                bgra_buf.push(px[2]); // B
                bgra_buf.push(px[1]); // G
                bgra_buf.push(px[0]); // R
                bgra_buf.push(px[3]); // A
            }
            out.write_all(&bgra_buf).map_err(|e| e.to_string())?;
        }
    }
    out.flush().map_err(|e| e.to_string())
}

pub fn compile_animated_materials(
    tools: &ToolPaths,
    category: &str,
    entries: &[crate::atlas::AnimatedMaterialEntry],
    addon_dir: &Path,
    import_to_gmod: bool,
) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let clean_cat = category.trim_matches(|c| c == '/' || c == '\\');
    let mut mat_dests = vec![addon_dir.join("materials")];
    if import_to_gmod && !tools.game_dir.as_os_str().is_empty() {
        mat_dests.push(tools.game_dir.join("materials"));
    }

    for base_mat_dir in &mat_dests {
        let model_dir = base_mat_dir.join("models").join(clean_cat);
        std::fs::create_dir_all(&model_dir).map_err(|e| e.to_string())?;
        for entry in entries {
            let vtf_path = model_dir.join(format!("{}.vtf", entry.mat_name));
            write_multiframe_vtf(&vtf_path, &entry.frames)?;

            let shader = if entry.unlit_emissive {
                "UnlitGeneric"
            } else {
                "VertexLitGeneric"
            };
            let mut lines = vec![
                format!("\"{}\"", shader),
                "{".to_string(),
                format!("\t\"$basetexture\" \"models/{}/{}\"", clean_cat, entry.mat_name),
                "\t\"$model\" \"1\"".to_string(),
            ];
            match entry.bucket {
                RenderBucket::TerrainCutout => {
                    lines.push("\t\"$alphatest\" \"1\"".to_string());
                    lines.push("\t\"$alphatestreference\" \"0.5\"".to_string());
                    lines.push("\t\"$nocull\" \"1\"".to_string());
                }
                RenderBucket::TerrainCutoutCull => {
                    lines.push("\t\"$alphatest\" \"1\"".to_string());
                    lines.push("\t\"$alphatestreference\" \"0.5\"".to_string());
                }
                RenderBucket::TerrainTranslucent => {
                    lines.push("\t\"$translucent\" \"1\"".to_string());
                    lines.push("\t\"$nocull\" \"1\"".to_string());
                }
                RenderBucket::TerrainOpaque => {}
            }
            lines.push("\t\"Proxies\"".to_string());
            lines.push("\t{".to_string());
            lines.push("\t\t\"AnimatedTexture\"".to_string());
            lines.push("\t\t{".to_string());
            lines.push("\t\t\t\"animatedTextureVar\" \"$basetexture\"".to_string());
            lines.push("\t\t\t\"animatedTextureFrameNumVar\" \"$frame\"".to_string());
            lines.push(format!("\t\t\t\"animatedTextureFrameRate\" \"{:.2}\"", entry.fps));
            lines.push("\t\t}".to_string());
            lines.push("\t}".to_string());
            lines.push("}".to_string());
            std::fs::write(
                model_dir.join(format!("{}.vmt", entry.mat_name)),
                lines.join("\n") + "\n",
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn compile_chunk_models_cached<F>(
    tools: &ToolPaths,
    chunks: &mut [CompiledChunkArtifact],
    addon_dir: &Path,
    import_to_gmod: bool,
    max_workers: usize,
    reuse_existing: bool,
    on_job_progress: F,
) -> Result<usize, String>
where
    F: Fn(usize, usize, usize, &str) + Sync,
{
    if !tools.studiomdl.is_file() {
        return Err(format!(
            "studiomdl.exe not found: {}",
            tools.studiomdl.display()
        ));
    }
    if !tools.game_dir.is_dir() {
        return Err(format!(
            "Garry's Mod game_dir not found: {}",
            tools.game_dir.display()
        ));
    }
    let game_models_dir = tools.game_dir.join("models");
    let addon_models_dir = addon_dir.join("models");
    std::fs::create_dir_all(&game_models_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&addon_models_dir).map_err(|e| e.to_string())?;

    #[derive(Clone, Copy)]
    enum QcJobKind {
        Main { chunk_idx: usize },
        Translucent { chunk_idx: usize },
        ExtraCol { chunk_idx: usize, col_idx: usize },
    }
    struct QcJob {
        kind: QcJobKind,
        chunk_name: String,
        qc_path: PathBuf,
        mdl_rel: String,
        weight: u64,
    }
    enum QcOutcome {
        Main { chunk_idx: usize, used_fallback: bool },
        Translucent { chunk_idx: usize, ok: bool },
        ExtraCol { chunk_idx: usize, col_idx: usize, ok: bool },
    }

    let mut jobs: Vec<QcJob> = Vec::with_capacity(chunks.len() * 2);
    for (chunk_idx, ch) in chunks.iter().enumerate() {
        if ch.qc_path.is_file() {
            let weight = ch
                .qc_path
                .with_extension("smd")
                .metadata()
                .map(|m| m.len())
                .unwrap_or(0)
                .saturating_add(1_000_000_000);
            jobs.push(QcJob {
                kind: QcJobKind::Main { chunk_idx },
                chunk_name: ch.chunk_name.clone(),
                qc_path: ch.qc_path.clone(),
                mdl_rel: ch.model_rel_path.clone(),
                weight,
            });
        }
        if let (Some(ref t_qc), Some(ref t_mdl)) =
            (&ch.translucent_qc_path, &ch.translucent_model_rel_path)
        {
            if t_qc.is_file() {
                let weight = t_qc
                    .with_extension("smd")
                    .metadata()
                    .map(|m| m.len())
                    .unwrap_or(0);
                jobs.push(QcJob {
                    kind: QcJobKind::Translucent { chunk_idx },
                    chunk_name: ch.chunk_name.clone(),
                    qc_path: t_qc.clone(),
                    mdl_rel: t_mdl.clone(),
                    weight,
                });
            }
        }
        for (col_idx, (col_qc, col_mdl)) in ch.extra_collision_models.iter().enumerate() {
            if col_qc.is_file() {
                let weight = col_qc
                    .with_extension("smd")
                    .metadata()
                    .map(|m| m.len())
                    .unwrap_or(0);
                jobs.push(QcJob {
                    kind: QcJobKind::ExtraCol { chunk_idx, col_idx },
                    chunk_name: ch.chunk_name.clone(),
                    qc_path: col_qc.clone(),
                    mdl_rel: col_mdl.clone(),
                    weight,
                });
            }
        }
    }

    // Schedule heaviest QCs first to minimize parallel tail latency
    jobs.sort_by_key(|a| std::cmp::Reverse(a.weight));

    let is_studiomdl_pp = tools
        .studiomdl
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.contains("++"))
        .unwrap_or(false);
    // 64-bit studiomdl++ uses mimalloc arenas per process; cap to 4 workers to avoid OS commit exhaustion
    let num_threads = if is_studiomdl_pp {
        max_workers.clamp(1, 4)
    } else {
        max_workers.clamp(1, 16)
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .map_err(|e| e.to_string())?;

    let is_nonempty = |p: &Path| p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false);
    let all_job_rels: Vec<String> = jobs.iter().map(|j| j.mdl_rel.clone()).collect();
    let _local_guard = if !import_to_gmod {
        let backup_root = std::env::temp_dir().join("mcgm_staged_backup");
        Some(StagedModelGuard::prepare(
            &tools.game_dir,
            &all_job_rels,
            &backup_root,
        )?)
    } else {
        None
    };
    let total_jobs = jobs.len();
    let done_jobs = std::sync::atomic::AtomicUsize::new(0);
    let cached_jobs = std::sync::atomic::AtomicUsize::new(0);

    let results: Vec<Result<QcOutcome, String>> = pool.install(|| {
        jobs.into_par_iter()
            .map(|job| {
                let mut was_cached = false;
                let outcome: Result<QcOutcome, String> = match job.kind {
                    QcJobKind::Main { chunk_idx } => {
                        let mut used_fallback = false;
                        let qc_has_collision = std::fs::read_to_string(&job.qc_path)
                            .map(|t| t.contains("$collisionmodel"))
                            .unwrap_or(false);
                        let expected_mdl = tools.game_dir.join(&job.mdl_rel);
                        let expected_phy = expected_mdl.with_extension("phy");
                        let addon_mdl = addon_dir.join(&job.mdl_rel);
                        let addon_phy = addon_mdl.with_extension("phy");

                        if reuse_existing
                            && is_nonempty(&addon_mdl)
                            && (!qc_has_collision || is_nonempty(&addon_phy))
                            && qc_cache_signature_matches(&job.qc_path, &addon_mdl)
                        {
                            if import_to_gmod {
                                sync_cached_model_files(addon_dir, &tools.game_dir, &job.mdl_rel)?;
                            }
                            was_cached = true;
                            let has_phy = is_nonempty(&addon_phy);
                            Ok(QcOutcome::Main {
                                chunk_idx,
                                used_fallback: qc_has_collision && !has_phy,
                            })
                        } else {
                            for p in model_sidecar_paths(&tools.game_dir, &job.mdl_rel) {
                                let _ = std::fs::remove_file(p);
                            }
                            if let Some(parent) = expected_mdl.parent() {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            let (ok, first_log) =
                                run_studiomdl(&tools.studiomdl, &tools.game_dir, &job.qc_path);
                            let has_mdl = is_nonempty(&expected_mdl);
                            let has_phy = is_nonempty(&expected_phy);

                            if !ok || !has_mdl {
                                if qc_has_collision {
                                    strip_collision_from_qc(&job.qc_path)?;
                                    let (retry_ok, retry_log) = run_studiomdl_verbose(
                                        &tools.studiomdl,
                                        &tools.game_dir,
                                        &job.qc_path,
                                    );
                                    let retry_has_mdl = is_nonempty(&expected_mdl);
                                    if !retry_ok || !retry_has_mdl {
                                        return Err(format!(
                                            "StudioMDL failed for {}:\n{}\n--- RETRY WITHOUT COLLISION ---\n{}",
                                            job.chunk_name, first_log, retry_log
                                        ));
                                    }
                                    used_fallback = true;
                                } else {
                                    let (_, verbose_log) = run_studiomdl_verbose(
                                        &tools.studiomdl,
                                        &tools.game_dir,
                                        &job.qc_path,
                                    );
                                    return Err(format!(
                                        "StudioMDL failed for {}:\n{}",
                                        job.chunk_name,
                                        if verbose_log.is_empty() {
                                            first_log
                                        } else {
                                            verbose_log
                                        }
                                    ));
                                }
                            } else if qc_has_collision && !has_phy {
                                used_fallback = true;
                            }

                            copy_compiled_model_files(
                                &tools.game_dir,
                                addon_dir,
                                &job.mdl_rel,
                                true,
                            )?;
                            write_qc_cache_signature(&job.qc_path, &addon_mdl);
                            Ok(QcOutcome::Main {
                                chunk_idx,
                                used_fallback,
                            })
                        }
                    }
                    QcJobKind::Translucent { chunk_idx } => {
                        let expected_t_mdl = tools.game_dir.join(&job.mdl_rel);
                        let addon_t_mdl = addon_dir.join(&job.mdl_rel);
                        if reuse_existing
                            && is_nonempty(&addon_t_mdl)
                            && qc_cache_signature_matches(&job.qc_path, &addon_t_mdl)
                        {
                            if import_to_gmod {
                                sync_cached_model_files(addon_dir, &tools.game_dir, &job.mdl_rel)?;
                            }
                            was_cached = true;
                            Ok(QcOutcome::Translucent { chunk_idx, ok: true })
                        } else {
                            for p in model_sidecar_paths(&tools.game_dir, &job.mdl_rel) {
                                let _ = std::fs::remove_file(p);
                            }
                            if let Some(parent) = expected_t_mdl.parent() {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            let (t_status, _t_log) =
                                run_studiomdl(&tools.studiomdl, &tools.game_dir, &job.qc_path);
                            let t_has_mdl = is_nonempty(&expected_t_mdl);
                            let ok = t_status
                                && t_has_mdl
                                && copy_compiled_model_files(
                                    &tools.game_dir,
                                    addon_dir,
                                    &job.mdl_rel,
                                    true,
                                )
                                .is_ok();
                            if ok {
                                write_qc_cache_signature(&job.qc_path, &addon_t_mdl);
                            }
                            Ok(QcOutcome::Translucent { chunk_idx, ok })
                        }
                    }
                    QcJobKind::ExtraCol { chunk_idx, col_idx } => {
                        let expected_c_mdl = tools.game_dir.join(&job.mdl_rel);
                        let expected_c_phy = expected_c_mdl.with_extension("phy");
                        let addon_c_mdl = addon_dir.join(&job.mdl_rel);
                        let addon_c_phy = addon_c_mdl.with_extension("phy");
                        if reuse_existing
                            && is_nonempty(&addon_c_mdl)
                            && is_nonempty(&addon_c_phy)
                            && qc_cache_signature_matches(&job.qc_path, &addon_c_mdl)
                        {
                            if import_to_gmod {
                                sync_cached_model_files(addon_dir, &tools.game_dir, &job.mdl_rel)?;
                            }
                            was_cached = true;
                            Ok(QcOutcome::ExtraCol {
                                chunk_idx,
                                col_idx,
                                ok: true,
                            })
                        } else {
                            for p in model_sidecar_paths(&tools.game_dir, &job.mdl_rel) {
                                let _ = std::fs::remove_file(p);
                            }
                            if let Some(parent) = expected_c_mdl.parent() {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            let (c_status, _c_log) =
                                run_studiomdl(&tools.studiomdl, &tools.game_dir, &job.qc_path);
                            let has_c_mdl = is_nonempty(&expected_c_mdl);
                            let has_c_phy = is_nonempty(&expected_c_phy);
                            let ok = c_status
                                && has_c_mdl
                                && has_c_phy
                                && copy_compiled_model_files(
                                    &tools.game_dir,
                                    addon_dir,
                                    &job.mdl_rel,
                                    true,
                                )
                                .is_ok();
                            if ok {
                                write_qc_cache_signature(&job.qc_path, &addon_c_mdl);
                            }
                            Ok(QcOutcome::ExtraCol {
                                chunk_idx,
                                col_idx,
                                ok,
                            })
                        }
                    }
                };
                if was_cached {
                    cached_jobs.fetch_add(1, Ordering::Relaxed);
                }
                let done = done_jobs.fetch_add(1, Ordering::Relaxed) + 1;
                let cached = cached_jobs.load(Ordering::Relaxed);
                on_job_progress(done, total_jobs, cached, &job.chunk_name);
                outcome
            })
            .collect()
    });

    let mut trans_ok_flags = vec![false; chunks.len()];
    let mut extra_col_ok: Vec<Vec<bool>> = chunks
        .iter()
        .map(|ch| vec![false; ch.extra_collision_models.len()])
        .collect();

    for res in results {
        match res? {
            QcOutcome::Main {
                chunk_idx,
                used_fallback,
            } => {
                chunks[chunk_idx].fallback_vmf_brushes = used_fallback;
            }
            QcOutcome::Translucent { chunk_idx, ok } => {
                trans_ok_flags[chunk_idx] = ok;
            }
            QcOutcome::ExtraCol {
                chunk_idx,
                col_idx,
                ok,
            } => {
                if let Some(slot) = extra_col_ok
                    .get_mut(chunk_idx)
                    .and_then(|v| v.get_mut(col_idx))
                {
                    *slot = ok;
                }
            }
        }
    }

    for (chunk_idx, ch) in chunks.iter_mut().enumerate() {
        if !trans_ok_flags[chunk_idx] {
            ch.translucent_model_rel_path = None;
            ch.translucent_qc_path = None;
        }
        if !ch.extra_collision_models.is_empty() {
            let prev_models = std::mem::take(&mut ch.extra_collision_models);
            let prev_boxes = std::mem::take(&mut ch.extra_collision_boxes);
            let mut kept_models = Vec::with_capacity(prev_models.len());
            for (idx, (col_qc, col_mdl)) in prev_models.into_iter().enumerate() {
                if extra_col_ok[chunk_idx].get(idx).copied().unwrap_or(false) {
                    kept_models.push((col_qc, col_mdl));
                } else if let Some(fb) = prev_boxes.get(idx) {
                    ch.overflow_boxes.extend(fb.iter().cloned());
                }
            }
            ch.extra_collision_models = kept_models;
        }
    }

    Ok(chunks.len())
}

fn run_studiomdl(studiomdl: &Path, game_dir: &Path, qc_path: &Path) -> (bool, String) {
    run_studiomdl_with_verbosity(studiomdl, game_dir, qc_path, false)
}

fn run_studiomdl_verbose(studiomdl: &Path, game_dir: &Path, qc_path: &Path) -> (bool, String) {
    run_studiomdl_with_verbosity(studiomdl, game_dir, qc_path, true)
}

fn run_studiomdl_with_verbosity(
    studiomdl: &Path,
    game_dir: &Path,
    qc_path: &Path,
    verbose: bool,
) -> (bool, String) {
    let mut cmd = Command::new(studiomdl);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let is_pp = studiomdl
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.contains("++"))
        .unwrap_or(false);
    cmd.args(["-fullcollide", "-nop4"]);
    if is_pp {
        cmd.arg("-nodx80");
    }
    if verbose {
        cmd.arg("-verbose");
    }
    cmd.arg("-game").arg(game_dir);
    if let (Some(p), Some(fname)) = (qc_path.parent(), qc_path.file_name()) {
        if !p.as_os_str().is_empty() {
            cmd.current_dir(p);
            cmd.arg(fname);
        } else {
            cmd.arg(qc_path);
        }
    } else {
        cmd.arg(qc_path);
    }
    match cmd.output() {
        Ok(out) => {
            let stdout_s = String::from_utf8_lossy(&out.stdout);
            let stderr_s = String::from_utf8_lossy(&out.stderr);
            let combined = if stderr_s.trim().is_empty() {
                stdout_s.into_owned()
            } else {
                format!("{}\n{}", stdout_s, stderr_s)
            };
            let lower = combined.to_ascii_lowercase();
            let has_fatal = combined.contains("ERROR:")
                || lower.contains("aborted processing")
                || !out.status.success();
            (!has_fatal, combined)
        }
        Err(e) => (false, format!("Failed spawning studiomdl.exe: {}", e)),
    }
}

pub fn strip_collision_from_qc(qc_path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(qc_path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if trimmed.starts_with("$collisionmodel") {
            let mut depth =
                lines[i].matches('{').count() as isize - lines[i].matches('}').count() as isize;
            if depth > 0 {
                i += 1;
                while i < lines.len() && depth > 0 {
                    depth += lines[i].matches('{').count() as isize;
                    depth -= lines[i].matches('}').count() as isize;
                    i += 1;
                }
                continue;
            }
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            if j < lines.len() && lines[j].trim_start().starts_with('{') {
                depth =
                    lines[j].matches('{').count() as isize - lines[j].matches('}').count() as isize;
                i = j + 1;
                while i < lines.len() && depth > 0 {
                    depth += lines[i].matches('{').count() as isize;
                    depth -= lines[i].matches('}').count() as isize;
                    i += 1;
                }
                continue;
            }
            // Single-line $collisionmodel "..." without a { ... } block
            i += 1;
            continue;
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    std::fs::write(qc_path, out).map_err(|e| e.to_string())
}

const MODEL_SIDECAR_EXTS: [&str; 7] = [
    ".mdl",
    ".vvd",
    ".dx80.vtx",
    ".dx90.vtx",
    ".sw.vtx",
    ".vtx",
    ".phy",
];

fn strip_mdl_ext(model_rel_path: &str) -> &str {
    let bytes = model_rel_path.as_bytes();
    if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".mdl") {
        &model_rel_path[..bytes.len() - 4]
    } else {
        model_rel_path
    }
}

fn model_sidecar_paths(base_dir: &Path, model_rel_path: &str) -> Vec<PathBuf> {
    let rel_no_ext = strip_mdl_ext(model_rel_path);
    MODEL_SIDECAR_EXTS
        .iter()
        .map(|ext| base_dir.join(format!("{}{}", rel_no_ext, ext)))
        .collect()
}

fn copy_compiled_model_files(
    game_dir: &Path,
    addon_dir: &Path,
    model_rel_path: &str,
    keep_in_game_dir: bool,
) -> Result<(), String> {
    let rel_no_ext = strip_mdl_ext(model_rel_path);
    let src_mdl = game_dir.join(format!("{}.mdl", rel_no_ext));
    if !src_mdl.is_file() || src_mdl.metadata().map(|m| m.len() == 0).unwrap_or(true) {
        return Err(format!(
            "StudioMDL did not produce expected .mdl: {}",
            src_mdl.display()
        ));
    }

    for ext in MODEL_SIDECAR_EXTS {
        let rel = format!("{}{}", rel_no_ext, ext);
        let src = game_dir.join(&rel);
        if src.is_file() {
            let dst = addon_dir.join(&rel);
            if let Some(p) = dst.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
            if !keep_in_game_dir {
                let _ = std::fs::remove_file(&src);
            }
        }
    }
    Ok(())
}

fn fnv1a_update_file(hash: &mut u64, path: &Path) -> std::io::Result<()> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            *hash ^= b as u64;
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(())
}

fn compute_file_fnv1a_hex(path: &Path) -> Option<String> {
    let mut hash: u64 = 0xcbf29ce484222325;
    fnv1a_update_file(&mut hash, path).ok()?;
    Some(format!("{:016x}", hash))
}

pub fn compute_qc_cache_hash(qc_path: &Path) -> Option<String> {
    let qc_bytes = std::fs::read(qc_path).ok()?;
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in &qc_bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let qc_str = String::from_utf8_lossy(&qc_bytes);
    if let Some(qc_dir) = qc_path.parent() {
        for line in qc_str.lines() {
            let trimmed = line.trim_start();
            let directive = trimmed.split_whitespace().next().unwrap_or("");
            let smd_rel_opt = match directive {
                "$collisionmodel" => {
                    let rest = &trimmed[directive.len()..];
                    rest.split('"').nth(1)
                }
                "$body" | "$model" => {
                    let rest = &trimmed[directive.len()..];
                    let quoted: Vec<&str> = rest
                        .split('"')
                        .enumerate()
                        .filter_map(|(i, s)| (i % 2 == 1).then_some(s))
                        .collect();
                    quoted
                        .iter()
                        .copied()
                        .find(|s| s.to_ascii_lowercase().ends_with(".smd"))
                        .or_else(|| quoted.last().copied())
                }
                _ => None,
            };
            if let Some(smd_rel) = smd_rel_opt {
                let smd_path = qc_dir.join(smd_rel);
                if smd_path.is_file() {
                    hash ^= 0xff;
                    hash = hash.wrapping_mul(0x100000001b3);
                    fnv1a_update_file(&mut hash, &smd_path).ok()?;
                } else {
                    hash ^= 0xfe;
                    hash = hash.wrapping_mul(0x100000001b3);
                    for &b in smd_rel.as_bytes() {
                        hash ^= b as u64;
                        hash = hash.wrapping_mul(0x100000001b3);
                    }
                }
            }
        }
    }
    Some(format!("{:016x}", hash))
}

pub fn qc_has_collision_model(qc_path: &Path) -> bool {
    std::fs::read_to_string(qc_path)
        .map(|s| {
            s.lines()
                .any(|l| l.trim_start().starts_with("$collisionmodel"))
        })
        .unwrap_or(false)
}

pub fn qc_cache_signature_matches(qc_path: &Path, addon_mdl: &Path) -> bool {
    let Some(expected) = compute_qc_cache_hash(qc_path) else {
        return false;
    };
    let sig_path = addon_mdl.with_extension("mcgm_hash");
    std::fs::read_to_string(sig_path)
        .map(|s| s.trim() == expected)
        .unwrap_or(false)
}

pub fn write_qc_cache_signature(qc_path: &Path, addon_mdl: &Path) {
    if let Some(hash) = compute_qc_cache_hash(qc_path) {
        let _ = std::fs::write(addon_mdl.with_extension("mcgm_hash"), hash);
    }
}

pub fn stage_water_vmts_to_game_dir(
    addon_dir: &Path,
    game_dir: &Path,
    category: &str,
) -> Result<Vec<PathBuf>, String> {
    let mut staged = Vec::new();
    if game_dir.as_os_str().is_empty() {
        return Ok(staged);
    }
    let clean_cat = category.trim_matches(|c| c == '/' || c == '\\');
    let src_dir = addon_dir.join("materials").join(clean_cat);
    if !src_dir.is_dir() {
        return Ok(staged);
    }
    let dst_dir = game_dir.join("materials").join(clean_cat);
    std::fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    for fname in ["mc_water.vmt", "mc_water_beneath.vmt"] {
        let src = src_dir.join(fname);
        let dst = dst_dir.join(fname);
        if src.is_file() && !dst.exists() {
            std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
            staged.push(dst);
        }
    }
    Ok(staged)
}

pub fn stage_collision_vmts_to_game_dir(
    addon_dir: &Path,
    game_dir: &Path,
    category: &str,
) -> Result<Vec<PathBuf>, String> {
    let mut staged = Vec::new();
    if game_dir.as_os_str().is_empty() {
        return Ok(staged);
    }
    let clean_cat = category.trim_matches(|c| c == '/' || c == '\\');
    let src_dir = addon_dir.join("materials").join(clean_cat);
    if !src_dir.is_dir() {
        return Ok(staged);
    }
    let dst_dir = game_dir.join("materials").join(clean_cat);
    std::fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let entries = std::fs::read_dir(&src_dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if fname.starts_with("col_") && fname.ends_with(".vmt") {
            let dst = dst_dir.join(fname);
            if !dst.exists() {
                std::fs::copy(&path, &dst).map_err(|e| e.to_string())?;
                staged.push(dst);
            }
        }
    }
    Ok(staged)
}

pub fn cleanup_staged_files(paths: &[PathBuf]) {
    for p in paths {
        if p.is_file() {
            let _ = std::fs::remove_file(p);
        }
    }
}

static STAGED_BACKUP_SEQ: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

enum StagedModelEntry {
    Restore {
        game_path: PathBuf,
        backup_path: PathBuf,
    },
    RemoveIfCreated(PathBuf),
}

/// Backs up any pre-existing model sidecars in `game_dir` before staging and
/// restores them (while removing files created by this run) on `Drop`.
pub struct StagedModelGuard {
    entries: Vec<StagedModelEntry>,
    backup_dir: Option<PathBuf>,
}

impl StagedModelGuard {
    pub fn prepare(
        game_dir: &Path,
        model_rel_paths: &[String],
        backup_root: &Path,
    ) -> Result<Self, String> {
        if game_dir.as_os_str().is_empty() || model_rel_paths.is_empty() {
            return Ok(Self {
                entries: Vec::new(),
                backup_dir: None,
            });
        }
        let mut seen = HashSet::new();
        let mut candidate_paths = Vec::new();
        for rel in model_rel_paths {
            if rel.is_empty() {
                continue;
            }
            for p in model_sidecar_paths(game_dir, rel) {
                let key = p.to_string_lossy().to_ascii_lowercase();
                if seen.insert(key) {
                    candidate_paths.push(p);
                }
            }
        }

        let mut entries = Vec::with_capacity(candidate_paths.len());
        let mut backup_dir: Option<PathBuf> = None;
        for (idx, game_path) in candidate_paths.into_iter().enumerate() {
            if game_path.is_file() {
                let bdir = match backup_dir {
                    Some(ref d) => d.clone(),
                    None => {
                        let seq = STAGED_BACKUP_SEQ.fetch_add(1, Ordering::Relaxed);
                        let d = backup_root.join(format!(
                            "staged_models_backup_{}_{}",
                            std::process::id(),
                            seq
                        ));
                        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
                        backup_dir = Some(d.clone());
                        d
                    }
                };
                let backup_path = bdir.join(format!("{}.bak", idx));
                std::fs::copy(&game_path, &backup_path).map_err(|e| {
                    format!(
                        "Failed backing up pre-existing game file {}: {}",
                        game_path.display(),
                        e
                    )
                })?;
                entries.push(StagedModelEntry::Restore {
                    game_path,
                    backup_path,
                });
            } else {
                entries.push(StagedModelEntry::RemoveIfCreated(game_path));
            }
        }

        Ok(Self {
            entries,
            backup_dir,
        })
    }
}

impl Drop for StagedModelGuard {
    fn drop(&mut self) {
        for entry in &self.entries {
            match entry {
                StagedModelEntry::Restore {
                    game_path,
                    backup_path,
                } => {
                    if backup_path.is_file() {
                        if let Some(p) = game_path.parent() {
                            let _ = std::fs::create_dir_all(p);
                        }
                        if std::fs::copy(backup_path, game_path).is_ok() {
                            let _ = std::fs::remove_file(backup_path);
                        }
                    }
                }
                StagedModelEntry::RemoveIfCreated(game_path) => {
                    if game_path.is_file() {
                        let _ = std::fs::remove_file(game_path);
                    }
                }
            }
        }
        if let Some(ref bdir) = self.backup_dir {
            let _ = std::fs::remove_dir_all(bdir);
        }
    }
}

fn sync_cached_model_files(
    addon_dir: &Path,
    game_dir: &Path,
    model_rel_path: &str,
) -> Result<(), String> {
    if game_dir.as_os_str().is_empty() || model_rel_path.is_empty() {
        return Ok(());
    }
    let rel_no_ext = strip_mdl_ext(model_rel_path);
    for ext in MODEL_SIDECAR_EXTS {
        let rel = format!("{}{}", rel_no_ext, ext);
        let src = addon_dir.join(&rel);
        let dst = game_dir.join(&rel);
        if src.is_file() {
            if let Some(p) = dst.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
        } else if dst.is_file() {
            let _ = std::fs::remove_file(&dst);
        }
    }
    Ok(())
}

pub fn stage_cached_models_to_game_dir(
    addon_dir: &Path,
    game_dir: &Path,
    chunks: &[CompiledChunkArtifact],
) -> Result<(), String> {
    if game_dir.as_os_str().is_empty() {
        return Ok(());
    }
    let mut rel_paths: Vec<&str> = Vec::new();
    for ch in chunks {
        if !ch.model_rel_path.is_empty() {
            rel_paths.push(ch.model_rel_path.as_str());
        }
        if let Some(ref tm) = ch.translucent_model_rel_path {
            if !tm.is_empty() {
                rel_paths.push(tm.as_str());
            }
        }
        for (_, col_mdl) in &ch.extra_collision_models {
            if !col_mdl.is_empty() {
                rel_paths.push(col_mdl.as_str());
            }
        }
    }
    for model_rel_path in rel_paths {
        sync_cached_model_files(addon_dir, game_dir, model_rel_path)?;
    }
    Ok(())
}

struct StreamedOutput {
    code: Option<i32>,
    success: bool,
    stdout: String,
    stderr: String,
}

fn run_streaming_tool<F: FnMut(&str)>(
    mut cmd: Command,
    tag: &str,
    cancel_flag: Option<&AtomicBool>,
    log_cb: &mut F,
) -> Result<StreamedOutput, String> {
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed starting {}: {}", tag, e))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (tx, rx) = mpsc::channel::<(bool, String)>();

    let tx_out = tx.clone();
    let h_out = std::thread::spawn(move || {
        if let Some(out) = stdout_pipe {
            let mut reader = BufReader::new(out);
            let mut buf = Vec::new();
            while let Ok(n) = reader.read_until(b'\n', &mut buf) {
                if n == 0 {
                    break;
                }
                let line = String::from_utf8_lossy(&buf)
                    .trim_end_matches(&['\r', '\n'][..])
                    .to_string();
                let _ = tx_out.send((false, line));
                buf.clear();
            }
        }
    });

    let tx_err = tx.clone();
    let h_err = std::thread::spawn(move || {
        if let Some(err) = stderr_pipe {
            let mut reader = BufReader::new(err);
            let mut buf = Vec::new();
            while let Ok(n) = reader.read_until(b'\n', &mut buf) {
                if n == 0 {
                    break;
                }
                let line = String::from_utf8_lossy(&buf)
                    .trim_end_matches(&['\r', '\n'][..])
                    .to_string();
                let _ = tx_err.send((true, line));
                buf.clear();
            }
        }
    });
    drop(tx);

    let mut stdout_lines = Vec::new();
    let mut stderr_lines = Vec::new();
    loop {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = h_out.join();
                let _ = h_err.join();
                return Err("Export cancelled by user.".to_string());
            }
        }
        match rx.recv_timeout(std::time::Duration::from_millis(150)) {
            Ok((is_err, line)) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    if is_err {
                        stderr_lines.push(line.clone());
                        log_cb(&format!("[{}][err] {}", tag, trimmed));
                    } else {
                        stdout_lines.push(line.clone());
                        log_cb(&format!("[{}] {}", tag, trimmed));
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = h_out.join();
    let _ = h_err.join();
    let status = child
        .wait()
        .map_err(|e| format!("Failed waiting for {}: {}", tag, e))?;

    Ok(StreamedOutput {
        code: status.code(),
        success: status.success(),
        stdout: stdout_lines.join("\n"),
        stderr: stderr_lines.join("\n"),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn vrad_ambient_rgb_for_style(lighting_style: &str) -> Option<[&'static str; 3]> {
    match lighting_style.trim().to_ascii_lowercase().as_str() {
        "gmod" => None,
        "night" => Some(["0.015", "0.02", "0.035"]),
        "bright" => Some(["0.07", "0.075", "0.085"]),
        _ => Some(["0.04", "0.05", "0.06"]),
    }
}

pub fn build_vrad_args(
    is_vrad_pp: bool,
    rad_mode: &str,
    hq_light: bool,
    fast_vrad: bool,
    force_cpu: bool,
    lighting_style: &str,
) -> Vec<String> {
    let mut args = Vec::with_capacity(20);
    if is_vrad_pp {
        let ao_samples = if fast_vrad {
            "12"
        } else if hq_light {
            "24"
        } else {
            "16"
        };
        // Omit -propambient: in VRAD++ it triggers CPU-only ComputeCollisionPropLighting
        // over all solid=6 VPhysics collision props, stalling for minutes per pass instead
        // of running ComputeLeafAmbientGPU directly on the GPU BVH (~0.1s).
        for flag in [
            rad_mode,
            "-StaticPropLighting",
            "-StaticPropPolys",
            "-textureshadows",
            "-forcetextureshadows",
            "-ao",
            "-aoradius",
            "30",
            "-aoscale",
            "0.25",
            "-aopropsamples",
            ao_samples,
        ] {
            args.push(flag.to_string());
        }
        if let Some([ar, ag, ab]) = vrad_ambient_rgb_for_style(lighting_style) {
            args.push("-ambient".to_string());
            args.push(ar.to_string());
            args.push(ag.to_string());
            args.push(ab.to_string());
        }
        if fast_vrad || force_cpu {
            args.push("-fastambient".to_string());
        }
    } else if hq_light {
        for flag in [
            rad_mode,
            "-StaticPropLighting",
            "-StaticPropPolys",
            "-TextureShadows",
        ] {
            args.push(flag.to_string());
        }
    } else {
        args.push(rad_mode.to_string());
        args.push("-StaticPropLighting".to_string());
    }
    if fast_vrad {
        args.push("-fast".to_string());
    }
    if force_cpu {
        args.push("-cpu".to_string());
    }
    args
}

pub const VVIS_AUTO_FAST_MAX_CLUSTERS: usize = 750;
pub const VVIS_AUTO_FAST_MAX_PORTALS: usize = 2000;
pub const VVIS_PP_AUTO_FAST_MAX_CLUSTERS: usize = 1500;
pub const VVIS_PP_AUTO_FAST_MAX_PORTALS: usize = 5000;

pub fn parse_prt_portal_counts(prt_text: &str) -> Option<(usize, usize)> {
    let mut lines = prt_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty());
    let header = lines.next()?;
    if !header.to_ascii_uppercase().starts_with("PRT") {
        return None;
    }
    let clusters = lines.next()?.parse::<usize>().ok()?;
    let portals = lines.next()?.parse::<usize>().ok()?;
    Some((clusters, portals))
}

fn read_prt_portal_counts(prt_path: &Path) -> Option<(usize, usize)> {
    use std::io::Read;
    let file = std::fs::File::open(prt_path).ok()?;
    let mut buf = Vec::with_capacity(256);
    file.take(256).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    parse_prt_portal_counts(&text)
}

pub fn should_use_fast_vvis(fast_vrad: bool, prt_counts: Option<(usize, usize)>) -> bool {
    fast_vrad
        || matches!(
            prt_counts,
            Some((clusters, portals))
                if clusters > VVIS_AUTO_FAST_MAX_CLUSTERS
                    || portals > VVIS_AUTO_FAST_MAX_PORTALS
        )
}

pub fn should_use_fast_vvis_for_tool(
    fast_vrad: bool,
    prt_counts: Option<(usize, usize)>,
    is_vvis_pp: bool,
) -> bool {
    if !is_vvis_pp {
        return should_use_fast_vvis(fast_vrad, prt_counts);
    }
    fast_vrad
        || matches!(
            prt_counts,
            Some((clusters, portals))
                if clusters > VVIS_PP_AUTO_FAST_MAX_CLUSTERS
                    || portals > VVIS_PP_AUTO_FAST_MAX_PORTALS
        )
}

pub fn format_command_debug(exe: &Path, args: &[String]) -> String {
    let quote_if_needed = |s: &str| -> String {
        if s.is_empty() || s.chars().any(|c| c.is_ascii_whitespace()) {
            format!("\"{}\"", s)
        } else {
            s.to_string()
        }
    };
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_if_needed(&exe.display().to_string()));
    for arg in args {
        parts.push(quote_if_needed(arg));
    }
    parts.join(" ")
}

#[allow(clippy::too_many_arguments)]
pub fn compile_bsp_map_cancellable<F: FnMut(&str)>(
    tools: &ToolPaths,
    vmf_path: &Path,
    map_name: &str,
    addon_dir: &Path,
    import_to_gmod: bool,
    fast_vrad: bool,
    hq_light: bool,
    lighting_style: &str,
    cancel_flag: Option<&AtomicBool>,
    mut log_cb: F,
) -> Result<PathBuf, String> {
    if !tools.vbsp.is_file() {
        return Err(format!("vbsp.exe not found: {}", tools.vbsp.display()));
    }
    if !tools.game_dir.is_dir() {
        return Err(format!(
            "Garry's Mod game_dir not found: {}",
            tools.game_dir.display()
        ));
    }
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .to_string();

    let vmf_dir = vmf_path.parent().filter(|p| !p.as_os_str().is_empty());

    let is_pp_exe = |p: &Path| -> bool {
        p.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.contains("++"))
            .unwrap_or(false)
    };
    let is_vbsp_pp = is_pp_exe(&tools.vbsp);
    let is_vvis_pp = is_pp_exe(&tools.vvis);
    let is_vrad_pp = is_pp_exe(&tools.vrad);

    // 1. VBSP (-notjunc, plus -cullverts -blocksize 1024 on VBSP++)
    let vbsp_tag = if is_vbsp_pp { "VBSP++" } else { "VBSP" };
    log_cb(&format!("Running {} on {}...", vbsp_tag, vmf_path.display()));
    let mut vbsp_args: Vec<String> = vec!["-notjunc".to_string()];
    if is_vbsp_pp {
        vbsp_args.push("-cullverts".to_string());
        vbsp_args.push("-blocksize".to_string());
        vbsp_args.push("1024".to_string());
    }
    vbsp_args.push("-threads".to_string());
    vbsp_args.push(threads.clone());
    vbsp_args.push("-game".to_string());
    vbsp_args.push(tools.game_dir.display().to_string());
    let vbsp_target = match (vmf_dir, vmf_path.file_name()) {
        (Some(_), Some(fname)) => fname.to_string_lossy().into_owned(),
        _ => vmf_path.display().to_string(),
    };
    vbsp_args.push(vbsp_target);
    log_cb(&format!(
        "[DEBUG] {} cmd: {}",
        vbsp_tag,
        format_command_debug(&tools.vbsp, &vbsp_args)
    ));
    let mut vbsp = Command::new(&tools.vbsp);
    vbsp.args(&vbsp_args);
    if let Some(dir) = vmf_dir {
        vbsp.current_dir(dir);
    }
    let t_vbsp = std::time::Instant::now();
    let out_vbsp = run_streaming_tool(vbsp, vbsp_tag, cancel_flag, &mut log_cb)?;
    log_cb(&format!(
        "[DEBUG] {} finished in {:.2}s (code={:?})",
        vbsp_tag,
        t_vbsp.elapsed().as_secs_f64(),
        out_vbsp.code
    ));
    let fmt_exit_code =
        |c: Option<i32>| c.map(|n| n.to_string()).unwrap_or_else(|| "unknown".to_string());

    if !out_vbsp.success {
        return Err(format!(
            "{} failed (exit code {}):\nSTDOUT:\n{}\nSTDERR:\n{}",
            vbsp_tag,
            fmt_exit_code(out_vbsp.code),
            out_vbsp.stdout,
            out_vbsp.stderr
        ));
    }

    let bsp_temp = vmf_path.with_extension("bsp");
    if !bsp_temp.is_file() {
        return Err(format!("VBSP did not produce BSP at {}", bsp_temp.display()));
    }

    // 2. VVIS (respects the pipeline's fast flag, and auto-enables -fast when VBSP's .prt
    // exceeds complexity thresholds where full PortalFlow would stall for hours)
    if tools.vvis.is_file() {
        let vvis_tag = if is_vvis_pp { "VVIS++" } else { "VVIS" };
        let prt_counts = read_prt_portal_counts(&vmf_path.with_extension("prt"));
        let use_fast_vvis = should_use_fast_vvis_for_tool(fast_vrad, prt_counts, is_vvis_pp);
        let vvis_mode_label = if use_fast_vvis && !fast_vrad {
            "auto -fast"
        } else if use_fast_vvis {
            "-fast"
        } else {
            "full PortalFlow"
        };
        if let Some((clusters, portals)) = prt_counts {
            log_cb(&format!(
                "[{}] .prt stats: {} clusters, {} portals ({})",
                vvis_tag, clusters, portals, vvis_mode_label
            ));
        }
        if use_fast_vvis && !fast_vrad {
            if let Some((clusters, portals)) = prt_counts {
                log_cb(&format!(
                    "[{}] High portal complexity ({} clusters, {} portals): auto-enabling -fast for VVIS (VRAD lighting quality unchanged).",
                    vvis_tag,
                    clusters,
                    portals
                ));
            }
        }
        let fast_tag = if use_fast_vvis { " (-fast)" } else { "" };
        let vvis_name = format!("{}{}", vvis_tag, fast_tag);
        log_cb(&format!("Running {} visibility pass...", vvis_name));
        let mut vvis_args: Vec<String> = Vec::new();
        if use_fast_vvis {
            vvis_args.push("-fast".to_string());
        }
        vvis_args.push("-threads".to_string());
        vvis_args.push(threads.clone());
        vvis_args.push("-game".to_string());
        vvis_args.push(tools.game_dir.display().to_string());
        let vvis_target = match (vmf_dir, bsp_temp.file_name()) {
            (Some(_), Some(fname)) => fname.to_string_lossy().into_owned(),
            _ => bsp_temp.display().to_string(),
        };
        vvis_args.push(vvis_target);
        log_cb(&format!(
            "[DEBUG] {} cmd: {}",
            vvis_tag,
            format_command_debug(&tools.vvis, &vvis_args)
        ));
        let mut vvis = Command::new(&tools.vvis);
        vvis.args(&vvis_args);
        if let Some(dir) = vmf_dir {
            vvis.current_dir(dir);
        }
        let t_vvis = std::time::Instant::now();
        let out_vvis = run_streaming_tool(vvis, vvis_tag, cancel_flag, &mut log_cb)?;
        log_cb(&format!(
            "[DEBUG] {} finished in {:.2}s (code={:?})",
            vvis_tag,
            t_vvis.elapsed().as_secs_f64(),
            out_vvis.code
        ));
        if !out_vvis.success {
            return Err(format!(
                "{} failed (exit code {}):\nSTDOUT:\n{}\nSTDERR:\n{}",
                vvis_tag,
                fmt_exit_code(out_vvis.code),
                out_vvis.stdout,
                out_vvis.stderr
            ));
        }
    }

    // 3. VRAD (VRAD++ uses GPU raytracing by default; auto-fallback to -cpu on failure)
    if tools.vrad.is_file() {
        let mode_desc = if hq_light {
            "HDR+LDR StaticPropPolys TextureShadows"
        } else if fast_vrad {
            "LDR StaticPropLighting -fast"
        } else {
            "LDR StaticPropLighting"
        };
        if is_vrad_pp {
            log_cb(&format!(
                "Running VRAD++ (GPU raytracing, {}, {} threads)...",
                mode_desc, threads
            ));
        } else {
            log_cb(&format!("Running VRAD ({}, {} threads)...", mode_desc, threads));
        }
        let build_vrad_cmd = |rad_mode: &str, force_cpu: bool| -> (Command, Vec<String>) {
            let mut vrad_args = build_vrad_args(
                is_vrad_pp,
                rad_mode,
                hq_light,
                fast_vrad,
                force_cpu,
                lighting_style,
            );
            vrad_args.push("-threads".to_string());
            vrad_args.push(threads.clone());
            vrad_args.push("-game".to_string());
            vrad_args.push(tools.game_dir.display().to_string());
            let vrad_target = match (vmf_dir, bsp_temp.file_name()) {
                (Some(_), Some(fname)) => fname.to_string_lossy().into_owned(),
                _ => bsp_temp.display().to_string(),
            };
            vrad_args.push(vrad_target);
            let mut vrad = Command::new(&tools.vrad);
            vrad.args(&vrad_args);
            if let Some(dir) = vmf_dir {
                vrad.current_dir(dir);
            }
            (vrad, vrad_args)
        };

        let tag = if is_vrad_pp { "VRAD++" } else { "VRAD" };
        let mut run_vrad_pass = |rad_mode: &str| -> Result<(), String> {
            let (cmd, args_list) = build_vrad_cmd(rad_mode, false);
            log_cb(&format!(
                "[DEBUG] {} ({}) cmd: {}",
                tag,
                rad_mode,
                format_command_debug(&tools.vrad, &args_list)
            ));
            let t_pass = std::time::Instant::now();
            let mut out_vrad = run_streaming_tool(cmd, tag, cancel_flag, &mut log_cb)?;
            log_cb(&format!(
                "[DEBUG] {} ({}) finished in {:.2}s (code={:?})",
                tag,
                rad_mode,
                t_pass.elapsed().as_secs_f64(),
                out_vrad.code
            ));
            let mut used_cpu_fallback = false;
            if !out_vrad.success && is_vrad_pp {
                used_cpu_fallback = true;
                log_cb(&format!(
                    "[VRAD++] GPU pass ({}) exited with code {:?}, retrying with -cpu fallback...",
                    rad_mode, out_vrad.code
                ));
                let (cpu_cmd, cpu_args) = build_vrad_cmd(rad_mode, true);
                log_cb(&format!(
                    "[DEBUG] VRAD++ CPU ({}) cmd: {}",
                    rad_mode,
                    format_command_debug(&tools.vrad, &cpu_args)
                ));
                let t_cpu = std::time::Instant::now();
                out_vrad = run_streaming_tool(
                    cpu_cmd,
                    "VRAD++ CPU",
                    cancel_flag,
                    &mut log_cb,
                )?;
                log_cb(&format!(
                    "[DEBUG] VRAD++ CPU ({}) finished in {:.2}s (code={:?})",
                    rad_mode,
                    t_cpu.elapsed().as_secs_f64(),
                    out_vrad.code
                ));
            }
            if !out_vrad.success {
                let fail_stage = if used_cpu_fallback {
                    "VRAD++ (CPU fallback)"
                } else {
                    tag
                };
                return Err(format!(
                    "{} ({}) failed (exit code {}):\nSTDOUT:\n{}\nSTDERR:\n{}",
                    fail_stage,
                    rad_mode,
                    fmt_exit_code(out_vrad.code),
                    out_vrad.stdout,
                    out_vrad.stderr
                ));
            }
            Ok(())
        };

        if is_vrad_pp && hq_light {
            // VRAD++ -both skips LDR lightdata (0 bytes); run separate -ldr and -hdr passes.
            run_vrad_pass("-ldr")?;
            run_vrad_pass("-hdr")?;
        } else if hq_light {
            run_vrad_pass("-both")?;
        } else {
            run_vrad_pass("-ldr")?;
        }
    }

    // 4. Copy compiled BSP to addon and optionally game_dir/maps
    let addon_maps = addon_dir.join("maps");
    std::fs::create_dir_all(&addon_maps).map_err(|e| e.to_string())?;
    let addon_bsp = addon_maps.join(format!("{}.bsp", map_name));
    std::fs::copy(&bsp_temp, &addon_bsp).map_err(|e| e.to_string())?;

    if import_to_gmod {
        let game_maps = tools.game_dir.join("maps");
        std::fs::create_dir_all(&game_maps).map_err(|e| e.to_string())?;
        let game_bsp = game_maps.join(format!("{}.bsp", map_name));
        std::fs::copy(&bsp_temp, &game_bsp).map_err(|e| e.to_string())?;
        return Ok(game_bsp);
    }

    Ok(addon_bsp)
}

pub fn write_addon_metadata(addon_dir: &Path, map_name: &str) -> Result<(), String> {
    let title = if map_name.starts_with("MCGM - ") {
        map_name.to_string()
    } else {
        format!("MCGM - {}", map_name)
    };
    let clean_map = map_name.trim();
    let json = serde_json::json!({
        "title": title,
        "type": "map",
        "tags": ["fun", "roleplay"],
        "ignore": [
            "_work/*",
            "*.vmf",
            "*.smd",
            "*.qc",
            "*.log",
            "*.prt",
            "*.lin",
            "*.mcgm_hash",
            format!("maps/{}.png", clean_map)
        ]
    });
    std::fs::write(
        addon_dir.join("addon.json"),
        serde_json::to_string_pretty(&json).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())
}

fn collect_gma_files_recursive(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed reading {}: {}", dir.display(), e))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|e| format!("Failed stat {}: {}", path.display(), e))?;
        if meta.is_dir() {
            let is_work_dir = path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("_work"));
            if is_work_dir {
                continue;
            }
            collect_gma_files_recursive(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let lower = rel.to_ascii_lowercase();
            // Skip addon.json, working build artifacts, cache signatures, and top-level maps/*.png (keep maps/thumb/*.png).
            if lower == "addon.json"
                || lower.starts_with("_work/")
                || lower.ends_with(".vmf")
                || lower.ends_with(".gma")
                || lower.ends_with(".mcgm_hash")
                || lower.ends_with(".smd")
                || lower.ends_with(".qc")
                || lower.ends_with(".log")
                || lower.ends_with(".prt")
                || lower.ends_with(".lin")
                || lower.ends_with(".vmx")
                || lower.ends_with(".tmp")
            {
                continue;
            }
            if lower.starts_with("maps/")
                && !lower.starts_with("maps/thumb/")
                && lower.ends_with(".png")
            {
                continue;
            }
            out.push((lower, path));
        }
    }
    Ok(())
}

static TEMP_GMA_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct TempFileGuard(Option<PathBuf>);

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(ref p) = self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn make_temp_gma_path(out_gma: &Path, tag: &str) -> PathBuf {
    let parent = out_gma
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = out_gma
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("addon");
    let seq = TEMP_GMA_SEQ.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{}.{}.{}.{}.tmp.gma",
        stem,
        tag,
        std::process::id(),
        seq
    ))
}

fn pack_gma_v3_native_impl<F: FnMut(&Path) -> Result<(), String>>(
    addon_dir: &Path,
    out_gma: &Path,
    title: &str,
    description_json: &str,
    mut after_file_list_hook: F,
) -> Result<(), String> {
    use std::io::{Read, Write};

    if let Some(parent) = out_gma.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {}", e))?;
    }

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect_gma_files_recursive(addon_dir, addon_dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let tmp_gma = make_temp_gma_path(out_gma, "native");
    let mut guard = TempFileGuard::new(tmp_gma.clone());

    {
        let mut out = std::io::BufWriter::with_capacity(
            128 * 1024,
            std::fs::File::create(&tmp_gma)
                .map_err(|e| format!("Failed creating {}: {}", tmp_gma.display(), e))?,
        );
        let mut header_crc = flate2::Crc::new();

        let write_tracked = |w: &mut std::io::BufWriter<std::fs::File>,
                             crc: &mut flate2::Crc,
                             bytes: &[u8]|
         -> Result<(), String> {
            crc.update(bytes);
            w.write_all(bytes).map_err(|e| e.to_string())
        };

        write_tracked(&mut out, &mut header_crc, b"GMAD")?;
        write_tracked(&mut out, &mut header_crc, &[3u8])?;
        write_tracked(&mut out, &mut header_crc, &0u64.to_le_bytes())?; // SteamID
        write_tracked(&mut out, &mut header_crc, &timestamp.to_le_bytes())?;
        write_tracked(&mut out, &mut header_crc, &[0u8])?; // Required content (empty)
        write_tracked(&mut out, &mut header_crc, title.as_bytes())?;
        write_tracked(&mut out, &mut header_crc, &[0u8])?;
        write_tracked(&mut out, &mut header_crc, description_json.as_bytes())?;
        write_tracked(&mut out, &mut header_crc, &[0u8])?;
        write_tracked(&mut out, &mut header_crc, b"MCGM")?; // Author
        write_tracked(&mut out, &mut header_crc, &[0u8])?;
        write_tracked(&mut out, &mut header_crc, &1i32.to_le_bytes())?; // Addon version

        let mut buf = vec![0u8; 65536];
        let mut file_sizes: Vec<u64> = Vec::with_capacity(files.len());
        for (idx, (rel, path)) in files.iter().enumerate() {
            let file_num = (idx + 1) as u32;
            let mut f = std::fs::File::open(path)
                .map_err(|e| format!("Failed reading {}: {}", path.display(), e))?;
            let mut file_crc = flate2::Crc::new();
            let mut actual_size: u64 = 0;
            loop {
                let n = f
                    .read(&mut buf)
                    .map_err(|e| format!("Failed reading {}: {}", path.display(), e))?;
                if n == 0 {
                    break;
                }
                file_crc.update(&buf[..n]);
                actual_size += n as u64;
            }
            file_sizes.push(actual_size);

            write_tracked(&mut out, &mut header_crc, &file_num.to_le_bytes())?;
            write_tracked(&mut out, &mut header_crc, rel.as_bytes())?;
            write_tracked(&mut out, &mut header_crc, &[0u8])?;
            write_tracked(&mut out, &mut header_crc, &(actual_size as i64).to_le_bytes())?;
            write_tracked(&mut out, &mut header_crc, &file_crc.sum().to_le_bytes())?;
        }
        write_tracked(&mut out, &mut header_crc, &0u32.to_le_bytes())?; // End of file list
        out.flush().map_err(|e| e.to_string())?;
        after_file_list_hook(&tmp_gma)?;

        for ((_, path), expected_size) in files.iter().zip(file_sizes.iter()) {
            let mut f = std::fs::File::open(path)
                .map_err(|e| format!("Failed reading {}: {}", path.display(), e))?;
            let mut written: u64 = 0;
            loop {
                let n = f
                    .read(&mut buf)
                    .map_err(|e| format!("Failed reading {}: {}", path.display(), e))?;
                if n == 0 {
                    break;
                }
                write_tracked(&mut out, &mut header_crc, &buf[..n])?;
                written += n as u64;
            }
            if written != *expected_size {
                return Err(format!(
                    "File size changed while packing GMA: {} ({} != {})",
                    path.display(),
                    written,
                    expected_size
                ));
            }
        }

        let final_crc = header_crc.sum();
        out.write_all(&final_crc.to_le_bytes())
            .map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        let file = out.into_inner().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }

    std::fs::rename(&tmp_gma, out_gma)
        .map_err(|e| format!("Failed replacing {}: {}", out_gma.display(), e))?;
    guard.disarm();
    Ok(())
}

pub fn pack_gma_v3_native(
    addon_dir: &Path,
    out_gma: &Path,
    title: &str,
    description_json: &str,
) -> Result<(), String> {
    pack_gma_v3_native_impl(addon_dir, out_gma, title, description_json, |_| Ok(()))
}

#[cfg(test)]
pub fn pack_gma_v3_native_with_hook<F: FnMut(&Path) -> Result<(), String>>(
    addon_dir: &Path,
    out_gma: &Path,
    title: &str,
    description_json: &str,
    hook: F,
) -> Result<(), String> {
    pack_gma_v3_native_impl(addon_dir, out_gma, title, description_json, hook)
}

pub fn create_gma_addon<F: FnMut(&str)>(
    tools: &ToolPaths,
    addon_dir: &Path,
    out_gma: &Path,
    map_name: &str,
    mut log_cb: F,
) -> Result<PathBuf, String> {
    if let Some(parent) = out_gma.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {}", e))?;
    }

    if let Some(gmad_exe) = tools.gmad.as_ref().filter(|p| p.is_file()) {
        let gmad_tmp = make_temp_gma_path(out_gma, "gmad");
        let mut gmad_guard = TempFileGuard::new(gmad_tmp.clone());
        log_cb(&format!("Packing GMA addon via gmad.exe: {}", out_gma.display()));
        let mut cmd = Command::new(gmad_exe);
        cmd.arg("create")
            .arg("-folder")
            .arg(addon_dir)
            .arg("-out")
            .arg(&gmad_tmp)
            .arg("-warninvalid");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        match cmd.output() {
            Ok(output)
                if output.status.success()
                    && gmad_tmp.is_file()
                    && gmad_tmp.metadata().map(|m| m.len() > 0).unwrap_or(false) =>
            {
                std::fs::rename(&gmad_tmp, out_gma)
                    .map_err(|e| format!("Failed replacing {}: {}", out_gma.display(), e))?;
                gmad_guard.disarm();
                return Ok(out_gma.to_path_buf());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                log_cb(&format!(
                    "[GMA] gmad.exe failed (code {:?}), falling back to native GMA v3 packer: {} {}",
                    output.status.code(),
                    stdout.trim(),
                    stderr.trim()
                ));
            }
            Err(e) => {
                log_cb(&format!(
                    "[GMA] Failed spawning gmad.exe ({}), falling back to native GMA v3 packer",
                    e
                ));
            }
        }
    } else {
        log_cb(&format!(
            "gmad.exe not found; packing GMA v3 addon natively: {}",
            out_gma.display()
        ));
    }

    let title = if map_name.starts_with("MCGM - ") {
        map_name.to_string()
    } else {
        format!("MCGM - {}", map_name)
    };
    let desc_json = serde_json::json!({
        "description": format!("Minecraft map exported by MCGM ({})", map_name),
        "type": "map",
        "tags": ["fun", "roleplay"]
    })
    .to_string();

    pack_gma_v3_native(addon_dir, out_gma, &title, &desc_json)?;
    Ok(out_gma.to_path_buf())
}

fn blend_px(img: &mut image::RgbaImage, x: u32, y: u32, col: [u8; 4]) {
    if x >= img.width() || y >= img.height() {
        return;
    }
    let a = col[3] as u32;
    if a == 255 {
        img.put_pixel(x, y, image::Rgba(col));
        return;
    }
    if a == 0 {
        return;
    }
    let dst = img.get_pixel(x, y).0;
    let inv = 255 - a;
    let r = ((col[0] as u32 * a + dst[0] as u32 * inv) / 255) as u8;
    let g = ((col[1] as u32 * a + dst[1] as u32 * inv) / 255) as u8;
    let b = ((col[2] as u32 * a + dst[2] as u32 * inv) / 255) as u8;
    img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
}

fn fill_rect_rgba(img: &mut image::RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, col: [u8; 4]) {
    let x_end = x1.min(img.width());
    let y_end = y1.min(img.height());
    for y in y0..y_end {
        for x in x0..x_end {
            blend_px(img, x, y, col);
        }
    }
}

/// Renders the Minecraft voxel sky/terrain/banner map icon matching Python's `generate_map_thumbnail`:
/// writes `maps/thumb/<map>.png` (512x512) and `maps/<map>.png` (256x256) to `addon_dir` and `game_dir`.
#[cfg(test)]
pub fn generate_map_thumbnail(
    map_name: &str,
    addon_dir: &Path,
    game_dir: Option<&Path>,
    sky_style: &str,
) -> Result<(), String> {
    let clean_map = map_name.trim();
    let is_night = sky_style.trim().eq_ignore_ascii_case("night");

    let mut img_512 = image::ImageBuffer::from_pixel(512, 512, image::Rgba([0, 0, 0, 255]));
    let horizon_y = 330u32;

    // Sky gradient
    for y in 0..horizon_y {
        let t = y as f32 / horizon_y as f32;
        let (r, g, b) = if is_night {
            (
                (5.0 * (1.0 - t) + 18.0 * t) as u8,
                (7.0 * (1.0 - t) + 28.0 * t) as u8,
                (18.0 * (1.0 - t) + 55.0 * t) as u8,
            )
        } else {
            (
                (120.0 * (1.0 - t) + 192.0 * t) as u8,
                (167.0 * (1.0 - t) + 216.0 * t) as u8,
                255u8,
            )
        };
        for x in 0..512u32 {
            img_512.put_pixel(x, y, image::Rgba([r, g, b, 255]));
        }
    }

    if is_night {
        for &(sx, sy) in &[
            (50, 40),
            (120, 90),
            (220, 50),
            (280, 110),
            (450, 45),
            (180, 160),
            (330, 80),
        ] {
            fill_rect_rgba(&mut img_512, sx, sy, sx + 3, sy + 3, [240, 245, 255, 255]);
        }
        fill_rect_rgba(&mut img_512, 360, 50, 420, 110, [240, 244, 252, 255]);
        fill_rect_rgba(&mut img_512, 368, 58, 412, 102, [255, 255, 255, 255]);
        fill_rect_rgba(&mut img_512, 376, 70, 390, 84, [170, 182, 200, 255]);
        fill_rect_rgba(&mut img_512, 30, 150, 200, 175, [35, 45, 70, 180]);
        fill_rect_rgba(&mut img_512, 250, 180, 470, 205, [35, 45, 70, 180]);
    } else {
        fill_rect_rgba(&mut img_512, 360, 50, 420, 110, [255, 213, 74, 255]);
        fill_rect_rgba(&mut img_512, 368, 58, 412, 102, [255, 255, 170, 255]);
        fill_rect_rgba(&mut img_512, 376, 66, 404, 94, [255, 255, 255, 255]);
        fill_rect_rgba(&mut img_512, 40, 140, 220, 168, [255, 255, 255, 215]);
        fill_rect_rgba(&mut img_512, 70, 126, 180, 140, [255, 255, 255, 215]);
        fill_rect_rgba(&mut img_512, 270, 170, 470, 198, [255, 255, 255, 215]);
    }

    // Ground layers (stepped voxel terrain)
    let dirt_color = if is_night {
        [45, 30, 20, 255]
    } else {
        [134, 96, 67, 255]
    };
    let grass_color = if is_night {
        [25, 45, 20, 255]
    } else {
        [86, 148, 48, 255]
    };
    fill_rect_rgba(&mut img_512, 0, horizon_y, 512, 512, dirt_color);
    fill_rect_rgba(&mut img_512, 0, horizon_y, 512, horizon_y + 24, grass_color);
    fill_rect_rgba(&mut img_512, 0, horizon_y - 20, 180, horizon_y, dirt_color);
    fill_rect_rgba(&mut img_512, 0, horizon_y - 20, 180, horizon_y - 4, grass_color);
    fill_rect_rgba(&mut img_512, 320, horizon_y - 14, 512, horizon_y, dirt_color);
    fill_rect_rgba(&mut img_512, 320, horizon_y - 14, 512, horizon_y - 2, grass_color);

    // 2px black border frame
    fill_rect_rgba(&mut img_512, 0, 0, 512, 2, [0, 0, 0, 255]);
    fill_rect_rgba(&mut img_512, 0, 510, 512, 512, [0, 0, 0, 255]);
    fill_rect_rgba(&mut img_512, 0, 0, 2, 512, [0, 0, 0, 255]);
    fill_rect_rgba(&mut img_512, 510, 0, 512, 512, [0, 0, 0, 255]);

    let img_256 = image::imageops::resize(&img_512, 256, 256, image::imageops::FilterType::Lanczos3);

    let addon_maps_dir = addon_dir.join("maps");
    let addon_thumb_dir = addon_maps_dir.join("thumb");
    std::fs::create_dir_all(&addon_thumb_dir).map_err(|e| e.to_string())?;
    let addon_thumb_path = addon_thumb_dir.join(format!("{}.png", clean_map));
    let addon_icon_path = addon_maps_dir.join(format!("{}.png", clean_map));
    crate::atlas::save_png_fast(&img_512, &addon_thumb_path)?;
    crate::atlas::save_png_fast(&img_256, &addon_icon_path)?;

    if let Some(gd) = game_dir {
        if !gd.as_os_str().is_empty() && gd.is_dir() {
            let gd_maps_dir = gd.join("maps");
            let gd_thumb_dir = gd_maps_dir.join("thumb");
            if std::fs::create_dir_all(&gd_thumb_dir).is_ok() {
                let _ = std::fs::copy(&addon_thumb_path, gd_thumb_dir.join(format!("{}.png", clean_map)));
                let _ = std::fs::copy(&addon_icon_path, gd_maps_dir.join(format!("{}.png", clean_map)));
            }
        }
    }
    Ok(())
}

/// Saves a rendered 512x512 map preview image to `maps/thumb/<map>.png` (512x512)
/// and `maps/<map>.png` (256x256) in `addon_dir` and optionally `game_dir`.
pub fn save_map_thumbnail_image(
    img_512: &image::RgbaImage,
    map_name: &str,
    addon_dir: &Path,
    game_dir: Option<&Path>,
) -> Result<(), String> {
    let clean_map = map_name.trim();
    let img_256 = image::imageops::resize(img_512, 256, 256, image::imageops::FilterType::Lanczos3);
    let addon_maps_dir = addon_dir.join("maps");
    let addon_thumb_dir = addon_maps_dir.join("thumb");
    std::fs::create_dir_all(&addon_thumb_dir).map_err(|e| e.to_string())?;
    let addon_thumb_path = addon_thumb_dir.join(format!("{}.png", clean_map));
    let addon_icon_path = addon_maps_dir.join(format!("{}.png", clean_map));
    crate::atlas::save_png_fast(img_512, &addon_thumb_path)?;
    crate::atlas::save_png_fast(&img_256, &addon_icon_path)?;

    if let Some(gd) = game_dir {
        if !gd.as_os_str().is_empty() && gd.is_dir() {
            let gd_maps_dir = gd.join("maps");
            let gd_thumb_dir = gd_maps_dir.join("thumb");
            if std::fs::create_dir_all(&gd_thumb_dir).is_ok() {
                let _ = std::fs::copy(&addon_thumb_path, gd_thumb_dir.join(format!("{}.png", clean_map)));
                let _ = std::fs::copy(&addon_icon_path, gd_maps_dir.join(format!("{}.png", clean_map)));
            }
        }
    }
    Ok(())
}

#[inline]
fn blend_pixel_rgba(img: &mut image::RgbaImage, x: i32, y: i32, col: [u8; 4]) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let px = img.get_pixel_mut(x as u32, y as u32);
    if col[3] == 255 {
        *px = image::Rgba(col);
        return;
    }
    let a = col[3] as f32 / 255.0;
    let inv = 1.0 - a;
    px[0] = (col[0] as f32 * a + px[0] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    px[1] = (col[1] as f32 * a + px[1] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    px[2] = (col[2] as f32 * a + px[2] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    px[3] = 255;
}

fn fill_conv_poly(img: &mut image::RgbaImage, pts: &[(f32, f32)], col: [u8; 4]) {
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for &(_, y) in pts {
        if y < min_y {
            min_y = y;
        }
        if y > max_y {
            max_y = y;
        }
    }
    let y0 = (min_y.floor() as i32).max(0);
    let y1 = (max_y.ceil() as i32).min(img.height() as i32 - 1);
    let n = pts.len();
    let mut xs = [0.0f32; 8];
    for y in y0..=y1 {
        let sy = y as f32 + 0.5;
        let mut count = 0usize;
        for i in 0..n {
            let (x1, y1p) = pts[i];
            let (x2, y2p) = pts[(i + 1) % n];
            if (y1p <= sy && y2p > sy) || (y2p <= sy && y1p > sy) {
                let t = (sy - y1p) / (y2p - y1p);
                if count < xs.len() {
                    xs[count] = x1 + t * (x2 - x1);
                    count += 1;
                }
            }
        }
        if count >= 2 {
            let slice = &mut xs[..count];
            slice.sort_by(|a, b| a.total_cmp(b));
            let mut k = 0;
            while k + 1 < count {
                let ix0 = (slice[k].floor() as i32).max(0);
                let ix1 = (slice[k + 1].ceil() as i32).min(img.width() as i32 - 1);
                for x in ix0..=ix1 {
                    blend_pixel_rgba(img, x, y, col);
                }
                k += 2;
            }
        }
    }
}

#[allow(clippy::manual_checked_ops)]
fn avg_texture_rgb(assets: &crate::assets::AssetIndex, tex_ref: &str) -> Option<[u8; 3]> {
    let img = assets.load_texture_image(tex_ref)?;
    let side = img.width().min(img.height()).max(1);
    let mut r_sum = 0u64;
    let mut g_sum = 0u64;
    let mut b_sum = 0u64;
    let mut cnt = 0u64;
    for y in 0..side {
        for x in 0..side {
            let p = img.get_pixel(x, y);
            if p[3] >= 64 {
                r_sum += p[0] as u64;
                g_sum += p[1] as u64;
                b_sum += p[2] as u64;
                cnt += 1;
            }
        }
    }
    if cnt == 0 {
        None
    } else {
        Some([
            (r_sum / cnt) as u8,
            (g_sum / cnt) as u8,
            (b_sum / cnt) as u8,
        ])
    }
}

fn fallback_block_preview_rgb(short: &str) -> [u8; 3] {
    if short == "grass_block" || short.contains("grass") {
        [106, 168, 62]
    } else if short.contains("leaves") {
        [72, 138, 48]
    } else if short.contains("water") {
        [52, 114, 224]
    } else if short.contains("lava") {
        [235, 108, 28]
    } else if short.contains("sand") || short == "dirt_path" {
        [214, 198, 134]
    } else if short.contains("dirt") || short.contains("podzol") {
        [134, 96, 67]
    } else if short.contains("snow") || short.contains("quartz") || short.contains("white") {
        [236, 240, 244]
    } else if short.contains("oak") || short.contains("plank") || short.contains("log") || short.contains("wood") {
        [162, 130, 78]
    } else if short.contains("brick") {
        [152, 92, 76]
    } else if short.contains("glass") || short.contains("ice") {
        [165, 208, 242]
    } else if short.contains("torch") || short.contains("glow") || short.contains("lantern") {
        [248, 212, 96]
    } else {
        [132, 136, 140]
    }
}

/// Renders a real 512x512 orthographic isometric 3D preview of the actual exported Minecraft map
/// using the voxel grid and real block textures from `AssetIndex`, with directional sunlight shading
/// and translucent water blending.
pub fn render_orthographic_map_preview(
    grid: &crate::anvil::VoxelGrid,
    assets: &crate::assets::AssetIndex,
    _map_name: &str,
    sky_style: &str,
) -> image::RgbaImage {
    let w = 512u32;
    let h = 512u32;
    let is_night = sky_style.trim().eq_ignore_ascii_case("night");
    let mut img = image::ImageBuffer::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));

    // 1. Sky gradient backdrop
    for y in 0..h {
        let t = y as f32 / (h as f32);
        let (r, g, b) = if is_night {
            (
                (8.0 * (1.0 - t) + 22.0 * t) as u8,
                (12.0 * (1.0 - t) + 34.0 * t) as u8,
                (28.0 * (1.0 - t) + 68.0 * t) as u8,
            )
        } else {
            (
                (118.0 * (1.0 - t) + 196.0 * t) as u8,
                (168.0 * (1.0 - t) + 220.0 * t) as u8,
                (252.0 * (1.0 - t) + 255.0 * t) as u8,
            )
        };
        for x in 0..w {
            img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
        }
    }

    // 2. Precompute per-palette preview colors (top_rgb, side_rgb, alpha, is_opaque, is_visible)
    type PalPreviewInfo = ([u8; 3], [u8; 3], u8, bool, bool);
    let mut pal_info: Vec<PalPreviewInfo> = Vec::with_capacity(grid.palette.len());
    let mut rgb_cache: HashMap<String, Option<[u8; 3]>> = HashMap::new();
    let mut cached_avg_rgb = |tex: &str| -> Option<[u8; 3]> {
        if let Some(hit) = rgb_cache.get(tex) {
            return *hit;
        }
        let val = avg_texture_rgb(assets, tex);
        rgb_cache.insert(tex.to_string(), val);
        val
    };
    for bs in &grid.palette {
        let short = bs.short_name();
        if bs.is_air || short == "barrier" || short == "light" || short == "structure_void" {
            pal_info.push(([0, 0, 0], [0, 0, 0], 0, false, false));
            continue;
        }
        if bs.is_water_tag {
            pal_info.push(([54, 118, 228], [42, 96, 198], 185, false, true));
            continue;
        }
        let is_trans = short.contains("glass") || short.contains("ice") || short.contains("slime") || short.contains("honey");
        let is_cutout = short.contains("leaves")
            || short.contains("grass") && short != "grass_block"
            || short.contains("flower")
            || short.contains("torch")
            || short.contains("vine")
            || short.contains("sapling")
            || short.contains("rail")
            || short.contains("fern")
            || short.contains("door")
            || short.contains("fence")
            || short.contains("wall")
            || short.contains("slab")
            || short.contains("stairs")
            || short.contains("carpet")
            || short.contains("snow");

        let ns = bs
            .name
            .split_once(':')
            .map(|(n, _)| n)
            .unwrap_or("minecraft");
        let (top_cand, side_cand) = if short == "grass_block" {
            (
                format!("{}:block/grass_block_top", ns),
                format!("{}:block/grass_block_side", ns),
            )
        } else if short.ends_with("_log") || short.ends_with("_stem") {
            (
                format!("{}:block/{}_top", ns, short),
                format!("{}:block/{}", ns, short),
            )
        } else {
            let base = format!("{}:block/{}", ns, short);
            (base.clone(), base)
        };

        let mut top_rgb = cached_avg_rgb(&top_cand)
            .or_else(|| {
                if side_cand != top_cand {
                    cached_avg_rgb(&side_cand)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| fallback_block_preview_rgb(short));
        let mut side_rgb = if side_cand == top_cand {
            top_rgb
        } else {
            cached_avg_rgb(&side_cand).unwrap_or(top_rgb)
        };

        // Apply plains grass / foliage tint to grayscale vanilla textures
        if short == "grass_block" {
            top_rgb = [
                ((top_rgb[0] as f32 * 145.0) / 255.0) as u8,
                ((top_rgb[1] as f32 * 189.0) / 255.0) as u8,
                ((top_rgb[2] as f32 * 89.0) / 255.0) as u8,
            ];
            side_rgb = [134, 96, 67];
        } else if short.contains("leaves") {
            let tint = if short.contains("birch") {
                [128.0f32, 167.0, 85.0]
            } else if short.contains("spruce") {
                [97.0, 153.0, 97.0]
            } else {
                [119.0, 171.0, 47.0]
            };
            top_rgb = [
                ((top_rgb[0] as f32 * tint[0]) / 255.0) as u8,
                ((top_rgb[1] as f32 * tint[1]) / 255.0) as u8,
                ((top_rgb[2] as f32 * tint[2]) / 255.0) as u8,
            ];
            side_rgb = top_rgb;
        }

        let alpha = if is_trans { 160 } else { 255 };
        let is_opaque = !is_trans && !is_cutout;
        pal_info.push((top_rgb, side_rgb, alpha, is_opaque, true));
    }

    // 3. Find tight non-air bounding box
    let sx = grid.coords.size_x();
    let sy = grid.coords.size_y();
    let sz = grid.coords.size_z();
    let (mut bmin_x, mut bmax_x) = (usize::MAX, 0usize);
    let (mut bmin_y, mut bmax_y) = (usize::MAX, 0usize);
    let (mut bmin_z, mut bmax_z) = (usize::MAX, 0usize);

    for gy in 0..sy {
        for gz in 0..sz {
            let row = (gy * sz + gz) * sx;
            for gx in 0..sx {
                let pid = grid.blocks[row + gx] as usize;
                if pal_info[pid].4 {
                    bmin_x = bmin_x.min(gx);
                    bmax_x = bmax_x.max(gx);
                    bmin_y = bmin_y.min(gy);
                    bmax_y = bmax_y.max(gy);
                    bmin_z = bmin_z.min(gz);
                    bmax_z = bmax_z.max(gz);
                }
            }
        }
    }

    if bmin_x <= bmax_x && bmin_y <= bmax_y && bmin_z <= bmax_z {
        // Subsample stride for very large maps so preview renders in <50ms
        let max_dim = (bmax_x - bmin_x + 1).max(bmax_z - bmin_z + 1);
        let step = (max_dim / 160).max(1);

        let dx = ((bmax_x - bmin_x) / step + 1) as f32;
        let dy = ((bmax_y - bmin_y) / step + 1) as f32;
        let dz = ((bmax_z - bmin_z) / step + 1) as f32;

        // Orthographic isometric projection metrics:
        // iso_x = (rx - rz) * hw, iso_y = (rx + rz) * hh - ry * hz
        // Total horizontal span = (dx + dz) * hw, vertical span = (dx + dz) * hh + dy * hz
        let usable_w = 472.0f32;
        let usable_h = 472.0f32;
        let aspect_hh = 0.5f32;
        let aspect_hz = 1.0f32;
        let hw_from_w = usable_w / (dx + dz).max(1.0);
        let hw_from_h = usable_h / ((dx + dz) * aspect_hh + dy * aspect_hz).max(1.0);
        let hw = hw_from_w.min(hw_from_h).clamp(1.2, 22.0);
        let hh = hw * aspect_hh;
        let hz = hw * aspect_hz;

        let min_ix = -dz * hw;
        let max_ix = dx * hw;
        let min_iy = -dy * hz;
        let max_iy = (dx + dz) * hh;
        let off_x = (w as f32) * 0.5 - (min_ix + max_ix) * 0.5;
        let off_y = (h as f32) * 0.5 - (min_iy + max_iy) * 0.5;

        let is_opaque_at = |gx: usize, gy: usize, gz: usize| -> bool {
            if gx >= sx || gy >= sy || gz >= sz {
                return false;
            }
            let pid = grid.blocks[(gy * sz + gz) * sx + gx] as usize;
            pal_info[pid].3
        };

        let night_mul = if is_night { 0.45f32 } else { 1.0f32 };
        let scale_rgb = |rgb: [u8; 3], f: f32, a: u8| -> [u8; 4] {
            [
                ((rgb[0] as f32 * f).round()).clamp(0.0, 255.0) as u8,
                ((rgb[1] as f32 * f).round()).clamp(0.0, 255.0) as u8,
                ((rgb[2] as f32 * f).round()).clamp(0.0, 255.0) as u8,
                a,
            ]
        };

        // Painter's order: back-to-front (gz, gx ascending) and bottom-to-top (gy ascending)
        let mut gy = bmin_y;
        while gy <= bmax_y {
            let ry = ((gy - bmin_y) / step) as f32;
            let elev_shade = 0.90 + 0.14 * (ry / dy.max(1.0));
            let mut gz = bmin_z;
            while gz <= bmax_z {
                let rz = ((gz - bmin_z) / step) as f32;
                let mut gx = bmin_x;
                while gx <= bmax_x {
                    let pid = grid.blocks[(gy * sz + gz) * sx + gx] as usize;
                    let (top_rgb, side_rgb, alpha, is_op, visible) = pal_info[pid];
                    if visible {
                        // Skip interior voxels completely hidden on all 3 visible faces (+Y, +X, +Z)
                        let occ_up = is_opaque_at(gx, gy + step, gz);
                        let occ_px = is_opaque_at(gx + step, gy, gz);
                        let occ_pz = is_opaque_at(gx, gy, gz + step);
                        if !(is_op && occ_up && occ_px && occ_pz) {
                            let rx = ((gx - bmin_x) / step) as f32;
                            let cx = off_x + (rx - rz) * hw;
                            let cy = off_y + (rx + rz) * hh - ry * hz;

                            // Directional sun shadow raycast
                            let mut shadowed = false;
                            if !occ_up {
                                for k in 1..=10usize {
                                    let sx_i = gx as isize - (k * step) as isize;
                                    let sy_i = gy + k * step;
                                    let sz_i = gz as isize - (k * step) as isize;
                                    if sx_i < 0 || sz_i < 0 || sy_i >= sy {
                                        break;
                                    }
                                    if is_opaque_at(sx_i as usize, sy_i, sz_i as usize) {
                                        shadowed = true;
                                        break;
                                    }
                                }
                            }
                            let sun_mul = if shadowed { 0.72 } else { 1.0 };

                            let top_c = scale_rgb(top_rgb, 1.02 * elev_shade * sun_mul * night_mul, alpha);
                            let left_c = scale_rgb(side_rgb, 0.78 * elev_shade * night_mul, alpha);
                            let right_c = scale_rgb(side_rgb, 0.58 * elev_shade * night_mul, alpha);

                            // Top diamond (+Y)
                            if !occ_up {
                                fill_conv_poly(
                                    &mut img,
                                    &[
                                        (cx, cy - hh - hz),
                                        (cx + hw, cy - hz),
                                        (cx, cy + hh - hz),
                                        (cx - hw, cy - hz),
                                    ],
                                    top_c,
                                );
                            }
                            // Left face (+Z)
                            if !occ_pz {
                                fill_conv_poly(
                                    &mut img,
                                    &[
                                        (cx - hw, cy - hz),
                                        (cx, cy + hh - hz),
                                        (cx, cy + hh),
                                        (cx - hw, cy),
                                    ],
                                    left_c,
                                );
                            }
                            // Right face (+X)
                            if !occ_px {
                                fill_conv_poly(
                                    &mut img,
                                    &[
                                        (cx, cy + hh - hz),
                                        (cx + hw, cy - hz),
                                        (cx + hw, cy),
                                        (cx, cy + hh),
                                    ],
                                    right_c,
                                );
                            }
                        }
                    }
                    gx += step;
                }
                gz += step;
            }
            gy += step;
        }
    }

    // 4. Clean 2px border frame (pure map view, no text overlay)
    fill_rect_rgba(&mut img, 0, 0, 512, 2, [0, 0, 0, 255]);
    fill_rect_rgba(&mut img, 0, 510, 512, 512, [0, 0, 0, 255]);
    fill_rect_rgba(&mut img, 0, 0, 2, 512, [0, 0, 0, 255]);
    fill_rect_rgba(&mut img, 510, 0, 512, 512, [0, 0, 0, 255]);

    img
}
