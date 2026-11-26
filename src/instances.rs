use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
    Vanilla,
    PrismLauncher,
    MultiMC,
    Custom,
}

impl LauncherKind {
    pub fn short_label(self) -> &'static str {
        match self {
            LauncherKind::Vanilla => "Vanilla",
            LauncherKind::PrismLauncher => "Prism",
            LauncherKind::MultiMC => "MultiMC",
            LauncherKind::Custom => "Custom",
        }
    }

    fn sort_order(self) -> u8 {
        match self {
            LauncherKind::Vanilla => 0,
            LauncherKind::PrismLauncher => 1,
            LauncherKind::MultiMC => 2,
            LauncherKind::Custom => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredWorld {
    pub name: String,
    pub path: PathBuf,
    pub modified_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftInstance {
    pub launcher: LauncherKind,
    pub name: String,
    pub mc_version: Option<String>,
    pub loader: Option<String>,
    pub instance_dir: PathBuf,
    pub saves_dir: PathBuf,
    pub worlds: Vec<DiscoveredWorld>,
}

impl MinecraftInstance {
    pub fn display_label(&self) -> String {
        let ver_part = match self.mc_version.as_deref() {
            Some(v) if !v.is_empty() && !self.name.to_lowercase().contains(&v.to_lowercase()) => {
                format!(" ({})", v)
            }
            _ => String::new(),
        };
        format!(
            "{} — {}{} · {}",
            self.launcher.short_label(),
            self.name,
            ver_part,
            self.worlds.len()
        )
    }

    pub fn short_tag(&self) -> String {
        if self.launcher == LauncherKind::Vanilla && self.name == "Default (.minecraft)" {
            "Vanilla".to_string()
        } else {
            format!("{}: {}", self.launcher.short_label(), self.name)
        }
    }
}

pub fn parse_ini_value(content: &str, key: &str) -> Option<String> {
    let trimmed_bom = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut current_section: Option<&str> = None;

    for raw_line in trimmed_bom.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            current_section = Some(line[1..line.len() - 1].trim());
            continue;
        }
        let in_allowed_section = match current_section {
            None => true,
            Some(sec) => sec.eq_ignore_ascii_case("general"),
        };
        if !in_allowed_section {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if !k.trim().eq_ignore_ascii_case(key) {
            continue;
        }
        let val = v.trim();
        if val.is_empty() || val.starts_with("@Variant(") || val.starts_with("@ByteArray(") {
            return None;
        }
        let unquoted = if val.len() >= 2 && val.starts_with('"') && val.ends_with('"') {
            val[1..val.len() - 1]
                .replace("\\\\", "\\")
                .replace("\\\"", "\"")
        } else {
            val.to_string()
        };
        if unquoted.is_empty() {
            return None;
        }
        return Some(unquoted);
    }
    None
}

pub fn parse_mmc_pack_metadata(content: &str) -> (Option<String>, Option<String>) {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return (None, None);
    };
    let Some(components) = val.get("components").and_then(|v| v.as_array()) else {
        return (None, None);
    };

    let mut mc_version: Option<String> = None;
    let mut loader: Option<String> = None;

    for comp in components {
        let uid = comp.get("uid").and_then(|v| v.as_str()).unwrap_or("");
        match uid {
            "net.minecraft" => {
                if let Some(ver) = comp.get("version").and_then(|v| v.as_str()) {
                    let trimmed = ver.trim();
                    if !trimmed.is_empty() {
                        mc_version = Some(trimmed.to_string());
                    }
                }
            }
            "net.fabricmc.fabric-loader" => loader = Some("Fabric".to_string()),
            "net.minecraftforge" => loader = Some("Forge".to_string()),
            "net.neoforged" => loader = Some("NeoForge".to_string()),
            "org.quiltmc.quilt-loader" => loader = Some("Quilt".to_string()),
            _ => {}
        }
    }
    (mc_version, loader)
}

pub fn strip_windows_verbatim_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{}", rest))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

fn clean_path(path: &Path) -> PathBuf {
    strip_windows_verbatim_prefix(path)
}

fn canon_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

pub(crate) fn is_valid_world_dir(dir: &Path) -> bool {
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

pub fn scan_worlds_in_saves_dir(saves_dir: &Path) -> Vec<DiscoveredWorld> {
    let mut worlds = Vec::new();
    if !saves_dir.is_dir() {
        return worlds;
    }
    let Ok(rd) = std::fs::read_dir(saves_dir) else {
        return worlds;
    };
    for ent in rd.flatten() {
        let p = clean_path(&ent.path());
        if !is_valid_world_dir(&p) {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("world")
            .to_string();
        let modified_secs = p
            .join("level.dat")
            .metadata()
            .or_else(|_| p.metadata())
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        worlds.push(DiscoveredWorld {
            name,
            path: p,
            modified_secs,
        });
    }
    worlds.sort_by(|a, b| {
        b.modified_secs
            .cmp(&a.modified_secs)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    worlds
}

pub(crate) fn is_snapshot_token(tok: &str) -> bool {
    let lower = tok.to_ascii_lowercase();
    let Some((yr, rest)) = lower.split_once('w') else {
        return false;
    };
    yr.len() == 2
        && yr.bytes().all(|b| b.is_ascii_digit())
        && rest.len() >= 3
        && rest.as_bytes().get(..2).is_some_and(|b| b.iter().all(u8::is_ascii_digit))
}

fn extract_version_and_loader(version_id: &str) -> (Option<String>, Option<String>) {
    let trimmed = version_id.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    let lower = trimmed.to_ascii_lowercase();
    let loader = if lower.contains("neoforge") {
        Some("NeoForge".to_string())
    } else if lower.contains("forge") {
        Some("Forge".to_string())
    } else if lower.contains("fabric") {
        Some("Fabric".to_string())
    } else if lower.contains("quilt") {
        Some("Quilt".to_string())
    } else {
        None
    };

    if matches!(lower.as_str(), "latest-release" | "latest-snapshot") {
        return (None, loader);
    }

    let mut mc_ver: Option<String> = None;
    for raw_tok in trimmed.split(['/', '\\', '-', '_', ' ']) {
        let tok = raw_tok.trim();
        if tok.is_empty() {
            continue;
        }
        if crate::assets::parse_mc_version_tuple(tok).is_some() {
            mc_ver = Some(tok.to_string());
        }
    }
    if mc_ver.is_none() && is_snapshot_token(trimmed) {
        mc_ver = Some(trimmed.to_string());
    }
    (mc_ver, loader)
}

fn resolve_cfg_relative_path(base: &Path, raw: &str) -> PathBuf {
    let normalized = raw.trim().replace('/', "\\");
    let p = PathBuf::from(&normalized);
    if p.is_absolute() {
        clean_path(&p)
    } else {
        clean_path(&base.join(p))
    }
}

pub fn discover_vanilla_instances(mc_dir: &Path) -> Vec<MinecraftInstance> {
    let mut out = Vec::new();
    let mut seen_saves: HashSet<String> = HashSet::new();
    if !mc_dir.is_dir() {
        return out;
    }

    let default_saves = clean_path(&mc_dir.join("saves"));
    let default_saves_key = canon_key(&default_saves);

    for profile_file in [
        "launcher_profiles.json",
        "launcher_profiles_microsoft_store.json",
    ] {
        let pf_path = mc_dir.join(profile_file);
        let Ok(text) = std::fs::read_to_string(&pf_path) else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(profiles) = val.get("profiles").and_then(|v| v.as_object()) else {
            continue;
        };
        for (_id, prof) in profiles {
            let Some(game_dir_raw) = prof.get("gameDir").and_then(|v| v.as_str()) else {
                continue;
            };
            let game_dir_trimmed = game_dir_raw.trim();
            if game_dir_trimmed.is_empty() {
                continue;
            }
            let game_dir = resolve_cfg_relative_path(mc_dir, game_dir_trimmed);
            let saves_dir = clean_path(&game_dir.join("saves"));
            if !saves_dir.is_dir() {
                continue;
            }
            let skey = canon_key(&saves_dir);
            if skey == default_saves_key || !seen_saves.insert(skey) {
                continue;
            }
            let last_ver = prof
                .get("lastVersionId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let (mc_version, loader) = extract_version_and_loader(last_ver);
            let prof_name = prof
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let name = prof_name
                .map(|s| s.to_string())
                .or_else(|| {
                    (!last_ver.is_empty()
                        && !matches!(
                            last_ver.to_ascii_lowercase().as_str(),
                            "latest-release" | "latest-snapshot"
                        ))
                    .then(|| last_ver.to_string())
                })
                .unwrap_or_else(|| {
                    game_dir
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Vanilla")
                        .to_string()
                });
            let worlds = scan_worlds_in_saves_dir(&saves_dir);
            out.push(MinecraftInstance {
                launcher: LauncherKind::Vanilla,
                name,
                mc_version,
                loader,
                instance_dir: game_dir,
                saves_dir,
                worlds,
            });
        }
    }

    if default_saves.is_dir() && seen_saves.insert(default_saves_key) {
        let worlds = scan_worlds_in_saves_dir(&default_saves);
        out.push(MinecraftInstance {
            launcher: LauncherKind::Vanilla,
            name: "Default (.minecraft)".to_string(),
            mc_version: None,
            loader: None,
            instance_dir: clean_path(mc_dir),
            saves_dir: default_saves,
            worlds,
        });
    }

    for sub in ["instances", "versions"] {
        let base = mc_dir.join(sub);
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        for ent in rd.flatten() {
            let inst_dir = clean_path(&ent.path());
            if !inst_dir.is_dir() {
                continue;
            }
            let saves_dir = clean_path(&inst_dir.join("saves"));
            if !saves_dir.is_dir() {
                continue;
            }
            if !seen_saves.insert(canon_key(&saves_dir)) {
                continue;
            }
            let folder_name = inst_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Vanilla")
                .to_string();
            let (mc_version, loader) = extract_version_and_loader(&folder_name);
            let worlds = scan_worlds_in_saves_dir(&saves_dir);
            out.push(MinecraftInstance {
                launcher: LauncherKind::Vanilla,
                name: folder_name,
                mc_version,
                loader,
                instance_dir: inst_dir,
                saves_dir,
                worlds,
            });
        }
    }

    out
}

pub fn discover_multimc_family_instances(
    launcher_root: &Path,
    kind: LauncherKind,
) -> Vec<MinecraftInstance> {
    if !launcher_root.is_dir() {
        return Vec::new();
    }
    let cfg_name = match kind {
        LauncherKind::PrismLauncher => "prismlauncher.cfg",
        LauncherKind::MultiMC => "multimc.cfg",
        _ => return Vec::new(),
    };

    let mut inst_dirs: Vec<PathBuf> = Vec::new();
    let mut seen_dirs: HashSet<String> = HashSet::new();

    let cfg_path = launcher_root.join(cfg_name);
    if let Ok(cfg_text) = std::fs::read_to_string(&cfg_path) {
        if let Some(raw_dir) = parse_ini_value(&cfg_text, "InstanceDir") {
            let resolved = resolve_cfg_relative_path(launcher_root, &raw_dir);
            if resolved.is_dir() && seen_dirs.insert(canon_key(&resolved)) {
                inst_dirs.push(resolved);
            }
        }
    }

    let default_inst = clean_path(&launcher_root.join("instances"));
    if default_inst.is_dir() && seen_dirs.insert(canon_key(&default_inst)) {
        inst_dirs.push(default_inst);
    }

    let mut out = Vec::new();
    for dir in inst_dirs {
        out.extend(scan_multimc_instances_dir(&dir, kind));
    }
    out
}

fn parse_single_multimc_instance(inst_dir: &Path, kind: LauncherKind) -> Option<MinecraftInstance> {
    let s1 = inst_dir.join("minecraft").join("saves");
    let s2 = inst_dir.join(".minecraft").join("saves");
    let saves_dir = if s1.is_dir() {
        clean_path(&s1)
    } else if s2.is_dir() {
        clean_path(&s2)
    } else {
        return None;
    };

    let folder_name = inst_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Instance")
        .to_string();
    let name = std::fs::read_to_string(inst_dir.join("instance.cfg"))
        .ok()
        .and_then(|t| parse_ini_value(&t, "name"))
        .unwrap_or(folder_name);

    let (mc_version, loader) = std::fs::read_to_string(inst_dir.join("mmc-pack.json"))
        .ok()
        .map(|t| parse_mmc_pack_metadata(&t))
        .unwrap_or((None, None));

    let worlds = scan_worlds_in_saves_dir(&saves_dir);
    Some(MinecraftInstance {
        launcher: kind,
        name,
        mc_version,
        loader,
        instance_dir: clean_path(inst_dir),
        saves_dir,
        worlds,
    })
}

pub fn scan_multimc_instances_dir(
    instances_dir: &Path,
    kind: LauncherKind,
) -> Vec<MinecraftInstance> {
    let mut out = Vec::new();
    if !instances_dir.is_dir() {
        return out;
    }
    let Ok(rd) = std::fs::read_dir(instances_dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let p = clean_path(&ent.path());
        if !p.is_dir() {
            continue;
        }
        let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if fname.is_empty() || fname.starts_with('.') || fname.starts_with('_') {
            continue;
        }
        if let Some(inst) = parse_single_multimc_instance(&p, kind) {
            out.push(inst);
        }
    }
    out
}

fn infer_multimc_kind(path: &Path) -> LauncherKind {
    let low = path.to_string_lossy().to_ascii_lowercase();
    if low.contains("prism") {
        LauncherKind::PrismLauncher
    } else if low.contains("multimc") {
        LauncherKind::MultiMC
    } else {
        LauncherKind::Custom
    }
}

fn dir_looks_like_instances_container(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_dir() {
            continue;
        }
        if p.join("instance.cfg").is_file()
            || p.join("mmc-pack.json").is_file()
            || p.join("minecraft").join("saves").is_dir()
            || p.join(".minecraft").join("saves").is_dir()
            || p.join("saves").is_dir()
        {
            return true;
        }
    }
    false
}

fn scan_generic_instances_container(dir: &Path, kind: LauncherKind) -> Vec<MinecraftInstance> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let p = clean_path(&ent.path());
        if !p.is_dir() {
            continue;
        }
        let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if fname.is_empty() || fname.starts_with('.') || fname.starts_with('_') {
            continue;
        }
        if let Some(inst) = parse_single_multimc_instance(&p, kind) {
            out.push(inst);
            continue;
        }
        let saves = p.join("saves");
        if saves.is_dir() {
            let saves_clean = clean_path(&saves);
            let worlds = scan_worlds_in_saves_dir(&saves_clean);
            out.push(MinecraftInstance {
                launcher: kind,
                name: fname.to_string(),
                mc_version: None,
                loader: None,
                instance_dir: p,
                saves_dir: saves_clean,
                worlds,
            });
        }
    }
    out
}

pub fn scan_custom_dir(dir: &Path) -> Vec<MinecraftInstance> {
    let dir = clean_path(dir);
    if !dir.is_dir() {
        return Vec::new();
    }

    // 1 & 2. Prism / MultiMC root
    if dir.join("prismlauncher.cfg").is_file() {
        return discover_multimc_family_instances(&dir, LauncherKind::PrismLauncher);
    }
    if dir.join("multimc.cfg").is_file() {
        return discover_multimc_family_instances(&dir, LauncherKind::MultiMC);
    }

    // 3. Vanilla .minecraft root
    if dir.join("launcher_profiles.json").is_file()
        || dir.join("launcher_profiles_microsoft_store.json").is_file()
    {
        return discover_vanilla_instances(&dir);
    }

    // 4. Single MultiMC / Prism instance dir
    if dir.join("instance.cfg").is_file() || dir.join("mmc-pack.json").is_file() {
        if let Some(inst) = parse_single_multimc_instance(&dir, infer_multimc_kind(&dir)) {
            return vec![inst];
        }
    }

    // 5. Parent containing an `instances/` dir
    let sub_instances = dir.join("instances");
    if sub_instances.is_dir() {
        let kind = infer_multimc_kind(&dir);
        let found = scan_generic_instances_container(&sub_instances, kind);
        if !found.is_empty() {
            return found;
        }
    }

    // 6. Direct `instances/` container
    if dir_looks_like_instances_container(&dir) {
        let kind = infer_multimc_kind(&dir);
        let found = scan_generic_instances_container(&dir, kind);
        if !found.is_empty() {
            return found;
        }
    }

    // 7. Folder containing `saves/`
    let sub_saves = dir.join("saves");
    if sub_saves.is_dir() {
        let saves_clean = clean_path(&sub_saves);
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Custom")
            .to_string();
        let worlds = scan_worlds_in_saves_dir(&saves_clean);
        return vec![MinecraftInstance {
            launcher: LauncherKind::Custom,
            name,
            mc_version: None,
            loader: None,
            instance_dir: dir,
            saves_dir: saves_clean,
            worlds,
        }];
    }

    // 8. Single world folder
    if is_valid_world_dir(&dir) {
        let world_name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("world")
            .to_string();
        let saves_dir = dir.parent().map(clean_path).unwrap_or_else(|| dir.clone());
        let modified_secs = dir
            .join("level.dat")
            .metadata()
            .or_else(|_| dir.metadata())
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return vec![MinecraftInstance {
            launcher: LauncherKind::Custom,
            name: world_name.clone(),
            mc_version: None,
            loader: None,
            instance_dir: saves_dir.clone(),
            saves_dir,
            worlds: vec![DiscoveredWorld {
                name: world_name,
                path: dir,
                modified_secs,
            }],
        }];
    }

    // 9. Plain `saves/` directory containing world folders
    let worlds = scan_worlds_in_saves_dir(&dir);
    if !worlds.is_empty() {
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Saves")
            .to_string();
        return vec![MinecraftInstance {
            launcher: LauncherKind::Custom,
            name,
            mc_version: None,
            loader: None,
            instance_dir: dir.clone(),
            saves_dir: dir,
            worlds,
        }];
    }

    Vec::new()
}

fn collect_prism_and_multimc_roots() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut prism_roots = Vec::new();
    let mut mmc_roots = Vec::new();

    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        prism_roots.push(appdata.join("PrismLauncher"));
        mmc_roots.push(appdata.join("MultiMC"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        prism_roots.push(local.join("PrismLauncher"));
        prism_roots.push(local.join("Programs").join("PrismLauncher"));
        mmc_roots.push(local.join("MultiMC"));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        for sub in [
            "PrismLauncher",
            r"Games\PrismLauncher",
            r"Documents\PrismLauncher",
            r"Desktop\PrismLauncher",
            r"Downloads\PrismLauncher",
        ] {
            prism_roots.push(profile.join(sub));
        }
        for sub in [
            "MultiMC",
            r"Games\MultiMC",
            r"Documents\MultiMC",
            r"Desktop\MultiMC",
            r"Downloads\MultiMC",
        ] {
            mmc_roots.push(profile.join(sub));
        }
    }

    for subkey in [
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\PrismLauncher",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PrismLauncher",
        r"Software\PrismLauncher",
    ] {
        if let Some(loc) =
            crate::compiler::read_registry_string_hkcu_or_hklm(subkey, "InstallLocation")
        {
            prism_roots.push(PathBuf::from(loc));
        }
    }

    for drive in crate::compiler::get_logical_drives() {
        for rel in [
            "PrismLauncher",
            r"Games\PrismLauncher",
            r"Minecraft\PrismLauncher",
            r"Launchers\PrismLauncher",
            r"Program Files\PrismLauncher",
            r"Program Files (x86)\PrismLauncher",
        ] {
            prism_roots.push(drive.join(rel));
        }
        for rel in [
            "MultiMC",
            r"Games\MultiMC",
            r"Minecraft\MultiMC",
            r"Launchers\MultiMC",
            r"Program Files\MultiMC",
            r"Program Files (x86)\MultiMC",
        ] {
            mmc_roots.push(drive.join(rel));
        }
    }

    (prism_roots, mmc_roots)
}

fn find_config_json_for_instances(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    let cwd = PathBuf::from("config.json");
    if cwd.is_file() {
        return Some(cwd);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.json")))
        .filter(|p| p.is_file())
}

fn instance_richness_score(inst: &MinecraftInstance) -> (u8, u8, usize) {
    let non_custom = if inst.launcher != LauncherKind::Custom {
        1
    } else {
        0
    };
    let has_ver = if inst.mc_version.is_some() { 1 } else { 0 };
    (non_custom, has_ver, inst.worlds.len())
}

pub fn discover_all_instances(
    config_path: Option<&Path>,
    extra_dirs: &[PathBuf],
) -> Vec<MinecraftInstance> {
    let mut raw: Vec<MinecraftInstance> = Vec::new();

    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        raw.extend(discover_vanilla_instances(&appdata.join(".minecraft")));
    }

    let (prism_roots, mmc_roots) = collect_prism_and_multimc_roots();
    let mut seen_roots: HashSet<String> = HashSet::new();
    for root in prism_roots {
        if root.is_dir() && seen_roots.insert(canon_key(&root)) {
            raw.extend(discover_multimc_family_instances(
                &root,
                LauncherKind::PrismLauncher,
            ));
        }
    }
    for root in mmc_roots {
        if root.is_dir() && seen_roots.insert(canon_key(&root)) {
            raw.extend(discover_multimc_family_instances(
                &root,
                LauncherKind::MultiMC,
            ));
        }
    }

    if let Some(cfg_file) = find_config_json_for_instances(config_path) {
        if let Ok(text) = std::fs::read_to_string(&cfg_file) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(arr) = val.get("instance_dirs").and_then(|v| v.as_array()) {
                    for item in arr {
                        if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                            raw.extend(scan_custom_dir(Path::new(s)));
                        }
                    }
                }
                if let Some(wp) = val.get("world_path").and_then(|v| v.as_str()) {
                    let wp = wp.trim();
                    if !wp.is_empty() {
                        raw.extend(scan_custom_dir(Path::new(wp)));
                    }
                }
            }
        }
    }

    for extra in extra_dirs {
        if extra.is_dir() {
            raw.extend(scan_custom_dir(extra));
        }
    }

    // Deduplicate by canonicalized lowercase saves_dir, keeping the richer entry.
    let mut deduped: Vec<MinecraftInstance> = Vec::new();
    let mut key_to_idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for inst in raw {
        let key = canon_key(&inst.saves_dir);
        if let Some(&idx) = key_to_idx.get(&key) {
            if instance_richness_score(&inst) > instance_richness_score(&deduped[idx]) {
                deduped[idx] = inst;
            }
        } else {
            let idx = deduped.len();
            key_to_idx.insert(key, idx);
            deduped.push(inst);
        }
    }

    deduped.sort_by(|a, b| {
        let a_has_worlds = !a.worlds.is_empty();
        let b_has_worlds = !b.worlds.is_empty();
        b_has_worlds
            .cmp(&a_has_worlds)
            .then_with(|| a.launcher.sort_order().cmp(&b.launcher.sort_order()))
            .then_with(|| {
                let a_def = a.launcher == LauncherKind::Vanilla && a.name == "Default (.minecraft)";
                let b_def = b.launcher == LauncherKind::Vanilla && b.name == "Default (.minecraft)";
                b_def.cmp(&a_def)
            })
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    deduped
}

pub fn find_instance_by_query<'a>(
    instances: &'a [MinecraftInstance],
    query: &str,
) -> Option<&'a MinecraftInstance> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }

    // 1. Exact match on display name or instance folder name
    if let Some(exact) = instances.iter().find(|inst| {
        inst.name.to_lowercase() == q
            || inst
                .instance_dir
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|f| f.to_lowercase() == q)
    }) {
        return Some(exact);
    }

    // 2. Substring match on display name, folder name, or display label
    instances.iter().find(|inst| {
        inst.name.to_lowercase().contains(&q)
            || inst.display_label().to_lowercase().contains(&q)
            || inst
                .instance_dir
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|f| f.to_lowercase().contains(&q))
    })
}

pub fn resolve_world_in_instance(
    inst: &MinecraftInstance,
    world_arg: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(w) = world_arg {
        if w.is_absolute() && w.is_dir() {
            return Some(clean_path(w));
        }
        let direct = inst.saves_dir.join(w);
        if is_valid_world_dir(&direct) {
            return Some(clean_path(&direct));
        }
        let q = w.to_string_lossy().trim().to_lowercase();
        if !q.is_empty() {
            if let Some(found) = inst.worlds.iter().find(|dw| dw.name.to_lowercase() == q) {
                return Some(found.path.clone());
            }
            if let Some(found) = inst
                .worlds
                .iter()
                .find(|dw| dw.name.to_lowercase().contains(&q))
            {
                return Some(found.path.clone());
            }
        }
        if is_valid_world_dir(w) {
            return Some(clean_path(w));
        }
        return None;
    }

    inst.worlds.first().map(|w| w.path.clone())
}

pub fn discover_launcher_library_roots(world_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut push_lib_root = |lib_base: &Path| {
        let mc_lib = lib_base.join("com").join("mojang").join("minecraft");
        if mc_lib.is_dir() && seen.insert(canon_key(&mc_lib)) {
            out.push(clean_path(&mc_lib));
        }
    };

    let (prism_roots, mmc_roots) = collect_prism_and_multimc_roots();
    for (roots, cfg_name) in [
        (prism_roots, "prismlauncher.cfg"),
        (mmc_roots, "multimc.cfg"),
    ] {
        for root in roots {
            if !root.is_dir() {
                continue;
            }
            if let Ok(cfg_text) = std::fs::read_to_string(root.join(cfg_name)) {
                if let Some(lib_dir) = parse_ini_value(&cfg_text, "LibrariesDir") {
                    push_lib_root(&resolve_cfg_relative_path(&root, &lib_dir));
                }
            }
            push_lib_root(&root.join("libraries"));
        }
    }

    if let Some(wd) = world_dir {
        let mut cur = wd.to_path_buf();
        for _ in 0..5 {
            if let Some(parent) = cur.parent() {
                let p = parent.to_path_buf();
                push_lib_root(&p.join("libraries"));
                cur = p;
            } else {
                break;
            }
        }
    }

    out
}
