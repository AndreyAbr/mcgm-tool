use crate::anvil::WorldCoords;
use clap::Parser;
use std::path::PathBuf;

#[link(name = "kernel32")]
extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}

/// Auto-detects the OS UI language ("ru" or "en") using environment overrides
/// and the Windows user UI language.
pub fn detect_os_lang() -> &'static str {
    for var in ["MCGM_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            let lower = val.trim().to_ascii_lowercase();
            if lower.starts_with("ru") {
                return "ru";
            }
            if lower.starts_with("en") {
                return "en";
            }
        }
    }
    // Primary language ID 0x19 == LANG_RUSSIAN
    let lang_id = unsafe { GetUserDefaultUILanguage() };
    if (lang_id & 0x03ff) == 0x19 {
        return "ru";
    }
    "en"
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "mcgm",
    version,
    about = "Minecraft to Garry's Mod map exporter"
)]
pub struct CliArgs {
    /// Path to Minecraft world save directory
    #[arg(short = 'w', long)]
    pub world: Option<PathBuf>,

    /// Coordinates: 6 integers (minX minY minZ maxX maxY maxZ)
    #[arg(long, num_args = 1..=6, allow_hyphen_values = true)]
    pub coords: Vec<String>,

    /// Minecraft dimension (overworld, nether, end, etc.)
    #[arg(long, default_value = "overworld")]
    pub dimension: String,

    /// Model / addon name
    #[arg(short = 'n', long)]
    pub name: Option<String>,

    /// Material and model category subfolder
    #[arg(short = 'c', long, default_value = "custom_mc")]
    pub category: String,

    /// Output BSP map name (without .bsp; defaults to gm_<name>)
    #[arg(short = 'm', long)]
    pub map: Option<String>,

    /// Output addon/project directory
    #[arg(long, default_value = "output")]
    pub output_dir: PathBuf,

    /// Path to config.json with tool paths
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Directly import compiled map, models, and materials into Garry's Mod (default: true)
    #[arg(long, default_value_t = true, action = clap::ArgAction::SetTrue)]
    pub import_to_gmod: bool,

    /// Skip direct import to Garry's Mod
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub no_import_to_gmod: bool,

    /// Keep exported .vmf map source file alongside output (and garrysmod/mapsrc/)
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub export_vmf: bool,

    /// Lighting preset: minecraft, gmod, bright, night
    #[arg(long, default_value = "minecraft")]
    pub lighting_style: String,

    /// High quality VRAD lighting (-both HDR+LDR + StaticPropPolys)
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub hq_light: bool,

    /// Fast VRAD (-fast) and VVIS (-fast) pass
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub fast_vrad: bool,

    /// Build quality preset: preview, balanced, hq
    #[arg(long, value_parser = ["preview", "balanced", "hq"])]
    pub preset: Option<String>,

    /// Macro-chunk size in blocks for chunked world compilation
    #[arg(long, default_value_t = 32)]
    pub chunk_size: usize,

    /// Collision mode: per_chunk_phy or vmf_brushes
    #[arg(long, value_parser = ["per_chunk_phy", "vmf_brushes"], default_value = "per_chunk_phy")]
    pub collision_mode: String,

    /// Max worker threads for parallel StudioMDL compilation
    #[arg(long)]
    pub max_workers: Option<usize>,

    /// Max convex pieces per chunk physics model
    #[arg(long)]
    pub max_convex: Option<usize>,

    /// Enable Garry's Mod Minecraft movement and surface physics GLua addon (default: true when --world is used)
    #[arg(long, visible_alias = "minecraft-physics", action = clap::ArgAction::SetTrue)]
    pub mc_physics: bool,

    /// Disable Garry's Mod Minecraft movement and surface physics GLua addon
    #[arg(long, visible_alias = "no-minecraft-physics", action = clap::ArgAction::SetTrue)]
    pub no_mc_physics: bool,

    /// Export all collision as VMF func_detail brushes instead of model $collisionmodel
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub vmf_brushes: bool,

    /// Reuse cached geometry/models if available for fast rebuild
    #[arg(long, visible_alias = "fast-rebuild", action = clap::ArgAction::SetTrue)]
    pub reuse_cache: bool,

    /// Source Engine units per Minecraft block (default from config.json or 39.37)
    #[arg(long)]
    pub scale: Option<f64>,

    /// Interface language (ru or en; auto-detected from OS locale if omitted)
    #[arg(long)]
    pub lang: Option<String>,

    /// Force launch native Win32 GUI window
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub gui: bool,

    /// When launching GUI, automatically begin the export pipeline
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub auto_start: bool,

    /// Automatically detect external tools, update config.json, and print results
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub detect_tools: bool,

    /// Run environment diagnostic check for required and optional tools
    #[arg(long, visible_alias = "check-environment", action = clap::ArgAction::SetTrue)]
    pub check_env: bool,

    /// Used with --check-env: simulate a clean machine with no tools installed
    #[arg(long, visible_alias = "simulate-missing", action = clap::ArgAction::SetTrue)]
    pub clean: bool,

    /// Output machine-readable JSON (e.g. with --check-env or --detect-tools)
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub json: bool,

    /// List detected Minecraft launcher instances and saves folders
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub list_instances: bool,

    /// Select a Minecraft instance by name (Vanilla, Prism Launcher, MultiMC)
    #[arg(long)]
    pub instance: Option<String>,

    /// Write live-flushed debug log to this path (defaults to <work_dir>/mcgm_debug.log)
    #[arg(long)]
    pub debug_log: Option<PathBuf>,
}

pub fn sanitize_safe_name(raw: &str, default: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.trim().chars() {
        if c.is_ascii_control()
            || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    while out.contains("..") {
        out = out.replace("..", "_");
    }
    let trimmed = out.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        return default.to_string();
    }
    let upper = trimmed.to_ascii_uppercase();
    let is_dos = matches!(
        upper.as_str(),
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
    if is_dos {
        format!("mc_{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

pub fn sanitize_safe_category(raw: &str) -> String {
    let segs: Vec<String> = raw
        .split(['/', '\\'])
        .filter_map(|seg| {
            let t = seg.trim();
            if t.is_empty() || t == "." || t == ".." || t.contains("..") {
                None
            } else {
                let s = sanitize_safe_name(t, "");
                (!s.is_empty()).then_some(s)
            }
        })
        .collect();
    if segs.is_empty() {
        "minecraft".to_string()
    } else {
        segs.join("/")
    }
}

impl CliArgs {
    pub fn parsed_coords(&self) -> Result<WorldCoords, String> {
        let mut nums = Vec::new();
        for tok in &self.coords {
            for part in tok.split(|c: char| {
                matches!(c, ',' | ';' | '[' | ']' | '(' | ')') || c.is_whitespace()
            }) {
                let p = part.trim();
                if p.is_empty() {
                    continue;
                }
                let val: i32 = p
                    .parse()
                    .map_err(|_| format!("Invalid integer coordinate: '{}'", p))?;
                nums.push(val);
            }
        }
        if nums.len() != 6 {
            return Err(format!(
                "Expected 6 coordinates (minX minY minZ maxX maxY maxZ), got {}",
                nums.len()
            ));
        }
        Ok(WorldCoords {
            min_x: nums[0],
            min_y: nums[1],
            min_z: nums[2],
            max_x: nums[3],
            max_y: nums[4],
            max_z: nums[5],
        }
        .normalized())
    }

    pub fn effective_import_to_gmod(&self) -> bool {
        if self.no_import_to_gmod {
            false
        } else {
            self.import_to_gmod
        }
    }

    pub fn effective_mc_physics(&self) -> bool {
        if self.no_mc_physics {
            false
        } else if self.mc_physics {
            true
        } else {
            self.world.is_some()
        }
    }

    pub fn effective_vmf_brushes(&self) -> bool {
        self.vmf_brushes || self.collision_mode.eq_ignore_ascii_case("vmf_brushes")
    }

    pub fn effective_hq_light(&self) -> bool {
        match self.preset.as_deref() {
            Some("hq") => true,
            _ => self.hq_light,
        }
    }

    pub fn effective_fast_vrad(&self) -> bool {
        match self.preset.as_deref() {
            Some("preview") => true,
            Some("hq") => false,
            _ => self.fast_vrad,
        }
    }

    pub fn effective_reuse_cache(&self) -> bool {
        self.reuse_cache || self.preset.as_deref() == Some("preview")
    }

    pub fn effective_name(&self) -> String {
        if let Some(ref n) = self.name {
            let trimmed = n.trim();
            if !trimmed.is_empty() {
                return sanitize_safe_name(trimmed, "mc_export");
            }
        }
        if let Some(ref m) = self.map {
            let trimmed = m.trim();
            if !trimmed.is_empty() {
                let stripped = trimmed
                    .strip_prefix("gm_")
                    .filter(|s| !s.is_empty())
                    .unwrap_or(trimmed);
                return sanitize_safe_name(stripped, "mc_export");
            }
        }
        if let Some(ref w) = self.world {
            if let Some(fname) = w.file_name().and_then(|s| s.to_str()) {
                let trimmed = fname.trim();
                if !trimmed.is_empty() {
                    return sanitize_safe_name(trimmed, "mc_export");
                }
            }
        }
        "mc_export".to_string()
    }

    pub fn effective_map(&self) -> String {
        if let Some(ref m) = self.map {
            let trimmed = m.trim();
            if !trimmed.is_empty() {
                return sanitize_safe_name(trimmed, "gm_mc_export");
            }
        }
        let n = self.effective_name();
        if n.starts_with("gm_") {
            n
        } else {
            format!("gm_{}", n)
        }
    }

    pub fn effective_category(&self) -> String {
        sanitize_safe_category(&self.category)
    }

    pub fn effective_lang(&self) -> &'static str {
        if let Some(ref l) = self.lang {
            if l.trim().eq_ignore_ascii_case("ru") {
                return "ru";
            }
            if l.trim().eq_ignore_ascii_case("en") {
                return "en";
            }
        }
        detect_os_lang()
    }

    pub fn effective_scale(&self, config_scale: f64) -> f64 {
        self.scale.unwrap_or(config_scale)
    }

    pub fn should_launch_gui(&self, raw_arg_count: usize) -> bool {
        if self.detect_tools || self.check_env || self.list_instances {
            return false;
        }
        self.gui
            || self.auto_start
            || (raw_arg_count <= 1 && self.world.is_none() && self.coords.is_empty())
    }
}
