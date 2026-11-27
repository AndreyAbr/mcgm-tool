use crate::{run_export_pipeline, PipelineOptions};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows_sys::Win32::Foundation::{
    GlobalFree, HGLOBAL, HWND, LPARAM, LRESULT, RECT, S_FALSE, S_OK, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, FillRect, GetDeviceCaps, GetStockObject,
    InvalidateRect, RedrawWindow, SetBkColor, SetBkMode, SetTextColor, COLOR_3DFACE,
    DEFAULT_GUI_FONT, FF_MODERN, FIXED_PITCH, HDC, LOGPIXELSX, NULL_BRUSH, OPAQUE,
    RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX,
    PBM_SETPOS, PBM_SETRANGE32,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListW, ShellExecuteW, BIF_NEWDIALOGSTYLE,
    BIF_RETURNONLYFSDIRS, BROWSEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[link(name = "kernel32")]
extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}

#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(hwnd_new_owner: HWND) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(u_format: u32, h_mem: HGLOBAL) -> HGLOBAL;
    fn CloseClipboard() -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: HWND,
        dw_attribute: u32,
        pv_attribute: *const std::ffi::c_void,
        cb_attribute: u32,
    ) -> i32;
}

#[link(name = "uxtheme")]
extern "system" {
    fn SetWindowTheme(
        hwnd: HWND,
        psz_sub_app_name: *const u16,
        psz_sub_id_list: *const u16,
    ) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn SHCreateItemFromParsingName(
        psz_path: *const u16,
        pbc: *mut std::ffi::c_void,
        riid: *const windows_sys::core::GUID,
        ppv: *mut *mut std::ffi::c_void,
    ) -> i32;
}

const EM_SETSEL: u32 = 0x00B1;
const EM_SCROLLCARET: u32 = 0x00B7;
const EM_LINEINDEX: u32 = 0x00BB;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const EM_LINEFROMCHAR: u32 = 0x00C9;
const EM_SETMARGINS: u32 = 0x00D3;
const EC_LEFTMARGIN: usize = 0x0001;
const EC_RIGHTMARGIN: usize = 0x0002;
const CF_UNICODETEXT: u32 = 13;
const WM_DPICHANGED_MSG: u32 = 0x02E0;
const BM_GETCHECK_MSG: u32 = 0x00F0;
const BM_SETCHECK_MSG: u32 = 0x00F1;
const BST_UNCHECKED_VAL: usize = 0;
const BST_CHECKED_VAL: usize = 1;

const RESERVED_DOS_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

// Control IDs
pub const ID_EDIT_WORLD: i32 = 1001;
pub const ID_BTN_BROWSE_WORLD: i32 = 1002;
pub const ID_COMBO_WORLDS: i32 = 1003;
pub const ID_EDIT_COORD_X1: i32 = 1005;
pub const ID_EDIT_COORD_Y1: i32 = 1051;
pub const ID_EDIT_COORD_Z1: i32 = 1052;
pub const ID_EDIT_COORD_X2: i32 = 1053;
pub const ID_EDIT_COORD_Y2: i32 = 1054;
pub const ID_EDIT_COORD_Z2: i32 = 1055;
pub const ID_EDIT_NAME: i32 = 1010;
pub const ID_EDIT_CAT: i32 = 1011;
pub const ID_EDIT_MAP: i32 = 1012;
pub const ID_EDIT_OUTDIR: i32 = 1013;
pub const ID_BTN_BROWSE_OUT: i32 = 1014;
pub const ID_COMBO_LIGHT: i32 = 1020;
pub const ID_CHK_IMPORT_GMOD: i32 = 1021;
pub const ID_CHK_EXPORT_VMF: i32 = 1022;
pub const ID_CHK_HQ_LIGHT: i32 = 1023;
pub const ID_CHK_FAST_VRAD: i32 = 1024;
pub const ID_CHK_MC_PHYSICS: i32 = 1025;
pub const ID_CHK_VMF_BRUSHES: i32 = 1026;
pub const ID_COMBO_DIM: i32 = 1027;
pub const ID_EDIT_SCALE: i32 = 1028;
pub const ID_COMBO_PRESET: i32 = 1029;
pub const ID_PROGRESS: i32 = 1030;
pub const ID_LOG: i32 = 1031;
pub const ID_BTN_START: i32 = 1040;
pub const ID_BTN_LANG: i32 = 1041;
pub const ID_BTN_STOP: i32 = 1042;
pub const ID_BTN_CLEAR: i32 = 1043;
pub const ID_BTN_RAD_32: i32 = 1044;
pub const ID_BTN_RAD_64: i32 = 1045;
pub const ID_BTN_RAD_128: i32 = 1046;
pub const ID_BTN_OPEN_OUT: i32 = 1047;
pub const ID_BTN_COPY_MAP: i32 = 1048;
pub const ID_BTN_LAUNCH_GMOD: i32 = 1049;
pub const ID_BTN_TOGGLE_ADV: i32 = 1056;
pub const ID_BTN_TOGGLE_LOG: i32 = 1057;
pub const ID_COMBO_INSTANCES: i32 = 1058;
pub const ID_BTN_ADD_INSTANCE: i32 = 1059;

pub const ID_GRP_SRC: i32 = 1101;
pub const ID_GRP_META: i32 = 1102;
pub const ID_GRP_OPTS: i32 = 1103;
pub const ID_GRP_ADV: i32 = 1104;
pub const ID_LBL_WORLD: i32 = 1201;
#[allow(dead_code)]
pub const ID_LBL_SAVES: i32 = 1202;
pub const ID_LBL_COORDS_MIN: i32 = 1203;
pub const ID_LBL_COORDS_MAX: i32 = 1204;
pub const ID_LBL_NAME: i32 = 1205;
pub const ID_LBL_CAT: i32 = 1206;
pub const ID_LBL_MAP: i32 = 1207;
pub const ID_LBL_OUTDIR: i32 = 1208;
pub const ID_LBL_LIGHT: i32 = 1209;
pub const ID_LBL_STATUS: i32 = 1210;
pub const ID_LBL_DIM: i32 = 1211;
pub const ID_LBL_SCALE: i32 = 1212;
pub const ID_LBL_PRESET: i32 = 1213;
#[allow(dead_code)]
pub const ID_LBL_RADIUS: i32 = 1214;
pub const ID_LBL_SEL_INFO: i32 = 1215;
pub const ID_LBL_TITLE: i32 = 1216;
#[allow(dead_code)]
pub const ID_LBL_STEP_WORLD: i32 = 1217;
#[allow(dead_code)]
pub const ID_LBL_STEP_AREA: i32 = 1218;
#[allow(dead_code)]
pub const ID_LBL_STEP_MAP: i32 = 1219;
pub const ID_LBL_LOG_HDR: i32 = 1220;
pub const ID_LBL_INSTANCE: i32 = 1221;
pub const ID_LBL_WORLD_ROW: i32 = 1222;

const WM_APP_DONE: u32 = WM_APP + 2;
const WM_APP_AUTOSTART: u32 = WM_APP + 3;
const WM_APP_WORLDS_READY: u32 = WM_APP + 4;
const ID_TIMER_LOG: usize = 1;

pub const GUI_REGULAR_FONT_FACE: &str = "Segoe UI";
#[allow(dead_code)]
pub const GUI_BOLD_FONT_FACE: &str = "Bahnschrift SemiBold";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiLang {
    Ru,
    En,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlVisualKind {
    PrimaryButton,
    DangerButton,
    PillButton,
    SecondaryButton,
    Checkbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiThemePalette {
    pub canvas_bg: u32,
    pub card_bg: u32,
    pub card_border: u32,
    pub input_bg: u32,
    pub input_border: u32,
    pub log_bg: u32,
    pub log_text: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_muted: u32,
    pub accent_text: u32,
    pub primary_bg: u32,
    pub primary_hover: u32,
    pub primary_pressed: u32,
    pub primary_disabled: u32,
    pub primary_text: u32,
    pub danger_bg: u32,
    pub danger_hover: u32,
    pub danger_pressed: u32,
    pub btn_bg: u32,
    pub btn_hover: u32,
    pub btn_pressed: u32,
    pub btn_border: u32,
    pub btn_text: u32,
    pub pill_bg: u32,
    pub pill_hover: u32,
    pub pill_border: u32,
    pub chk_accent: u32,
    pub chk_border: u32,
    pub chk_hover_border: u32,
    pub prog_track: u32,
    pub prog_fill: u32,
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

pub fn light_palette() -> GuiThemePalette {
    GuiThemePalette {
        canvas_bg: rgb(240, 240, 240),
        card_bg: rgb(255, 255, 255),
        card_border: rgb(224, 224, 224),
        input_bg: rgb(255, 255, 255),
        input_border: rgb(204, 204, 204),
        log_bg: rgb(250, 250, 250),
        log_text: rgb(32, 32, 32),
        text_primary: rgb(26, 26, 26),
        text_secondary: rgb(74, 74, 74),
        text_muted: rgb(118, 118, 118),
        accent_text: rgb(26, 26, 26),
        primary_bg: rgb(36, 36, 36),
        primary_hover: rgb(58, 58, 58),
        primary_pressed: rgb(20, 20, 20),
        primary_disabled: rgb(176, 176, 176),
        primary_text: rgb(255, 255, 255),
        danger_bg: rgb(96, 96, 96),
        danger_hover: rgb(72, 72, 72),
        danger_pressed: rgb(52, 52, 52),
        btn_bg: rgb(248, 248, 248),
        btn_hover: rgb(236, 236, 236),
        btn_pressed: rgb(222, 222, 222),
        btn_border: rgb(204, 204, 204),
        btn_text: rgb(28, 28, 28),
        pill_bg: rgb(244, 244, 244),
        pill_hover: rgb(232, 232, 232),
        pill_border: rgb(208, 208, 208),
        chk_accent: rgb(36, 36, 36),
        chk_border: rgb(156, 156, 156),
        chk_hover_border: rgb(72, 72, 72),
        prog_track: rgb(230, 230, 230),
        prog_fill: rgb(44, 44, 44),
    }
}

pub fn scale_px(px: i32, dpi: u32) -> i32 {
    let eff = if dpi == 0 { 96 } else { dpi };
    ((((px as i64) * (eff as i64)) + 48) / 96) as i32
}

#[allow(dead_code)]
pub fn classify_button_kind(id: i32) -> ControlVisualKind {
    match id {
        ID_BTN_START => ControlVisualKind::PrimaryButton,
        ID_BTN_STOP => ControlVisualKind::DangerButton,
        ID_BTN_RAD_32 | ID_BTN_RAD_64 | ID_BTN_RAD_128 => ControlVisualKind::PillButton,
        ID_CHK_IMPORT_GMOD
        | ID_CHK_EXPORT_VMF
        | ID_CHK_HQ_LIGHT
        | ID_CHK_FAST_VRAD
        | ID_CHK_MC_PHYSICS
        | ID_CHK_VMF_BRUSHES => ControlVisualKind::Checkbox,
        _ => ControlVisualKind::SecondaryButton,
    }
}

pub fn compute_card_rects(
    client_w: i32,
    mode: GuiLayoutMode,
    dpi: u32,
) -> Vec<(i32, i32, i32, i32)> {
    let sc = |v: i32| scale_px(v, dpi);
    let left = sc(10);
    let right = (client_w - sc(10)).max(left + sc(40));
    let mut cards = vec![
        (left, sc(34), right, sc(108)),
        (left, sc(112), right, sc(230)),
        (left, sc(234), right, sc(362)),
    ];
    if mode.show_advanced {
        cards.push((left, sc(488), right, sc(714)));
    }
    cards
}

fn get_window_dpi(hwnd: HWND) -> u32 {
    if !hwnd.is_null() {
        unsafe {
            let user32 = to_wide("user32.dll");
            let hmod = GetModuleHandleW(user32.as_ptr());
            if !hmod.is_null() {
                if let Some(proc_addr) =
                    GetProcAddress(hmod, c"GetDpiForWindow".as_ptr() as *const u8)
                {
                    let f: unsafe extern "system" fn(HWND) -> u32 = std::mem::transmute(proc_addr);
                    let d = f(hwnd);
                    if d >= 72 {
                        return d;
                    }
                }
            }
            let hdc = windows_sys::Win32::Graphics::Gdi::GetDC(hwnd);
            if !hdc.is_null() {
                let d = GetDeviceCaps(hdc, LOGPIXELSX as i32) as u32;
                windows_sys::Win32::Graphics::Gdi::ReleaseDC(hwnd, hdc);
                if d >= 72 {
                    return d;
                }
            }
        }
    }
    96
}

#[derive(Debug, Clone, Default)]
pub struct GuiLaunchConfig {
    pub world: Option<PathBuf>,
    pub coords: Option<crate::anvil::WorldCoords>,
    pub dimension: Option<String>,
    pub name: Option<String>,
    pub category: Option<String>,
    pub map_name: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub lighting_style: Option<String>,
    pub scale: Option<f64>,
    pub mc_physics: Option<bool>,
    pub hq_light: Option<bool>,
    pub fast_vrad: Option<bool>,
    pub export_vmf: Option<bool>,
    pub import_to_gmod: Option<bool>,
    pub force_vmf_brushes: Option<bool>,
    pub chunk_size: Option<usize>,
    pub max_workers: Option<usize>,
    pub max_convex: Option<usize>,
    pub reuse_cache: Option<bool>,
    pub debug_log: Option<PathBuf>,
    pub startup_error: Option<String>,
    pub auto_start: bool,
    pub lang: Option<UiLang>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuiLayoutMode {
    pub show_advanced: bool,
    pub show_log: bool,
}

pub fn compute_min_client_size(mode: GuiLayoutMode) -> (i32, i32) {
    let min_w = if mode.show_advanced { 660 } else { 620 };
    let base_h = 492;
    let adv_h = if mode.show_advanced { 234 } else { 0 };
    let log_h = if mode.show_log { 160 } else { 0 };
    (min_w, base_h + adv_h + log_h)
}

pub fn compute_default_client_size(mode: GuiLayoutMode) -> (i32, i32) {
    let def_w = if mode.show_advanced { 680 } else { 640 };
    let base_h = 492;
    let adv_h = if mode.show_advanced { 234 } else { 0 };
    let log_h = if mode.show_log { 196 } else { 0 };
    (def_w, base_h + adv_h + log_h)
}

pub const ADVANCED_ONLY_CONTROLS: &[i32] = &[
    ID_GRP_ADV,
    ID_LBL_WORLD,
    ID_EDIT_WORLD,
    ID_LBL_DIM,
    ID_COMBO_DIM,
    ID_LBL_SCALE,
    ID_EDIT_SCALE,
    ID_LBL_NAME,
    ID_EDIT_NAME,
    ID_LBL_CAT,
    ID_EDIT_CAT,
    ID_CHK_MC_PHYSICS,
    ID_CHK_EXPORT_VMF,
    ID_CHK_HQ_LIGHT,
    ID_CHK_FAST_VRAD,
    ID_CHK_VMF_BRUSHES,
    ID_BTN_COPY_MAP,
];

#[allow(dead_code)]
pub fn is_advanced_only_control(id: i32) -> bool {
    ADVANCED_ONLY_CONTROLS.contains(&id)
}

pub fn derive_model_name_from_map(map_name: &str) -> String {
    let trimmed = map_name.trim();
    let stem = if trimmed.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("gm_")) {
        &trimmed[3..]
    } else {
        trimmed
    };
    sanitize_identifier(stem, "mc_export")
}

pub fn should_auto_replace_map_name(current: &str, last_auto: Option<&str>) -> bool {
    let cur = current.trim();
    if cur.is_empty() || matches!(cur, "gm_mc_custom" | "gm_mc_export" | "gm_minecraft") {
        return true;
    }
    last_auto.is_some_and(|la| cur == la)
}

pub fn merge_discovered_world(
    discovered: &[(String, PathBuf)],
    current_world: Option<&Path>,
    lang: UiLang,
) -> (Vec<(String, PathBuf)>, Option<usize>) {
    let mut list = discovered.to_vec();
    if let Some(p) = current_world.filter(|p| !p.as_os_str().is_empty()) {
        let p_low = p.to_string_lossy().to_lowercase();
        if let Some(idx) = list
            .iter()
            .position(|(_, d)| d.to_string_lossy().to_lowercase() == p_low)
        {
            return (list, Some(idx));
        }
        let folder = p
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| p.to_string_lossy().into_owned());
        let label = format!("{} ({})", folder, tr_text(lang, "world_custom"));
        list.insert(0, (label, p.to_path_buf()));
        return (list, Some(0));
    }
    if !list.is_empty() {
        (list, Some(0))
    } else {
        (list, None)
    }
}

pub fn build_instance_combo_items(
    instances: &[crate::instances::MinecraftInstance],
    lang: UiLang,
) -> Vec<String> {
    let mut items = Vec::with_capacity(instances.len() + 1);
    items.push(format!("{} ({})", tr_text(lang, "instances_all"), instances.len()));
    for inst in instances {
        items.push(inst.display_label());
    }
    items
}

pub fn worlds_for_instance_selection(
    instances: &[crate::instances::MinecraftInstance],
    selected_instance: Option<usize>,
) -> Vec<(String, PathBuf)> {
    if let Some(idx) = selected_instance {
        if let Some(inst) = instances.get(idx) {
            return inst
                .worlds
                .iter()
                .map(|w| (w.name.clone(), w.path.clone()))
                .collect();
        }
    }
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for inst in instances {
        let tag = inst.short_tag();
        for w in &inst.worlds {
            let key = w.path.to_string_lossy().to_lowercase();
            if seen.insert(key) {
                out.push((format!("{} [{}]", w.name, tag), w.path.clone()));
            }
        }
    }
    out
}

pub fn resolve_browse_initial_dir(
    instances: &[crate::instances::MinecraftInstance],
    selected_instance: Option<usize>,
    current_world: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(idx) = selected_instance {
        if let Some(inst) = instances.get(idx) {
            return Some(crate::instances::strip_windows_verbatim_prefix(&inst.saves_dir));
        }
    }
    if let Some(parent) = current_world
        .filter(|p| !p.as_os_str().is_empty())
        .and_then(|p| p.parent())
    {
        return Some(crate::instances::strip_windows_verbatim_prefix(parent));
    }
    instances
        .first()
        .map(|inst| crate::instances::strip_windows_verbatim_prefix(&inst.saves_dir))
}

pub fn format_selection_summary(lang: UiLang, coords: Option<[i32; 6]>) -> String {
    let Some([x1, y1, z1, x2, y2, z2]) = coords else {
        return tr_text(lang, "sel_empty").to_string();
    };
    let dx = (x2 - x1).unsigned_abs() as u64 + 1;
    let dy = (y2 - y1).unsigned_abs() as u64 + 1;
    let dz = (z2 - z1).unsigned_abs() as u64 + 1;
    match lang {
        UiLang::Ru => format!("{} × {} × {} блоков", dx, dy, dz),
        UiLang::En => format!("{} × {} × {} blocks", dx, dy, dz),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct PersistedGuiState {
    pub(crate) lang: Option<String>,
    pub(crate) world_dir: Option<String>,
    pub(crate) min_x: Option<i32>,
    pub(crate) min_y: Option<i32>,
    pub(crate) min_z: Option<i32>,
    pub(crate) max_x: Option<i32>,
    pub(crate) max_y: Option<i32>,
    pub(crate) max_z: Option<i32>,
    pub(crate) dimension: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) map_name: Option<String>,
    pub(crate) output_dir: Option<String>,
    pub(crate) lighting_style: Option<String>,
    pub(crate) scale: Option<String>,
    pub(crate) build_preset: Option<String>,
    pub(crate) import_to_gmod: Option<bool>,
    pub(crate) export_vmf: Option<bool>,
    pub(crate) hq_light: Option<bool>,
    pub(crate) fast_vrad: Option<bool>,
    pub(crate) mc_physics: Option<bool>,
    pub(crate) force_vmf_brushes: Option<bool>,
    #[serde(default)]
    pub(crate) show_advanced: Option<bool>,
    #[serde(default)]
    pub(crate) show_log: Option<bool>,
    #[serde(default)]
    pub(crate) selected_instance_saves: Option<String>,
    #[serde(default)]
    pub(crate) custom_instance_dirs: Vec<String>,
}

pub fn detect_default_language() -> UiLang {
    for key in ["MCGM_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = std::env::var(key) {
            let low = val.trim().to_lowercase();
            if low.starts_with("ru") {
                return UiLang::Ru;
            }
            if low.starts_with("en") || low == "c" || low.starts_with("c.") || low.starts_with("c_") {
                return UiLang::En;
            }
        }
    }
    let lang_id = unsafe { GetUserDefaultUILanguage() };
    if (lang_id & 0x03FF) == 0x19 {
        UiLang::Ru
    } else {
        UiLang::En
    }
}

pub fn tr_text(lang: UiLang, key: &str) -> &'static str {
    match (lang, key) {
        (UiLang::Ru, "win_title") => "MCGM",
        (UiLang::En, "win_title") => "MCGM",
        (UiLang::Ru, "ui_title") => "Конвертер",
        (UiLang::En, "ui_title") => "Converter",
        (UiLang::Ru, "step_world") => "1. Мир",
        (UiLang::En, "step_world") => "1. World",
        (UiLang::Ru, "step_area") => "2. Область",
        (UiLang::En, "step_area") => "2. Area",
        (UiLang::Ru, "step_map") => "3. Карта",
        (UiLang::En, "step_map") => "3. Map",
        (UiLang::Ru, "rad_small") => "±32",
        (UiLang::En, "rad_small") => "±32",
        (UiLang::Ru, "rad_medium") => "±64",
        (UiLang::En, "rad_medium") => "±64",
        (UiLang::Ru, "rad_large") => "±128",
        (UiLang::En, "rad_large") => "±128",
        (UiLang::Ru, "lbl_preset") => "Качество:",
        (UiLang::En, "lbl_preset") => "Quality:",
        (UiLang::Ru, "preset_preview") => "Быстро",
        (UiLang::En, "preset_preview") => "Fast",
        (UiLang::Ru, "preset_balanced") => "Стандарт",
        (UiLang::En, "preset_balanced") => "Standard",
        (UiLang::Ru, "preset_hq") => "Высокое",
        (UiLang::En, "preset_hq") => "High",
        (UiLang::Ru, "preset_custom") => "Своё",
        (UiLang::En, "preset_custom") => "Custom",
        (UiLang::Ru, "grp_src") => "Исходный мир и координаты",
        (UiLang::En, "grp_src") => "Source world and coordinates",
        (UiLang::Ru, "grp_meta") => "Настройки модели и карты",
        (UiLang::En, "grp_meta") => "Model and map settings",
        (UiLang::Ru, "grp_opts") => "Параметры компиляции и физики",
        (UiLang::En, "grp_opts") => "Build and physics options",
        (UiLang::Ru, "grp_adv") => "Расширенные настройки",
        (UiLang::En, "grp_adv") => "Advanced settings",
        (UiLang::Ru, "lbl_log_hdr") => "Журнал",
        (UiLang::En, "lbl_log_hdr") => "Log",
        (UiLang::Ru, "lbl_world") => "Папка мира:",
        (UiLang::En, "lbl_world") => "World folder:",
        (UiLang::Ru, "lbl_saves") => "Найдено:",
        (UiLang::En, "lbl_saves") => "Saved:",
        (UiLang::Ru, "lbl_min") => "Мин (X, Y, Z):",
        (UiLang::En, "lbl_min") => "Min (X, Y, Z):",
        (UiLang::Ru, "lbl_max") => "Макс (X, Y, Z):",
        (UiLang::En, "lbl_max") => "Max (X, Y, Z):",
        (UiLang::Ru, "lbl_dim") => "Измерение:",
        (UiLang::En, "lbl_dim") => "Dimension:",
        (UiLang::Ru, "dim_overworld") => "Обычный мир",
        (UiLang::En, "dim_overworld") => "Overworld",
        (UiLang::Ru, "dim_nether") => "Незер",
        (UiLang::En, "dim_nether") => "Nether",
        (UiLang::Ru, "dim_end") => "Энд",
        (UiLang::En, "dim_end") => "End",
        (UiLang::Ru, "lbl_scale") => "Масштаб:",
        (UiLang::En, "lbl_scale") => "Scale:",
        (UiLang::Ru, "lbl_radius") => "Радиус:",
        (UiLang::En, "lbl_radius") => "Radius:",
        (UiLang::Ru, "lbl_name") => "Имя модели:",
        (UiLang::En, "lbl_name") => "Model name:",
        (UiLang::Ru, "lbl_cat") => "Категория:",
        (UiLang::En, "lbl_cat") => "Category:",
        (UiLang::Ru, "lbl_map") => "Имя карты:",
        (UiLang::En, "lbl_map") => "Map name:",
        (UiLang::Ru, "lbl_outdir") => "Куда сохранить:",
        (UiLang::En, "lbl_outdir") => "Save to:",
        (UiLang::Ru, "lbl_light") => "Свет:",
        (UiLang::En, "lbl_light") => "Lighting:",
        (UiLang::Ru, "light_minecraft") => "День",
        (UiLang::En, "light_minecraft") => "Day",
        (UiLang::Ru, "light_gmod") => "Реалистичный",
        (UiLang::En, "light_gmod") => "Realistic",
        (UiLang::Ru, "light_bright") => "Яркий",
        (UiLang::En, "light_bright") => "Bright",
        (UiLang::Ru, "light_night") => "Ночь",
        (UiLang::En, "light_night") => "Night",
        (UiLang::Ru, "chk_import") => "Импортировать напрямую в Garry's Mod",
        (UiLang::En, "chk_import") => "Import directly to Garry's Mod",
        (UiLang::Ru, "chk_vmf") => "Сохранить исходник карты (.vmf) для Hammer",
        (UiLang::En, "chk_vmf") => "Save map source (.vmf) for Hammer",
        (UiLang::Ru, "chk_hq") => "Высокое качество света",
        (UiLang::En, "chk_hq") => "High quality lighting",
        (UiLang::Ru, "chk_fast") => "Быстрый свет (-fast)",
        (UiLang::En, "chk_fast") => "Fast lighting (-fast)",
        (UiLang::Ru, "chk_phys") => "Физика Minecraft",
        (UiLang::En, "chk_phys") => "Minecraft physics",
        (UiLang::Ru, "chk_brushes") => "Коллизия в VMF",
        (UiLang::En, "chk_brushes") => "VMF brush collision",
        (UiLang::Ru, "btn_browse") => "Обзор...",
        (UiLang::En, "btn_browse") => "Browse...",
        (UiLang::Ru, "btn_start") => "Создать карту",
        (UiLang::En, "btn_start") => "Create map",
        (UiLang::Ru, "btn_running") => "Создание...",
        (UiLang::En, "btn_running") => "Creating...",
        (UiLang::Ru, "btn_stop") => "Стоп",
        (UiLang::En, "btn_stop") => "Stop",
        (UiLang::Ru, "btn_clear") => "Очистить",
        (UiLang::En, "btn_clear") => "Clear",
        (UiLang::Ru, "btn_open_out") => "Папка",
        (UiLang::En, "btn_open_out") => "Folder",
        (UiLang::Ru, "btn_copy_map") => "Копировать map",
        (UiLang::En, "btn_copy_map") => "Copy map cmd",
        (UiLang::Ru, "btn_launch_gmod") => "Открыть GMod",
        (UiLang::En, "btn_launch_gmod") => "Launch GMod",
        (UiLang::Ru, "btn_adv_show") => "▸ Дополнительно",
        (UiLang::En, "btn_adv_show") => "▸ Advanced",
        (UiLang::Ru, "btn_adv_hide") => "▾ Дополнительно",
        (UiLang::En, "btn_adv_hide") => "▾ Advanced",
        (UiLang::Ru, "btn_log_show") => "▸ Журнал",
        (UiLang::En, "btn_log_show") => "▸ Log",
        (UiLang::Ru, "btn_log_hide") => "▾ Журнал",
        (UiLang::En, "btn_log_hide") => "▾ Log",
        (UiLang::Ru, "btn_lang") => "RU / EN",
        (UiLang::En, "btn_lang") => "EN / RU",
        (UiLang::Ru, "status_ready") => "Готово",
        (UiLang::En, "status_ready") => "Ready",
        (UiLang::Ru, "status_running") => "Экспорт...",
        (UiLang::En, "status_running") => "Exporting...",
        (UiLang::Ru, "status_cancelling") => "Отмена...",
        (UiLang::En, "status_cancelling") => "Cancelling...",
        (UiLang::Ru, "status_cancelled") => "Отменено",
        (UiLang::En, "status_cancelled") => "Cancelled",
        (UiLang::Ru, "status_done") => "Готово!",
        (UiLang::En, "status_done") => "Done!",
        (UiLang::Ru, "status_partial") => "Готово (без GMA)",
        (UiLang::En, "status_partial") => "Done (GMA failed)",
        (UiLang::Ru, "status_failed") => "Ошибка",
        (UiLang::En, "status_failed") => "Error",
        (UiLang::Ru, "browse_world") => "Выберите папку сохранения мира Minecraft",
        (UiLang::En, "browse_world") => "Select Minecraft world save directory",
        (UiLang::Ru, "browse_out") => "Выберите папку вывода",
        (UiLang::En, "browse_out") => "Select output directory",
        (UiLang::Ru, "err_title") => "Ошибка проверки данных",
        (UiLang::En, "err_title") => "Validation error",
        (UiLang::Ru, "export_err_title") => "Ошибка экспорта",
        (UiLang::En, "export_err_title") => "Export error",
        (UiLang::Ru, "done_title") => "Экспорт завершён",
        (UiLang::En, "done_title") => "Export complete",
        (UiLang::Ru, "partial_title") => "Экспорт завершён частично",
        (UiLang::En, "partial_title") => "Export partially complete",
        (UiLang::Ru, "confirm_close_title") => "Идёт экспорт",
        (UiLang::En, "confirm_close_title") => "Export in progress",
        (UiLang::Ru, "confirm_close_msg") => "Экспорт ещё выполняется. Прервать и закрыть окно?",
        (UiLang::En, "confirm_close_msg") => "An export is currently running. Cancel and close the window?",
        (UiLang::Ru, "sel_empty") => "Выберите область",
        (UiLang::En, "sel_empty") => "Select an area",
        (UiLang::Ru, "world_custom") => "выбрано вручную",
        (UiLang::En, "world_custom") => "custom folder",
        (UiLang::Ru, "worlds_none") => "Миры не найдены",
        (UiLang::En, "worlds_none") => "No worlds found",
        (UiLang::Ru, "lbl_instance") => "Инстанс:",
        (UiLang::En, "lbl_instance") => "Instance:",
        (UiLang::Ru, "lbl_world_row") => "Мир:",
        (UiLang::En, "lbl_world_row") => "World:",
        (UiLang::Ru, "btn_add_instance") => "+ Папка...",
        (UiLang::En, "btn_add_instance") => "+ Folder...",
        (UiLang::Ru, "instances_all") => "Все инстансы",
        (UiLang::En, "instances_all") => "All instances",
        (UiLang::Ru, "browse_instance") => "Выберите папку лаунчера (Prism / MultiMC / .minecraft) или instances",
        (UiLang::En, "browse_instance") => "Select launcher folder (Prism / MultiMC / .minecraft) or instances",
        (UiLang::Ru, "worlds_none_in_instance") => "Нет миров",
        (UiLang::En, "worlds_none_in_instance") => "No worlds",
        (UiLang::Ru, "instances_added_none") => "В выбранной папке инстансы Minecraft не найдены.",
        (UiLang::En, "instances_added_none") => "No Minecraft instances found in the selected folder.",
        _ => "",
    }
}

struct GuiHandles {
    hwnd: isize,
    controls: HashMap<i32, isize>,
}

#[derive(Default)]
struct ThemeBrushes {
    canvas: isize,
    input: isize,
    log: isize,
}

impl ThemeBrushes {
    fn create(pal: &GuiThemePalette) -> Self {
        unsafe {
            Self {
                canvas: CreateSolidBrush(pal.canvas_bg) as isize,
                input: CreateSolidBrush(pal.input_bg) as isize,
                log: CreateSolidBrush(pal.log_bg) as isize,
            }
        }
    }

    fn destroy(&mut self) {
        unsafe {
            for b in [&mut self.canvas, &mut self.input, &mut self.log] {
                if *b != 0 {
                    DeleteObject(*b as _);
                    *b = 0;
                }
            }
        }
    }
}

#[derive(Default, Clone, Copy)]
struct ThemeFonts {
    regular: isize,
    title: isize,
    bold: isize,
    log: isize,
}

impl ThemeFonts {
    fn create(dpi: u32) -> Self {
        let face = to_wide(GUI_REGULAR_FONT_FACE);
        let log_face = to_wide("Consolas");
        unsafe {
            let reg = CreateFontW(
                -scale_px(12, dpi),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                face.as_ptr(),
            );
            let eff_reg = if !reg.is_null() {
                reg as isize
            } else {
                GetStockObject(DEFAULT_GUI_FONT) as isize
            };
            let title = CreateFontW(
                -scale_px(14, dpi),
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                face.as_ptr(),
            );
            let bold = CreateFontW(
                -scale_px(12, dpi),
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                face.as_ptr(),
            );
            let log = CreateFontW(
                -scale_px(12, dpi),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                (FIXED_PITCH | FF_MODERN) as u32,
                log_face.as_ptr(),
            );
            Self {
                regular: eff_reg,
                title: if !title.is_null() { title as isize } else { eff_reg },
                bold: if !bold.is_null() { bold as isize } else { eff_reg },
                log: if !log.is_null() { log as isize } else { eff_reg },
            }
        }
    }

    fn destroy(&mut self) {
        unsafe {
            let def = GetStockObject(DEFAULT_GUI_FONT) as isize;
            for f in [&mut self.regular, &mut self.title, &mut self.bold, &mut self.log] {
                if *f != 0 && *f != def {
                    DeleteObject(*f as _);
                }
                *f = 0;
            }
        }
    }
}

struct GuiState {
    lang: UiLang,
    dpi: u32,
    brushes: ThemeBrushes,
    fonts: ThemeFonts,
    busy: bool,
    cancel_flag: Arc<AtomicBool>,
    discovered_instances: Vec<crate::instances::MinecraftInstance>,
    selected_instance_idx: Option<usize>,
    custom_instance_dirs: Vec<PathBuf>,
    pending_saved_instance_saves: Option<String>,
    discovered_worlds: Vec<(String, PathBuf)>,
    anchor_center: Option<[i32; 3]>,
    build_preset: String,
    status_key: &'static str,
    custom_status_text: Option<String>,
    config_path: Option<PathBuf>,
    syncing_preset: bool,
    suppress_world_combo: bool,
    suppress_instance_combo: bool,
    layout_mode: GuiLayoutMode,
    last_auto_map_name: Option<String>,
    model_name_manually_edited: bool,
    suppress_name_sync: bool,
    cli_chunk_size: Option<usize>,
    cli_max_workers: Option<usize>,
    cli_max_convex: Option<usize>,
    cli_reuse_cache: Option<bool>,
    cli_debug_log: Option<PathBuf>,
    custom_dimension: Option<String>,
}

type PendingLogBatch = (i32, Option<String>, Vec<String>);
static GUI_HANDLES: OnceLock<GuiHandles> = OnceLock::new();
static GUI_STATE: OnceLock<Mutex<GuiState>> = OnceLock::new();
static PENDING_LOG_BATCH: OnceLock<Mutex<PendingLogBatch>> = OnceLock::new();

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn get_ctrl_hwnd(id: i32) -> Option<isize> {
    GUI_HANDLES.get().and_then(|h| h.controls.get(&id).copied())
}

fn get_ctrl_text(hwnd: isize) -> String {
    if hwnd == 0 {
        return String::new();
    }
    unsafe {
        let len = GetWindowTextLengthW(hwnd as HWND);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len as usize) + 1];
        let copied = GetWindowTextW(hwnd as HWND, buf.as_mut_ptr(), buf.len() as i32);
        if copied <= 0 {
            return String::new();
        }
        from_wide(&buf[..copied as usize])
    }
}

fn set_ctrl_text(hwnd: isize, text: &str) {
    if hwnd == 0 {
        return;
    }
    let w = to_wide(text);
    unsafe {
        SetWindowTextW(hwnd as HWND, w.as_ptr());
    }
}

fn get_chk_state(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    unsafe { SendMessageW(hwnd as HWND, BM_GETCHECK_MSG, 0, 0) == BST_CHECKED_VAL as isize }
}

fn set_chk_state(hwnd: isize, checked: bool) {
    if hwnd == 0 {
        return;
    }
    unsafe {
        SendMessageW(
            hwnd as HWND,
            BM_SETCHECK_MSG,
            if checked { BST_CHECKED_VAL } else { BST_UNCHECKED_VAL },
            0,
        );
    }
}

fn append_log_line(log_hwnd: isize, line: &str) {
    if log_hwnd == 0 {
        return;
    }
    let clean = line.trim_end_matches(&['\r', '\n'][..]);
    let normalized = clean.replace("\r\n", "\n").replace('\n', "\r\n");
    append_log_normalized(log_hwnd, &normalized);
}

fn append_log_normalized(log_hwnd: isize, normalized: &str) {
    if log_hwnd == 0 || normalized.is_empty() {
        return;
    }
    unsafe {
        let cur_len = GetWindowTextLengthW(log_hwnd as HWND);
        if cur_len > 400_000 {
            let line_idx = SendMessageW(log_hwnd as HWND, EM_LINEFROMCHAR, 100_000, 0) + 1;
            let char_idx = SendMessageW(log_hwnd as HWND, EM_LINEINDEX, line_idx as usize, 0);
            if char_idx > 0 {
                SendMessageW(log_hwnd as HWND, EM_SETSEL, 0, char_idx);
                let empty = to_wide("");
                SendMessageW(log_hwnd as HWND, EM_REPLACESEL, 0, empty.as_ptr() as isize);
            }
        }
        let cur_len_after = GetWindowTextLengthW(log_hwnd as HWND);
        let to_append = if cur_len_after > 0 {
            format!("\r\n{}", normalized)
        } else {
            normalized.to_string()
        };
        let end_pos = cur_len_after.max(0) as usize;
        SendMessageW(log_hwnd as HWND, EM_SETSEL, end_pos, end_pos as isize);
        let w = to_wide(&to_append);
        SendMessageW(log_hwnd as HWND, EM_REPLACESEL, 0, w.as_ptr() as isize);
        SendMessageW(log_hwnd as HWND, EM_SCROLLCARET, 0, 0);
    }
}

fn flush_pending_logs() {
    let Some(batch) = PENDING_LOG_BATCH.get() else {
        return;
    };
    let (pct, status_opt, lines) = {
        let Ok(mut b) = batch.lock() else {
            return;
        };
        if b.2.is_empty() && b.1.is_none() {
            return;
        }
        let pct = b.0;
        let status = b.1.take();
        let lines = std::mem::take(&mut b.2);
        (pct, status, lines)
    };
    if let Some(ref text) = status_opt {
        if let Some(lock) = GUI_STATE.get() {
            if let Ok(mut st) = lock.lock() {
                st.custom_status_text = Some(text.clone());
            }
        }
        if let Some(hp) = get_ctrl_hwnd(ID_PROGRESS) {
            unsafe {
                SendMessageW(hp as HWND, PBM_SETPOS, pct.clamp(0, 100) as usize, 0);
            }
        }
        if let Some(hl) = get_ctrl_hwnd(ID_LBL_STATUS) {
            set_ctrl_text(hl, text);
        }
    }
    if !lines.is_empty() {
        if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
            let joined = lines
                .iter()
                .map(|l| {
                    l.trim_end_matches(&['\r', '\n'][..])
                        .replace("\r\n", "\n")
                        .replace('\n', "\r\n")
                })
                .collect::<Vec<_>>()
                .join("\r\n");
            append_log_normalized(hlog, &joined);
        }
    }
}

fn find_config_json(override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = override_path {
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    let cwd_cfg = PathBuf::from("config.json");
    if cwd_cfg.is_file() {
        return Some(cwd_cfg);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.json")))
        .filter(|p| p.is_file())
}

fn resolve_gui_state_load_path() -> Option<PathBuf> {
    if let Some(exe_state) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("gui_state.json")))
    {
        if exe_state.is_file() {
            return Some(exe_state);
        }
    }
    let cwd_state = PathBuf::from("gui_state.json");
    if cwd_state.is_file() {
        return Some(cwd_state);
    }
    None
}

fn resolve_gui_state_save_path() -> PathBuf {
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        if exe_dir.is_dir() {
            return exe_dir.join("gui_state.json");
        }
    }
    PathBuf::from("gui_state.json")
}

unsafe extern "system" fn browse_callback(
    hwnd: HWND,
    umsg: u32,
    _lparam: LPARAM,
    lpdata: LPARAM,
) -> i32 {
    const BFFM_INITIALIZED: u32 = 1;
    const BFFM_SETSELECTIONW: u32 = WM_USER + 103;
    if umsg == BFFM_INITIALIZED && lpdata != 0 {
        SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, lpdata);
    }
    0
}

fn browse_folder_modern(
    owner: HWND,
    title: &str,
    initial_dir: Option<&Path>,
) -> Result<Option<PathBuf>, ()> {
    const CLSID_FILE_OPEN_DIALOG: windows_sys::core::GUID =
        windows_sys::core::GUID::from_u128(0xdc1c5a9c_e88a_4dde_a5a1_60f82a20aef7);
    const IID_I_FILE_OPEN_DIALOG: windows_sys::core::GUID =
        windows_sys::core::GUID::from_u128(0xd57c7288_d4ad_4768_be02_9d969532d960);
    const IID_I_SHELL_ITEM: windows_sys::core::GUID =
        windows_sys::core::GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
    const FOS_PICKFOLDERS: u32 = 0x0000_0020;
    const FOS_FORCEFILESYSTEM: u32 = 0x0000_0040;
    const FOS_PATHMUSTEXIST: u32 = 0x0000_0800;
    const SIGDN_FILESYSPATH: u32 = 0x8005_8000;
    const HRESULT_CANCELLED: i32 = 0x8007_04C7u32 as i32;

    #[repr(C)]
    struct ComObj {
        vtbl: *const *const std::ffi::c_void,
    }

    unsafe {
        let hr_init = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let should_uninit = hr_init == S_OK || hr_init == S_FALSE;
        let mut dlg_raw: *mut std::ffi::c_void = std::ptr::null_mut();
        let hr_create = CoCreateInstance(
            &CLSID_FILE_OPEN_DIALOG,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_I_FILE_OPEN_DIALOG,
            &mut dlg_raw,
        );
        if hr_create < 0 || dlg_raw.is_null() {
            if should_uninit {
                CoUninitialize();
            }
            return Err(());
        }

        let dlg = dlg_raw as *mut ComObj;
        let vtbl = (*dlg).vtbl;

        let release_fn: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
            std::mem::transmute(*vtbl.add(2));
        let show_fn: unsafe extern "system" fn(*mut std::ffi::c_void, HWND) -> i32 =
            std::mem::transmute(*vtbl.add(3));
        let set_options_fn: unsafe extern "system" fn(*mut std::ffi::c_void, u32) -> i32 =
            std::mem::transmute(*vtbl.add(9));
        let get_options_fn: unsafe extern "system" fn(*mut std::ffi::c_void, *mut u32) -> i32 =
            std::mem::transmute(*vtbl.add(10));
        let set_folder_fn: unsafe extern "system" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32 =
            std::mem::transmute(*vtbl.add(12));
        let set_title_fn: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16) -> i32 =
            std::mem::transmute(*vtbl.add(17));
        let get_result_fn: unsafe extern "system" fn(*mut std::ffi::c_void, *mut *mut std::ffi::c_void) -> i32 =
            std::mem::transmute(*vtbl.add(20));

        let mut opts: u32 = 0;
        if get_options_fn(dlg_raw, &mut opts) >= 0 {
            let _ = set_options_fn(
                dlg_raw,
                opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST,
            );
        }

        let wtitle = to_wide(title);
        let _ = set_title_fn(dlg_raw, wtitle.as_ptr());

        if let Some(init) = initial_dir.filter(|p| p.is_dir()) {
            let clean = crate::instances::strip_windows_verbatim_prefix(init);
            let winit = to_wide(&clean.to_string_lossy());
            let mut item_raw: *mut std::ffi::c_void = std::ptr::null_mut();
            if SHCreateItemFromParsingName(
                winit.as_ptr(),
                std::ptr::null_mut(),
                &IID_I_SHELL_ITEM,
                &mut item_raw,
            ) >= 0
                && !item_raw.is_null()
            {
                let _ = set_folder_fn(dlg_raw, item_raw);
                let item = item_raw as *mut ComObj;
                let item_rel: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                    std::mem::transmute(*(*item).vtbl.add(2));
                item_rel(item_raw);
            }
        }

        let hr_show = show_fn(dlg_raw, owner);
        let out = if hr_show == HRESULT_CANCELLED {
            Ok(None)
        } else if hr_show >= 0 {
            let mut res_raw: *mut std::ffi::c_void = std::ptr::null_mut();
            if get_result_fn(dlg_raw, &mut res_raw) >= 0 && !res_raw.is_null() {
                let item = res_raw as *mut ComObj;
                let item_rel: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                    std::mem::transmute(*(*item).vtbl.add(2));
                let get_name: unsafe extern "system" fn(
                    *mut std::ffi::c_void,
                    u32,
                    *mut *mut u16,
                ) -> i32 = std::mem::transmute(*(*item).vtbl.add(5));
                let mut psz: *mut u16 = std::ptr::null_mut();
                let picked = if get_name(res_raw, SIGDN_FILESYSPATH, &mut psz) >= 0 && !psz.is_null()
                {
                    let mut len = 0usize;
                    while *psz.add(len) != 0 {
                        len += 1;
                    }
                    let slice = std::slice::from_raw_parts(psz, len);
                    let path = PathBuf::from(String::from_utf16_lossy(slice));
                    CoTaskMemFree(psz as *const _);
                    Some(path)
                } else {
                    None
                };
                item_rel(res_raw);
                Ok(picked)
            } else {
                Ok(None)
            }
        } else {
            Err(())
        };

        release_fn(dlg_raw);
        if should_uninit {
            CoUninitialize();
        }
        out
    }
}

fn browse_folder(owner: HWND, title: &str, initial_dir: Option<&Path>) -> Option<PathBuf> {
    if let Ok(res) = browse_folder_modern(owner, title, initial_dir) {
        return res;
    }
    let wtitle = to_wide(title);
    let initial_wide = initial_dir
        .filter(|p| p.is_dir())
        .map(crate::instances::strip_windows_verbatim_prefix)
        .map(|p| to_wide(&p.to_string_lossy()));
    let mut display_name = vec![0u16; 260];
    let mut path_buf = vec![0u16; 1024];
    unsafe {
        let hr = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let should_uninit = hr == S_OK || hr == S_FALSE;
        let bi = BROWSEINFOW {
            hwndOwner: owner,
            pidlRoot: std::ptr::null_mut(),
            pszDisplayName: display_name.as_mut_ptr(),
            lpszTitle: wtitle.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            lpfn: if initial_wide.is_some() {
                Some(browse_callback)
            } else {
                None
            },
            lParam: initial_wide
                .as_ref()
                .map(|w| w.as_ptr() as isize)
                .unwrap_or(0),
            iImage: 0,
        };
        let pidl = SHBrowseForFolderW(&bi);
        let result = if !pidl.is_null() {
            let ok = SHGetPathFromIDListW(pidl, path_buf.as_mut_ptr());
            CoTaskMemFree(pidl as *const _);
            if ok != 0 {
                Some(PathBuf::from(from_wide(&path_buf)))
            } else {
                None
            }
        } else {
            None
        };
        if should_uninit {
            CoUninitialize();
        }
        result
    }
}

fn copy_text_to_clipboard(owner: HWND, text: &str) -> bool {
    let wide = to_wide(text);
    let byte_len = wide.len() * std::mem::size_of::<u16>();
    unsafe {
        if OpenClipboard(owner) == 0 {
            return false;
        }
        EmptyClipboard();
        let h_mem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
        if h_mem.is_null() {
            CloseClipboard();
            return false;
        }
        let ptr = GlobalLock(h_mem) as *mut u16;
        if ptr.is_null() {
            let _ = GlobalFree(h_mem);
            CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        GlobalUnlock(h_mem);
        if SetClipboardData(CF_UNICODETEXT, h_mem).is_null() {
            let _ = GlobalFree(h_mem);
            CloseClipboard();
            return false;
        }
        CloseClipboard();
        true
    }
}

fn shell_open_target(owner: HWND, target: &str) -> bool {
    let op = to_wide("open");
    let file = to_wide(target);
    unsafe {
        let res = ShellExecuteW(
            owner,
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
        (res as isize) > 32
    }
}

fn sanitize_identifier(raw: &str, default: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    let trimmed = out.trim_matches(&['_', '-'][..]).to_string();
    if trimmed.is_empty() || is_dos_reserved(&trimmed) {
        return default.to_string();
    }
    trimmed.chars().take(48).collect()
}

fn is_dos_reserved(s: &str) -> bool {
    let base = s.split('.').next().unwrap_or(s).to_ascii_uppercase();
    RESERVED_DOS_NAMES.contains(&base.as_str())
}

#[allow(clippy::too_many_arguments)]
fn create_control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    ex_style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
    hinstance: isize,
    font: isize,
) -> HWND {
    let wc = to_wide(class);
    let wt = to_wide(text);
    unsafe {
        let hwnd = CreateWindowExW(
            ex_style,
            wc.as_ptr(),
            wt.as_ptr(),
            style,
            x,
            y,
            w,
            h,
            parent,
            id as isize as HMENU,
            hinstance as _,
            std::ptr::null(),
        );
        if font != 0 && !hwnd.is_null() {
            SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
        }
        hwnd
    }
}

pub fn normalize_dimension_id(dim: &str) -> String {
    let trimmed = dim.trim();
    if trimmed.is_empty() {
        return "overworld".to_string();
    }
    let low = trimmed.to_ascii_lowercase();
    match low.as_str() {
        "overworld" | "minecraft:overworld" | "0" | "dim0" | "dim_overworld" => {
            "overworld".to_string()
        }
        "nether" | "the_nether" | "minecraft:the_nether" | "-1" | "dim-1" | "dim_nether" => {
            "nether".to_string()
        }
        "end" | "the_end" | "minecraft:the_end" | "1" | "dim1" | "dim_end" => "end".to_string(),
        _ => low,
    }
}

fn dimension_combo_entries(lang: UiLang, custom_dim: Option<&str>) -> Vec<String> {
    let mut labels = vec![
        tr_text(lang, "dim_overworld").to_string(),
        tr_text(lang, "dim_nether").to_string(),
        tr_text(lang, "dim_end").to_string(),
    ];
    if let Some(custom) = custom_dim {
        let norm = normalize_dimension_id(custom);
        if !matches!(norm.as_str(), "overworld" | "nether" | "end") {
            labels.push(norm);
        }
    }
    labels
}

fn dim_index_from_str(dim: &str, custom_dim: Option<&str>) -> usize {
    let norm = normalize_dimension_id(dim);
    match norm.as_str() {
        "overworld" => 0,
        "nether" => 1,
        "end" => 2,
        _ => {
            if custom_dim.is_some_and(|c| normalize_dimension_id(c) == norm) {
                3
            } else {
                0
            }
        }
    }
}

fn dim_str_from_index(idx: isize, custom_dim: Option<&str>) -> String {
    match idx {
        1 => "nether".to_string(),
        2 => "end".to_string(),
        3 => custom_dim
            .map(normalize_dimension_id)
            .filter(|d| !matches!(d.as_str(), "overworld" | "nether" | "end"))
            .unwrap_or_else(|| "overworld".to_string()),
        _ => "overworld".to_string(),
    }
}

fn set_dim_combo_value(dim: &str, lang: UiLang) {
    let norm = normalize_dimension_id(dim);
    let is_custom = !matches!(norm.as_str(), "overworld" | "nether" | "end");
    let custom_opt = if is_custom {
        if let Some(lock) = GUI_STATE.get() {
            if let Ok(mut st) = lock.lock() {
                st.custom_dimension = Some(norm.clone());
            }
        }
        Some(norm.clone())
    } else {
        GUI_STATE
            .get()
            .and_then(|l| l.lock().ok())
            .and_then(|st| st.custom_dimension.clone())
    };
    if let Some(hd) = get_ctrl_hwnd(ID_COMBO_DIM) {
        let labels = dimension_combo_entries(lang, custom_opt.as_deref());
        let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let sel_idx = dim_index_from_str(&norm, custom_opt.as_deref());
        repopulate_combo(hd, &label_refs, sel_idx);
    }
}

fn light_index_from_str(style: &str) -> usize {
    match style.trim().to_ascii_lowercase().as_str() {
        "gmod" => 1,
        "bright" => 2,
        "night" => 3,
        _ => 0,
    }
}

fn light_str_from_index(idx: isize) -> &'static str {
    match idx {
        1 => "gmod",
        2 => "bright",
        3 => "night",
        _ => "minecraft",
    }
}

fn preset_index_from_str(p: &str) -> usize {
    match p.trim().to_ascii_lowercase().as_str() {
        "preview" => 0,
        "balanced" => 1,
        "hq" => 2,
        _ => 3,
    }
}

fn preset_str_from_index(idx: isize) -> &'static str {
    match idx {
        0 => "preview",
        1 => "balanced",
        2 => "hq",
        _ => "custom",
    }
}

fn repopulate_combo(combo_hwnd: isize, items: &[&str], selected_idx: usize) {
    if combo_hwnd == 0 {
        return;
    }
    unsafe {
        SendMessageW(combo_hwnd as HWND, CB_RESETCONTENT, 0, 0);
        for item in items {
            let wi = to_wide(item);
            SendMessageW(combo_hwnd as HWND, CB_ADDSTRING, 0, wi.as_ptr() as isize);
        }
        let idx = selected_idx.min(items.len().saturating_sub(1));
        SendMessageW(combo_hwnd as HWND, CB_SETCURSEL, idx, 0);
    }
}

fn apply_language(lang: UiLang, busy: bool, status_key: &'static str, custom_status: Option<&str>, preset: &str) {
    let Some(handles) = GUI_HANDLES.get() else { return };
    set_ctrl_text(handles.hwnd, tr_text(lang, "win_title"));
    let layout_mode = GUI_STATE
        .get()
        .and_then(|l| l.try_lock().ok())
        .map(|s| s.layout_mode)
        .unwrap_or_default();
    let map = [
        (ID_LBL_TITLE, "ui_title"),
        (ID_GRP_SRC, "step_world"),
        (ID_LBL_INSTANCE, "lbl_instance"),
        (ID_LBL_WORLD_ROW, "lbl_world_row"),
        (ID_BTN_ADD_INSTANCE, "btn_add_instance"),
        (ID_GRP_META, "step_area"),
        (ID_GRP_OPTS, "step_map"),
        (ID_BTN_RAD_32, "rad_small"),
        (ID_BTN_RAD_64, "rad_medium"),
        (ID_BTN_RAD_128, "rad_large"),
        (ID_LBL_PRESET, "lbl_preset"),
        (ID_GRP_ADV, "grp_adv"),
        (ID_LBL_LOG_HDR, "lbl_log_hdr"),
        (ID_LBL_WORLD, "lbl_world"),
        (ID_LBL_COORDS_MIN, "lbl_min"),
        (ID_LBL_COORDS_MAX, "lbl_max"),
        (ID_LBL_DIM, "lbl_dim"),
        (ID_LBL_SCALE, "lbl_scale"),
        (ID_LBL_NAME, "lbl_name"),
        (ID_LBL_CAT, "lbl_cat"),
        (ID_LBL_MAP, "lbl_map"),
        (ID_LBL_OUTDIR, "lbl_outdir"),
        (ID_LBL_LIGHT, "lbl_light"),
        (ID_CHK_IMPORT_GMOD, "chk_import"),
        (ID_CHK_EXPORT_VMF, "chk_vmf"),
        (ID_CHK_HQ_LIGHT, "chk_hq"),
        (ID_CHK_FAST_VRAD, "chk_fast"),
        (ID_CHK_MC_PHYSICS, "chk_phys"),
        (ID_CHK_VMF_BRUSHES, "chk_brushes"),
        (ID_BTN_BROWSE_WORLD, "btn_browse"),
        (ID_BTN_BROWSE_OUT, "btn_browse"),
        (ID_BTN_LANG, "btn_lang"),
        (ID_BTN_STOP, "btn_stop"),
        (ID_BTN_CLEAR, "btn_clear"),
        (ID_BTN_OPEN_OUT, "btn_open_out"),
        (ID_BTN_COPY_MAP, "btn_copy_map"),
        (ID_BTN_LAUNCH_GMOD, "btn_launch_gmod"),
    ];
    for (id, key) in map {
        if let Some(&h) = handles.controls.get(&id) {
            set_ctrl_text(h, tr_text(lang, key));
        }
    }
    if let Some(&h) = handles.controls.get(&ID_BTN_TOGGLE_ADV) {
        set_ctrl_text(
            h,
            tr_text(lang, if layout_mode.show_advanced { "btn_adv_hide" } else { "btn_adv_show" }),
        );
    }
    if let Some(&h) = handles.controls.get(&ID_BTN_TOGGLE_LOG) {
        set_ctrl_text(
            h,
            tr_text(lang, if layout_mode.show_log { "btn_log_hide" } else { "btn_log_show" }),
        );
    }
    if let Some(&h) = handles.controls.get(&ID_BTN_START) {
        set_ctrl_text(h, if busy { tr_text(lang, "btn_running") } else { tr_text(lang, "btn_start") });
    }
    if let Some(&hs) = handles.controls.get(&ID_LBL_STATUS) {
        if let Some(custom) = custom_status {
            set_ctrl_text(hs, custom);
        } else {
            set_ctrl_text(hs, tr_text(lang, status_key));
        }
    }

    if let Some(&hp) = handles.controls.get(&ID_COMBO_PRESET) {
        let idx = preset_index_from_str(preset);
        repopulate_combo(
            hp,
            &[
                tr_text(lang, "preset_preview"),
                tr_text(lang, "preset_balanced"),
                tr_text(lang, "preset_hq"),
                tr_text(lang, "preset_custom"),
            ],
            idx,
        );
    }
    if let Some(&hd) = handles.controls.get(&ID_COMBO_DIM) {
        let cur = unsafe { SendMessageW(hd as HWND, CB_GETCURSEL, 0, 0) }.max(0) as usize;
        let custom_dim = GUI_STATE
            .get()
            .and_then(|l| l.try_lock().ok())
            .and_then(|s| s.custom_dimension.clone());
        let labels = dimension_combo_entries(lang, custom_dim.as_deref());
        let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        repopulate_combo(hd, &label_refs, cur);
    }
    if let Some(&hl) = handles.controls.get(&ID_COMBO_LIGHT) {
        let cur = unsafe { SendMessageW(hl as HWND, CB_GETCURSEL, 0, 0) }.max(0) as usize;
        repopulate_combo(
            hl,
            &[
                tr_text(lang, "light_minecraft"),
                tr_text(lang, "light_gmod"),
                tr_text(lang, "light_bright"),
                tr_text(lang, "light_night"),
            ],
            cur,
        );
    }
    let cur_world = get_ctrl_hwnd(ID_EDIT_WORLD)
        .map(|hw| get_ctrl_text(hw).trim().to_string())
        .filter(|s| !s.is_empty());
    sync_instances_combo(lang);
    sync_worlds_combo(lang, cur_world.as_deref().map(Path::new));
    update_selection_info_label(lang);
}

fn update_selection_info_label(lang: UiLang) {
    let Some(h_lbl) = get_ctrl_hwnd(ID_LBL_SEL_INFO) else { return };
    let parse = |id: i32| -> Option<i32> {
        get_ctrl_hwnd(id).and_then(|h| get_ctrl_text(h).trim().parse::<i32>().ok())
    };
    let coords = match (
        parse(ID_EDIT_COORD_X1),
        parse(ID_EDIT_COORD_Y1),
        parse(ID_EDIT_COORD_Z1),
        parse(ID_EDIT_COORD_X2),
        parse(ID_EDIT_COORD_Y2),
        parse(ID_EDIT_COORD_Z2),
    ) {
        (Some(x1), Some(y1), Some(z1), Some(x2), Some(y2), Some(z2)) => Some([x1, y1, z1, x2, y2, z2]),
        _ => None,
    };
    set_ctrl_text(h_lbl, &format_selection_summary(lang, coords));
}

fn apply_panel_visibility(mode: GuiLayoutMode, busy: bool) {
    let Some(handles) = GUI_HANDLES.get() else { return };
    let set_vis = |id: i32, show: bool| {
        if let Some(&ctrl) = handles.controls.get(&id) {
            unsafe {
                ShowWindow(ctrl as HWND, if show { SW_SHOW } else { SW_HIDE });
            }
        }
    };
    for &id in ADVANCED_ONLY_CONTROLS {
        set_vis(id, mode.show_advanced);
    }
    for id in [ID_LBL_LOG_HDR, ID_BTN_CLEAR, ID_LOG] {
        set_vis(id, mode.show_log);
    }
    set_vis(ID_BTN_STOP, busy);
}

fn set_panel_mode(hwnd: HWND, new_mode: GuiLayoutMode) {
    let (lang, busy, dpi) = if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.layout_mode = new_mode;
            (st.lang, st.busy, st.dpi)
        } else {
            return;
        }
    } else {
        return;
    };

    if let Some(h) = get_ctrl_hwnd(ID_BTN_TOGGLE_ADV) {
        set_ctrl_text(
            h,
            tr_text(lang, if new_mode.show_advanced { "btn_adv_hide" } else { "btn_adv_show" }),
        );
    }
    if let Some(h) = get_ctrl_hwnd(ID_BTN_TOGGLE_LOG) {
        set_ctrl_text(
            h,
            tr_text(lang, if new_mode.show_log { "btn_log_hide" } else { "btn_log_show" }),
        );
    }

    apply_panel_visibility(new_mode, busy);

    unsafe {
        let mut wr: RECT = std::mem::zeroed();
        let mut cr: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut wr);
        GetClientRect(hwnd, &mut cr);
        let cur_cw = cr.right - cr.left;
        let nc_w = (wr.right - wr.left) - cur_cw;
        let nc_h = (wr.bottom - wr.top) - (cr.bottom - cr.top);
        let (def_cw_96, def_ch_96) = compute_default_client_size(new_mode);
        let def_cw = scale_px(def_cw_96, dpi);
        let def_ch = scale_px(def_ch_96, dpi);
        let target_cw = cur_cw.max(def_cw);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            target_cw + nc_w,
            def_ch + nc_h,
            SWP_NOMOVE | SWP_NOZORDER,
        );
        GetClientRect(hwnd, &mut cr);
        layout_controls(cr.right - cr.left, cr.bottom - cr.top);
        RedrawWindow(
            hwnd,
            std::ptr::null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
        );
    }
}

fn sync_instances_combo(lang: UiLang) {
    let Some(hc) = get_ctrl_hwnd(ID_COMBO_INSTANCES) else {
        return;
    };
    let (items, sel_idx) = {
        let Some(lock) = GUI_STATE.get() else {
            return;
        };
        let Ok(mut st) = lock.lock() else {
            return;
        };
        st.suppress_instance_combo = true;
        let items = build_instance_combo_items(&st.discovered_instances, lang);
        let sel = st
            .selected_instance_idx
            .filter(|&i| i < st.discovered_instances.len())
            .map(|i| i + 1)
            .unwrap_or(0);
        (items, sel)
    };
    unsafe {
        SendMessageW(hc as HWND, CB_RESETCONTENT, 0, 0);
        for item in &items {
            let wi = to_wide(item);
            SendMessageW(hc as HWND, CB_ADDSTRING, 0, wi.as_ptr() as isize);
        }
        SendMessageW(hc as HWND, CB_SETCURSEL, sel_idx, 0);
    }
    if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.suppress_instance_combo = false;
        }
    }
}

fn sync_worlds_combo(lang: UiLang, current_world: Option<&Path>) {
    let Some(hc) = get_ctrl_hwnd(ID_COMBO_WORLDS) else { return };
    let (discovered, has_selected_inst) = GUI_STATE
        .get()
        .and_then(|l| l.lock().ok())
        .map(|s| {
            let w = worlds_for_instance_selection(&s.discovered_instances, s.selected_instance_idx);
            (w, s.selected_instance_idx.is_some())
        })
        .unwrap_or_default();
    let (merged, sel_idx) = merge_discovered_world(&discovered, current_world, lang);
    if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.suppress_world_combo = true;
        }
    }
    unsafe {
        SendMessageW(hc as HWND, CB_RESETCONTENT, 0, 0);
        if merged.is_empty() {
            let empty_key = if has_selected_inst {
                "worlds_none_in_instance"
            } else {
                "worlds_none"
            };
            let wi = to_wide(tr_text(lang, empty_key));
            SendMessageW(hc as HWND, CB_ADDSTRING, 0, wi.as_ptr() as isize);
            SendMessageW(hc as HWND, CB_SETCURSEL, 0, 0);
        } else {
            for (lbl, _) in &merged {
                let wl = to_wide(lbl);
                SendMessageW(hc as HWND, CB_ADDSTRING, 0, wl.as_ptr() as isize);
            }
            SendMessageW(hc as HWND, CB_SETCURSEL, sel_idx.unwrap_or(0), 0);
        }
    }
    if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.discovered_worlds = merged;
            st.suppress_world_combo = false;
        }
    }
}

fn layout_controls(client_w: i32, client_h: i32) {
    let Some(handles) = GUI_HANDLES.get() else { return };
    let (mode, busy, dpi) = GUI_STATE
        .get()
        .and_then(|l| l.try_lock().ok())
        .map(|s| (s.layout_mode, s.busy, s.dpi))
        .unwrap_or((GuiLayoutMode::default(), false, 96));
    let sc = |v: i32| scale_px(v, dpi);
    let (min_cw_96, min_ch_96) = compute_min_client_size(mode);
    let w = client_w.max(sc(min_cw_96));
    let h = client_h.max(sc(min_ch_96));
    let m = sc(10);
    let gm = sc(20);
    let full_w = w - m * 2;
    let inner_w = w - gm * 2;
    let mv = |id: i32, x: i32, y: i32, cw: i32, ch: i32| {
        if let Some(&ctrl) = handles.controls.get(&id) {
            unsafe {
                MoveWindow(ctrl as HWND, x, y, cw, ch, 1);
            }
        }
    };

    // Header
    mv(ID_LBL_TITLE, m, sc(8), full_w - sc(86), sc(22));
    mv(ID_BTN_LANG, w - m - sc(78), sc(6), sc(78), sc(23));

    let cards = compute_card_rects(w, mode, dpi);
    if let Some(&(l, t, r, b)) = cards.first() {
        mv(ID_GRP_SRC, l, t, r - l, b - t);
    }
    if let Some(&(l, t, r, b)) = cards.get(1) {
        mv(ID_GRP_META, l, t, r - l, b - t);
    }
    if let Some(&(l, t, r, b)) = cards.get(2) {
        mv(ID_GRP_OPTS, l, t, r - l, b - t);
    }
    if mode.show_advanced {
        if let Some(&(l, t, r, b)) = cards.get(3) {
            mv(ID_GRP_ADV, l, t, r - l, b - t);
        }
    }

    // Step 1: Instance & World (inside groupbox 1: 34..108)
    let row_lbl_w = sc(72);
    let row_combo_x = gm + row_lbl_w + sc(4);
    let row_combo_w = inner_w - row_lbl_w - sc(4) - sc(110);
    mv(ID_LBL_INSTANCE, gm, sc(52), row_lbl_w, sc(18));
    mv(ID_COMBO_INSTANCES, row_combo_x, sc(49), row_combo_w, sc(220));
    mv(ID_BTN_ADD_INSTANCE, w - gm - sc(102), sc(49), sc(102), sc(23));

    mv(ID_LBL_WORLD_ROW, gm, sc(79), row_lbl_w, sc(18));
    mv(ID_COMBO_WORLDS, row_combo_x, sc(76), row_combo_w, sc(220));
    mv(ID_BTN_BROWSE_WORLD, w - gm - sc(102), sc(76), sc(102), sc(23));

    // Step 2: Area + Coords (inside groupbox 2: 112..230)
    let rad_gap = sc(8);
    let rad_w = (inner_w - rad_gap * 2) / 3;
    mv(ID_BTN_RAD_32, gm, sc(130), rad_w, sc(23));
    mv(ID_BTN_RAD_64, gm + rad_w + rad_gap, sc(130), rad_w, sc(23));
    mv(ID_BTN_RAD_128, gm + (rad_w + rad_gap) * 2, sc(130), inner_w - (rad_w + rad_gap) * 2, sc(23));

    let coord_lbl_w = sc(104);
    let coord_x = gm + sc(108);
    let coord_gap = sc(8);
    let coord_w = (inner_w - sc(108) - coord_gap * 2) / 3;
    let coord_w3 = inner_w - sc(108) - (coord_w + coord_gap) * 2;
    mv(ID_LBL_COORDS_MIN, gm, sc(160), coord_lbl_w, sc(18));
    mv(ID_EDIT_COORD_X1, coord_x, sc(157), coord_w, sc(22));
    mv(ID_EDIT_COORD_Y1, coord_x + coord_w + coord_gap, sc(157), coord_w, sc(22));
    mv(ID_EDIT_COORD_Z1, coord_x + (coord_w + coord_gap) * 2, sc(157), coord_w3, sc(22));

    mv(ID_LBL_COORDS_MAX, gm, sc(186), coord_lbl_w, sc(18));
    mv(ID_EDIT_COORD_X2, coord_x, sc(183), coord_w, sc(22));
    mv(ID_EDIT_COORD_Y2, coord_x + coord_w + coord_gap, sc(183), coord_w, sc(22));
    mv(ID_EDIT_COORD_Z2, coord_x + (coord_w + coord_gap) * 2, sc(183), coord_w3, sc(22));
    mv(ID_LBL_SEL_INFO, gm, sc(210), inner_w, sc(16));

    // Step 3: Map settings (inside groupbox 3: 234..362)
    mv(ID_LBL_MAP, gm, sc(252), sc(122), sc(18));
    mv(ID_EDIT_MAP, gm + sc(126), sc(249), inner_w - sc(126), sc(22));

    let mid_gap = sc(16);
    let half_w = (inner_w - mid_gap) / 2;
    let right_x = gm + half_w + mid_gap;
    let left_lbl_w = sc(88);
    let right_lbl_w = sc(72);
    mv(ID_LBL_PRESET, gm, sc(279), left_lbl_w, sc(18));
    mv(ID_COMBO_PRESET, gm + left_lbl_w + sc(4), sc(276), half_w - left_lbl_w - sc(4), sc(160));
    mv(ID_LBL_LIGHT, right_x, sc(279), right_lbl_w, sc(18));
    mv(ID_COMBO_LIGHT, right_x + right_lbl_w + sc(4), sc(276), half_w - right_lbl_w - sc(4), sc(160));

    mv(ID_LBL_OUTDIR, gm, sc(306), sc(112), sc(18));
    mv(ID_EDIT_OUTDIR, gm + sc(116), sc(303), inner_w - sc(116) - sc(94), sc(22));
    mv(ID_BTN_BROWSE_OUT, w - gm - sc(88), sc(303), sc(88), sc(23));

    mv(ID_CHK_IMPORT_GMOD, gm, sc(334), inner_w, sc(19));

    // Action & Progress
    if busy {
        mv(ID_BTN_START, m, sc(369), full_w - sc(124), sc(32));
        mv(ID_BTN_STOP, w - m - sc(116), sc(369), sc(116), sc(32));
    } else {
        mv(ID_BTN_START, m, sc(369), full_w, sc(32));
        mv(ID_BTN_STOP, w - m - sc(116), sc(369), sc(116), sc(32));
    }
    mv(ID_PROGRESS, m, sc(408), full_w, sc(15));
    mv(ID_LBL_STATUS, m, sc(427), full_w, sc(16));

    // Footer toggles + quick actions
    mv(ID_BTN_TOGGLE_ADV, m, sc(452), sc(134), sc(25));
    mv(ID_BTN_TOGGLE_LOG, m + sc(140), sc(452), sc(100), sc(25));
    mv(ID_BTN_OPEN_OUT, w - m - sc(280), sc(452), sc(128), sc(25));
    mv(ID_BTN_LAUNCH_GMOD, w - m - sc(144), sc(452), sc(144), sc(25));

    // Advanced panel (inside groupbox 4: 488..714)
    if mode.show_advanced {
        let ay = sc(488);
        let ax = gm;
        let aw = inner_w;
        let albl = sc(104);
        let aedit_x = ax + albl;

        mv(ID_LBL_WORLD, ax, ay + sc(22), albl, sc(18));
        mv(ID_EDIT_WORLD, aedit_x, ay + sc(19), aw - albl, sc(23));

        let amid_gap = sc(14);
        let ahalf_w = (aw - amid_gap) / 2;
        let aright_x = ax + ahalf_w + amid_gap;
        mv(ID_LBL_DIM, ax, ay + sc(52), sc(92), sc(18));
        mv(ID_COMBO_DIM, ax + sc(96), ay + sc(49), ahalf_w - sc(96), sc(140));
        mv(ID_LBL_SCALE, aright_x, ay + sc(52), sc(84), sc(18));
        mv(ID_EDIT_SCALE, aright_x + sc(88), ay + sc(49), ahalf_w - sc(88), sc(23));

        mv(ID_LBL_NAME, ax, ay + sc(82), sc(92), sc(18));
        mv(ID_EDIT_NAME, ax + sc(96), ay + sc(79), ahalf_w - sc(96), sc(23));
        mv(ID_LBL_CAT, aright_x, ay + sc(82), sc(84), sc(18));
        mv(ID_EDIT_CAT, aright_x + sc(88), ay + sc(79), ahalf_w - sc(88), sc(23));

        let col_w = (aw - sc(12)) / 2;
        let col2_x = ax + col_w + sc(12);
        mv(ID_CHK_MC_PHYSICS, ax, ay + sc(114), col_w, sc(20));
        mv(ID_CHK_EXPORT_VMF, col2_x, ay + sc(114), col_w, sc(20));
        mv(ID_CHK_HQ_LIGHT, ax, ay + sc(138), col_w, sc(20));
        mv(ID_CHK_FAST_VRAD, col2_x, ay + sc(138), col_w, sc(20));
        mv(ID_CHK_VMF_BRUSHES, ax, ay + sc(162), col_w, sc(20));
        mv(ID_BTN_COPY_MAP, col2_x, ay + sc(160), sc(156), sc(24));
    }

    // Log panel
    if mode.show_log {
        let log_top = if mode.show_advanced { sc(722) } else { sc(488) };
        mv(ID_LBL_LOG_HDR, m, log_top + sc(4), full_w - sc(120), sc(20));
        mv(ID_BTN_CLEAR, w - m - sc(112), log_top, sc(112), sc(24));
        let log_y = log_top + sc(28);
        let log_h = (h - log_y - m).max(sc(80));
        mv(ID_LOG, m, log_y, full_w, log_h);
    }
}

fn apply_build_preset_ui(preset: &str) {
    let p = preset.trim().to_ascii_lowercase();
    let (fast, hq) = match p.as_str() {
        "preview" => (true, false),
        "balanced" => (false, false),
        "hq" => (false, true),
        _ => return,
    };
    if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.syncing_preset = true;
            st.build_preset = p.clone();
        }
    }
    if let Some(hf) = get_ctrl_hwnd(ID_CHK_FAST_VRAD) {
        set_chk_state(hf, fast);
    }
    if let Some(hh) = get_ctrl_hwnd(ID_CHK_HQ_LIGHT) {
        set_chk_state(hh, hq);
    }
    if let Some(hp) = get_ctrl_hwnd(ID_COMBO_PRESET) {
        unsafe {
            SendMessageW(hp as HWND, CB_SETCURSEL, preset_index_from_str(&p), 0);
        }
    }
    if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.syncing_preset = false;
        }
    }
}

fn sync_preset_from_checkboxes_ui() {
    let syncing = GUI_STATE
        .get()
        .and_then(|l| l.lock().ok())
        .map(|s| s.syncing_preset)
        .unwrap_or(false);
    if syncing {
        return;
    }
    let fast = get_ctrl_hwnd(ID_CHK_FAST_VRAD).map(get_chk_state).unwrap_or(false);
    let hq = get_ctrl_hwnd(ID_CHK_HQ_LIGHT).map(get_chk_state).unwrap_or(false);
    let detected = match (fast, hq) {
        (true, false) => "preview",
        (false, false) => "balanced",
        (false, true) => "hq",
        (true, true) => "custom",
    };
    if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.build_preset = detected.to_string();
        }
    }
    if let Some(hp) = get_ctrl_hwnd(ID_COMBO_PRESET) {
        unsafe {
            SendMessageW(hp as HWND, CB_SETCURSEL, preset_index_from_str(detected), 0);
        }
    }
}

fn set_coord_boxes(x1: i32, y1: i32, z1: i32, x2: i32, y2: i32, z2: i32) {
    let pairs = [
        (ID_EDIT_COORD_X1, x1),
        (ID_EDIT_COORD_Y1, y1),
        (ID_EDIT_COORD_Z1, z1),
        (ID_EDIT_COORD_X2, x2),
        (ID_EDIT_COORD_Y2, y2),
        (ID_EDIT_COORD_Z2, z2),
    ];
    for (id, val) in pairs {
        if let Some(h) = get_ctrl_hwnd(id) {
            set_ctrl_text(h, &val.to_string());
        }
    }
}

fn populate_from_world_path(world_path: &Path, populate_coords: bool) {
    if !world_path.is_dir() {
        return;
    }
    let (lang, last_auto, model_manual) = GUI_STATE
        .get()
        .and_then(|l| l.lock().ok())
        .map(|s| (s.lang, s.last_auto_map_name.clone(), s.model_name_manually_edited))
        .unwrap_or((UiLang::En, None, false));

    if let Some(spawn) = crate::anvil::read_level_spawn(world_path) {
        let cx = spawn[0].round() as i32;
        let cy = spawn[1].round() as i32;
        let cz = spawn[2].round() as i32;
        if let Some(lock) = GUI_STATE.get() {
            if let Ok(mut st) = lock.lock() {
                st.anchor_center = Some([cx, cy, cz]);
            }
        }
        if populate_coords {
            let r = 64;
            let y1 = (cy - 32).max(-64);
            let y2 = (cy + 64).min(319);
            set_coord_boxes(cx - r, y1, cz - r, cx + r, y2, cz + r);
            if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
                append_log_line(
                    hlog,
                    &format!(
                        "[GUI] level.dat spawn/player: X={}, Y={}, Z={} -> bounds [{}, {}, {}] .. [{}, {}, {}]",
                        cx, cy, cz, cx - r, y1, cz - r, cx + r, y2, cz + r
                    ),
                );
            }
        }
    }

    if let Some(folder_name) = world_path.file_name().and_then(|s| s.to_str()) {
        let safe = sanitize_identifier(folder_name, "");
        if !safe.is_empty() {
            let map_val = if safe.starts_with("gm_") {
                safe.clone()
            } else {
                format!("gm_{}", safe)
            };
            if let Some(hm) = get_ctrl_hwnd(ID_EDIT_MAP) {
                let cur_map = get_ctrl_text(hm).trim().to_string();
                if should_auto_replace_map_name(&cur_map, last_auto.as_deref()) {
                    if let Some(lock) = GUI_STATE.get() {
                        if let Ok(mut st) = lock.lock() {
                            st.suppress_name_sync = true;
                            st.last_auto_map_name = Some(map_val.clone());
                        }
                    }
                    set_ctrl_text(hm, &map_val);
                    if !model_manual {
                        if let Some(hn) = get_ctrl_hwnd(ID_EDIT_NAME) {
                            let derived = derive_model_name_from_map(&map_val);
                            set_ctrl_text(hn, &derived);
                        }
                    }
                    if let Some(lock) = GUI_STATE.get() {
                        if let Ok(mut st) = lock.lock() {
                            st.suppress_name_sync = false;
                        }
                    }
                }
            }
        }
    }
    update_selection_info_label(lang);
}

fn apply_radius_preset(radius: i32) {
    let r = radius.max(4);
    let (anchor, lang) = GUI_STATE
        .get()
        .and_then(|l| l.lock().ok())
        .map(|s| (s.anchor_center, s.lang))
        .unwrap_or((None, UiLang::En));

    let [cx, cy, cz] = if let Some(a) = anchor {
        a
    } else {
        let parse = |id: i32| -> Option<i32> {
            get_ctrl_hwnd(id).and_then(|h| get_ctrl_text(h).trim().parse::<i32>().ok())
        };
        match (
            parse(ID_EDIT_COORD_X1),
            parse(ID_EDIT_COORD_Y1),
            parse(ID_EDIT_COORD_Z1),
            parse(ID_EDIT_COORD_X2),
            parse(ID_EDIT_COORD_Y2),
            parse(ID_EDIT_COORD_Z2),
        ) {
            (Some(x1), Some(y1), Some(z1), Some(x2), Some(y2), Some(z2)) => {
                [(x1 + x2) / 2, (y1 + y2) / 2, (z1 + z2) / 2]
            }
            _ => [0, 64, 0],
        }
    };

    if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.anchor_center = Some([cx, cy, cz]);
        }
    }
    let y1 = (cy - (r / 2).min(32)).max(-64);
    let y2 = (cy + r.max(32)).min(319);
    set_coord_boxes(cx - r, y1, cz - r, cx + r, y2, cz + r);
    update_selection_info_label(lang);
}

fn collect_persisted_state() -> PersistedGuiState {
    let get_s = |id: i32| -> String {
        get_ctrl_hwnd(id).map(|h| get_ctrl_text(h).trim().to_string()).unwrap_or_default()
    };
    let get_i = |id: i32| -> Option<i32> { get_s(id).parse::<i32>().ok() };
    let get_chk = |id: i32| -> bool { get_ctrl_hwnd(id).map(get_chk_state).unwrap_or(false) };

    let (lang, preset, layout_mode, selected_instance_saves, custom_instance_dirs, custom_dim) = GUI_STATE
        .get()
        .and_then(|l| l.lock().ok())
        .map(|s| {
            let sel_saves = s
                .selected_instance_idx
                .and_then(|i| s.discovered_instances.get(i))
                .map(|inst| inst.saves_dir.to_string_lossy().into_owned())
                .or_else(|| s.pending_saved_instance_saves.clone());
            let custom_dirs = s
                .custom_instance_dirs
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            (
                s.lang,
                s.build_preset.clone(),
                s.layout_mode,
                sel_saves,
                custom_dirs,
                s.custom_dimension.clone(),
            )
        })
        .unwrap_or((
            UiLang::En,
            "balanced".to_string(),
            GuiLayoutMode::default(),
            None,
            Vec::new(),
            None,
        ));

    let dim_idx = get_ctrl_hwnd(ID_COMBO_DIM)
        .map(|h| unsafe { SendMessageW(h as HWND, CB_GETCURSEL, 0, 0) })
        .unwrap_or(0);
    let light_idx = get_ctrl_hwnd(ID_COMBO_LIGHT)
        .map(|h| unsafe { SendMessageW(h as HWND, CB_GETCURSEL, 0, 0) })
        .unwrap_or(0);

    PersistedGuiState {
        lang: Some(if lang == UiLang::Ru { "ru" } else { "en" }.to_string()),
        world_dir: Some(get_s(ID_EDIT_WORLD)),
        min_x: get_i(ID_EDIT_COORD_X1),
        min_y: get_i(ID_EDIT_COORD_Y1),
        min_z: get_i(ID_EDIT_COORD_Z1),
        max_x: get_i(ID_EDIT_COORD_X2),
        max_y: get_i(ID_EDIT_COORD_Y2),
        max_z: get_i(ID_EDIT_COORD_Z2),
        dimension: Some(dim_str_from_index(dim_idx, custom_dim.as_deref())),
        name: Some(get_s(ID_EDIT_NAME)),
        category: Some(get_s(ID_EDIT_CAT)),
        map_name: Some(get_s(ID_EDIT_MAP)),
        output_dir: Some(get_s(ID_EDIT_OUTDIR)),
        lighting_style: Some(light_str_from_index(light_idx).to_string()),
        scale: Some(get_s(ID_EDIT_SCALE)),
        build_preset: Some(preset),
        import_to_gmod: Some(get_chk(ID_CHK_IMPORT_GMOD)),
        export_vmf: Some(get_chk(ID_CHK_EXPORT_VMF)),
        hq_light: Some(get_chk(ID_CHK_HQ_LIGHT)),
        fast_vrad: Some(get_chk(ID_CHK_FAST_VRAD)),
        mc_physics: Some(get_chk(ID_CHK_MC_PHYSICS)),
        force_vmf_brushes: Some(get_chk(ID_CHK_VMF_BRUSHES)),
        show_advanced: Some(layout_mode.show_advanced),
        show_log: Some(layout_mode.show_log),
        selected_instance_saves,
        custom_instance_dirs,
    }
}

fn save_gui_state() {
    let state = collect_persisted_state();
    let path = resolve_gui_state_save_path();
    if let Ok(json) = serde_json::to_string_pretty(&state) {
        let _ = std::fs::write(path, json);
    }
}

fn load_persisted_state() -> Option<PersistedGuiState> {
    let path = resolve_gui_state_load_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<PersistedGuiState>(&text).ok()
}

pub fn apply_cli_overrides_to_pipeline_options(
    opts: &mut PipelineOptions,
    chunk_size: Option<usize>,
    max_workers: Option<usize>,
    max_convex: Option<usize>,
    reuse_cache: Option<bool>,
    debug_log: Option<PathBuf>,
    build_preset: &str,
) {
    if let Some(cs) = chunk_size {
        opts.chunk_size = cs;
    }
    if let Some(mw) = max_workers {
        opts.max_workers = mw;
    }
    if let Some(mc) = max_convex {
        opts.max_convex = mc;
    }
    opts.reuse_cache = reuse_cache.unwrap_or_else(|| build_preset == "preview");
    if let Some(dl) = debug_log {
        opts.debug_log = Some(dl);
    }
}

#[cfg(test)]
pub(crate) fn resolve_gui_pipeline_options(
    saved: Option<&PersistedGuiState>,
    cfg: &GuiLaunchConfig,
) -> PipelineOptions {
    let mut opts = PipelineOptions {
        name: "mc_custom".to_string(),
        category: "custom_mc".to_string(),
        map_name: "gm_mc_custom".to_string(),
        coords: crate::anvil::WorldCoords {
            min_x: -50,
            min_y: -64,
            min_z: -50,
            max_x: 50,
            max_y: 128,
            max_z: 50,
        },
        ..Default::default()
    };
    if let Some(ps) = saved {
        if let Some(ref wd) = ps.world_dir {
            if !wd.is_empty() {
                opts.world_dir = PathBuf::from(wd);
            }
        }
        if let (Some(x1), Some(y1), Some(z1), Some(x2), Some(y2), Some(z2)) =
            (ps.min_x, ps.min_y, ps.min_z, ps.max_x, ps.max_y, ps.max_z)
        {
            opts.coords = crate::anvil::WorldCoords {
                min_x: x1,
                min_y: y1,
                min_z: z1,
                max_x: x2,
                max_y: y2,
                max_z: z2,
            }
            .normalized();
        }
        if let Some(ref dim) = ps.dimension {
            opts.dimension = normalize_dimension_id(dim);
        }
        if let Some(ref n) = ps.name {
            if !n.is_empty() {
                opts.name = n.clone();
            }
        }
        if let Some(ref c) = ps.category {
            if !c.is_empty() {
                opts.category = c.clone();
            }
        }
        if let Some(ref m) = ps.map_name {
            if !m.is_empty() {
                opts.map_name = m.clone();
            }
        }
        if let Some(ref o) = ps.output_dir {
            if !o.is_empty() {
                opts.output_dir = PathBuf::from(o);
            }
        }
        if let Some(ref l) = ps.lighting_style {
            opts.lighting_style = l.clone();
        }
        if let Some(ref sc) = ps.scale {
            if let Ok(v) = sc.replace(',', ".").parse::<f64>() {
                if v.is_finite() && v > 0.0 {
                    opts.scale = v;
                }
            }
        }
        if let Some(v) = ps.import_to_gmod {
            opts.import_to_gmod = v;
        }
        if let Some(v) = ps.export_vmf {
            opts.export_vmf = v;
        }
        if let Some(v) = ps.hq_light {
            opts.hq_light = v;
        }
        if let Some(v) = ps.fast_vrad {
            opts.fast_vrad = v;
        }
        if let Some(v) = ps.mc_physics {
            opts.mc_physics = v;
        }
        if let Some(v) = ps.force_vmf_brushes {
            opts.force_vmf_brushes = v;
        }
        if let Some(ref p) = ps.build_preset {
            match p.trim().to_ascii_lowercase().as_str() {
                "preview" => {
                    opts.fast_vrad = true;
                    opts.hq_light = false;
                }
                "balanced" => {
                    opts.fast_vrad = false;
                    opts.hq_light = false;
                }
                "hq" => {
                    opts.fast_vrad = false;
                    opts.hq_light = true;
                }
                _ => {}
            }
        }
    }

    if let Some(ref w) = cfg.world {
        opts.world_dir = w.clone();
    }
    if let Some(c) = cfg.coords {
        opts.coords = c.normalized();
    }
    if let Some(ref dim) = cfg.dimension {
        opts.dimension = normalize_dimension_id(dim);
    }
    if let Some(ref n) = cfg.name {
        opts.name = n.clone();
        if cfg.map_name.is_none() {
            opts.map_name = if n.starts_with("gm_") {
                n.clone()
            } else {
                format!("gm_{}", n)
            };
        }
    }
    if let Some(ref cat) = cfg.category {
        opts.category = cat.clone();
    }
    if let Some(ref m) = cfg.map_name {
        opts.map_name = m.clone();
    }
    if let Some(ref o) = cfg.output_dir {
        opts.output_dir = o.clone();
    }
    if let Some(ref l) = cfg.lighting_style {
        opts.lighting_style = l.clone();
    }
    if let Some(sc) = cfg.scale {
        if sc.is_finite() && sc > 0.0 {
            opts.scale = sc;
        }
    }
    if let Some(v) = cfg.import_to_gmod {
        opts.import_to_gmod = v;
    }
    if let Some(v) = cfg.export_vmf {
        opts.export_vmf = v;
    }
    if let Some(v) = cfg.hq_light {
        opts.hq_light = v;
    }
    if let Some(v) = cfg.fast_vrad {
        opts.fast_vrad = v;
    }
    if let Some(v) = cfg.mc_physics {
        opts.mc_physics = v;
    }
    if let Some(v) = cfg.force_vmf_brushes {
        opts.force_vmf_brushes = v;
    }
    if let Some(ref cp) = cfg.config_path {
        opts.config_path = Some(cp.clone());
    }

    let derived_preset = match (opts.fast_vrad, opts.hq_light) {
        (true, false) => "preview",
        (false, false) => "balanced",
        (false, true) => "hq",
        (true, true) => "custom",
    };
    apply_cli_overrides_to_pipeline_options(
        &mut opts,
        cfg.chunk_size,
        cfg.max_workers,
        cfg.max_convex,
        cfg.reuse_cache,
        cfg.debug_log.clone(),
        derived_preset,
    );
    opts
}

fn validate_and_build_options(lang: UiLang, config_path: Option<PathBuf>) -> Result<PipelineOptions, String> {
    let get_s = |id: i32| -> String {
        get_ctrl_hwnd(id).map(|h| get_ctrl_text(h).trim().to_string()).unwrap_or_default()
    };
    let get_chk = |id: i32| -> bool { get_ctrl_hwnd(id).map(get_chk_state).unwrap_or(false) };

    let world_s = get_s(ID_EDIT_WORLD);
    if world_s.is_empty() || !Path::new(&world_s).is_dir() {
        return Err(match lang {
            UiLang::Ru => "Папка сохранения мира Minecraft не существует.".to_string(),
            UiLang::En => "Minecraft world save directory does not exist.".to_string(),
        });
    }

    let parse_coord = |id: i32, label: &str| -> Result<i32, String> {
        get_s(id).parse::<i32>().map_err(|_| match lang {
            UiLang::Ru => format!("Некорректная координата {}: введите целое число.", label),
            UiLang::En => format!("Invalid {} coordinate: enter an integer.", label),
        })
    };
    let min_x = parse_coord(ID_EDIT_COORD_X1, "Min X")?;
    let min_y = parse_coord(ID_EDIT_COORD_Y1, "Min Y")?;
    let min_z = parse_coord(ID_EDIT_COORD_Z1, "Min Z")?;
    let max_x = parse_coord(ID_EDIT_COORD_X2, "Max X")?;
    let max_y = parse_coord(ID_EDIT_COORD_Y2, "Max Y")?;
    let max_z = parse_coord(ID_EDIT_COORD_Z2, "Max Z")?;

    let scale_raw = get_s(ID_EDIT_SCALE).replace(',', ".");
    let scale = scale_raw.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).ok_or_else(|| {
        match lang {
            UiLang::Ru => "Некорректный масштаб: введите положительное число (например, 39.37).".to_string(),
            UiLang::En => "Invalid scale: enter a positive number (e.g. 39.37).".to_string(),
        }
    })?;

    let name = get_s(ID_EDIT_NAME);
    if name.is_empty() {
        return Err(match lang {
            UiLang::Ru => "Укажите имя модели.".to_string(),
            UiLang::En => "Model name cannot be empty.".to_string(),
        });
    }
    if is_dos_reserved(&name) {
        return Err(match lang {
            UiLang::Ru => format!("Имя модели '{}' является зарезервированным именем Windows.", name),
            UiLang::En => format!("Model name '{}' is a reserved Windows device name.", name),
        });
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(match lang {
            UiLang::Ru => format!("Имя модели '{}' должно содержать только латинские буквы, цифры, '_' и '-'.", name),
            UiLang::En => format!("Model name '{}' must contain only ASCII letters, digits, '_' and '-'.", name),
        });
    }

    let raw_cat = get_s(ID_EDIT_CAT);
    let cat = raw_cat.trim_matches(&['/', '\\'][..]).replace('\\', "/");
    if cat.is_empty() {
        return Err(match lang {
            UiLang::Ru => "Укажите категорию модели.".to_string(),
            UiLang::En => "Category cannot be empty.".to_string(),
        });
    }
    if !raw_cat.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '\\')) {
        return Err(match lang {
            UiLang::Ru => format!("Категория '{}' содержит недопустимые символы (разрешены латиница, цифры, '_', '-', '/').", raw_cat),
            UiLang::En => format!("Category '{}' contains invalid characters (allowed: ASCII letters, digits, '_', '-', '/').", raw_cat),
        });
    }
    for part in cat.split('/').filter(|p| !p.is_empty()) {
        if is_dos_reserved(part) {
            return Err(match lang {
                UiLang::Ru => format!("Часть категории '{}' является зарезервированным именем Windows.", part),
                UiLang::En => format!("Category segment '{}' is a reserved Windows device name.", part),
            });
        }
    }

    let map_name = get_s(ID_EDIT_MAP);
    if map_name.is_empty() {
        return Err(match lang {
            UiLang::Ru => "Укажите имя карты (.bsp).".to_string(),
            UiLang::En => "Map name (.bsp) cannot be empty.".to_string(),
        });
    }
    if is_dos_reserved(&map_name) {
        return Err(match lang {
            UiLang::Ru => format!("Имя карты '{}' является зарезервированным именем Windows.", map_name),
            UiLang::En => format!("Map name '{}' is a reserved Windows device name.", map_name),
        });
    }
    if !map_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(match lang {
            UiLang::Ru => format!("Имя карты '{}' должно содержать только латинские буквы, цифры, '_' и '-'.", map_name),
            UiLang::En => format!("Map name '{}' must contain only ASCII letters, digits, '_' and '-'.", map_name),
        });
    }

    let out_dir = get_s(ID_EDIT_OUTDIR).trim_matches(&['"', '\''][..]).trim().to_string();
    if out_dir.is_empty() {
        return Err(match lang {
            UiLang::Ru => "Укажите папку вывода.".to_string(),
            UiLang::En => "Output directory cannot be empty.".to_string(),
        });
    }
    if out_dir.chars().any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*') || (c as u32) < 32) {
        return Err(match lang {
            UiLang::Ru => format!("Папка вывода '{}' содержит недопустимые символы (< > \" | ? *).", out_dir),
            UiLang::En => format!("Output directory '{}' contains illegal characters (< > \" | ? *).", out_dir),
        });
    }
    for comp in Path::new(&out_dir).components() {
        if let std::path::Component::Normal(os_part) = comp {
            let part = os_part.to_string_lossy();
            if is_dos_reserved(&part) {
                return Err(match lang {
                    UiLang::Ru => format!("Путь вывода содержит зарезервированное имя Windows '{}'.", part),
                    UiLang::En => format!("Output directory contains reserved Windows device name '{}'.", part),
                });
            }
        }
    }

    let dim_idx = get_ctrl_hwnd(ID_COMBO_DIM)
        .map(|h| unsafe { SendMessageW(h as HWND, CB_GETCURSEL, 0, 0) })
        .unwrap_or(0);
    let light_idx = get_ctrl_hwnd(ID_COMBO_LIGHT)
        .map(|h| unsafe { SendMessageW(h as HWND, CB_GETCURSEL, 0, 0) })
        .unwrap_or(0);
    let (build_preset, custom_dim, cli_cs, cli_mw, cli_mc, cli_rc, cli_dl) = GUI_STATE
        .get()
        .and_then(|l| l.lock().ok())
        .map(|st| {
            (
                st.build_preset.clone(),
                st.custom_dimension.clone(),
                st.cli_chunk_size,
                st.cli_max_workers,
                st.cli_max_convex,
                st.cli_reuse_cache,
                st.cli_debug_log.clone(),
            )
        })
        .unwrap_or_else(|| ("balanced".to_string(), None, None, None, None, None, None));

    let mut opts = PipelineOptions {
        world_dir: PathBuf::from(&world_s),
        coords: crate::anvil::WorldCoords {
            min_x,
            min_y,
            min_z,
            max_x,
            max_y,
            max_z,
        }
        .normalized(),
        dimension: dim_str_from_index(dim_idx, custom_dim.as_deref()),
        name,
        category: cat,
        map_name,
        output_dir: PathBuf::from(&out_dir),
        config_path: find_config_json(config_path.as_deref()),
        import_to_gmod: get_chk(ID_CHK_IMPORT_GMOD),
        export_vmf: get_chk(ID_CHK_EXPORT_VMF),
        lighting_style: light_str_from_index(light_idx).to_string(),
        hq_light: get_chk(ID_CHK_HQ_LIGHT),
        fast_vrad: get_chk(ID_CHK_FAST_VRAD),
        mc_physics: get_chk(ID_CHK_MC_PHYSICS),
        force_vmf_brushes: get_chk(ID_CHK_VMF_BRUSHES),
        scale,
        ..Default::default()
    };
    apply_cli_overrides_to_pipeline_options(
        &mut opts,
        cli_cs,
        cli_mw,
        cli_mc,
        cli_rc,
        cli_dl,
        &build_preset,
    );
    Ok(opts)
}

/// Formats a caught panic payload into a human-readable error string.
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown internal panic".to_string()
    }
}

pub fn format_done_summary(lang: UiLang, summary: &crate::PipelineSummary) -> String {
    match (lang, &summary.gma_error) {
        (UiLang::Ru, Some(err)) => format!(
            "Частичный успех за {:.2}с: BSP готов ({}), но создать GMA не удалось: {}\nЧанки: {}, Двери: {}",
            summary.total_secs,
            summary.bsp_path.display(),
            err,
            summary.chunk_count,
            summary.door_count
        ),
        (UiLang::En, Some(err)) => format!(
            "Partial success in {:.2}s: BSP ready ({}), but creating GMA failed: {}\nChunks: {}, Doors: {}",
            summary.total_secs,
            summary.bsp_path.display(),
            err,
            summary.chunk_count,
            summary.door_count
        ),
        (UiLang::Ru, None) => {
            let gma_line = summary
                .gma_path
                .as_ref()
                .map(|p| format!("\nGMA-аддон: {}", p.display()))
                .unwrap_or_default();
            format!(
                "Готово за {:.2}с! BSP: {}{}\nЧанки: {}, Двери: {}",
                summary.total_secs,
                summary.bsp_path.display(),
                gma_line,
                summary.chunk_count,
                summary.door_count
            )
        }
        (UiLang::En, None) => {
            let gma_line = summary
                .gma_path
                .as_ref()
                .map(|p| format!("\nGMA Addon: {}", p.display()))
                .unwrap_or_default();
            format!(
                "Done in {:.2}s! BSP: {}{}\nChunks: {}, Doors: {}",
                summary.total_secs,
                summary.bsp_path.display(),
                gma_line,
                summary.chunk_count,
                summary.door_count
            )
        }
    }
}

fn handle_start_clicked() {
    let Some(handles) = GUI_HANDLES.get() else { return };
    let (lang, busy, cfg_path) = {
        let Some(lock) = GUI_STATE.get() else { return };
        // Poisoning-safe: if the state lock was poisoned by an earlier panic,
        // bail out instead of panicking on the GUI thread.
        let Ok(st) = lock.lock() else { return };
        (st.lang, st.busy, st.config_path.clone())
    };
    if busy {
        return;
    }

    match validate_and_build_options(lang, cfg_path) {
        Ok(mut opts) => {
            save_gui_state();
            let cancel_flag = {
                let Some(lock) = GUI_STATE.get() else { return };
                let Ok(mut st) = lock.lock() else { return };
                if st.busy {
                    return;
                }
                st.busy = true;
                st.status_key = "status_running";
                st.custom_status_text = None;
                st.cancel_flag = Arc::new(AtomicBool::new(false));
                st.cancel_flag.clone()
            };
            opts.cancel_flag = Some(cancel_flag.clone());

            if let Some(&hb) = handles.controls.get(&ID_BTN_START) {
                unsafe { EnableWindow(hb as HWND, 0); }
                set_ctrl_text(hb, tr_text(lang, "btn_running"));
            }
            if let Some(&hs) = handles.controls.get(&ID_BTN_STOP) {
                unsafe {
                    ShowWindow(hs as HWND, SW_SHOW);
                    EnableWindow(hs as HWND, 1);
                }
            }
            unsafe {
                let mut cr: RECT = std::mem::zeroed();
                GetClientRect(handles.hwnd as HWND, &mut cr);
                layout_controls(cr.right - cr.left, cr.bottom - cr.top);
            }
            if let Some(&hp) = handles.controls.get(&ID_PROGRESS) {
                unsafe { SendMessageW(hp as HWND, PBM_SETPOS, 2, 0); }
            }
            if let Some(&hl) = handles.controls.get(&ID_LBL_STATUS) {
                set_ctrl_text(hl, tr_text(lang, "status_running"));
            }
            if let Some(&hlog) = handles.controls.get(&ID_LOG) {
                set_ctrl_text(hlog, "");
                append_log_line(
                    hlog,
                    &format!(
                        "[GUI] Starting export: world='{}', dim='{}', scale={:.2}, map='{}'",
                        opts.world_dir.display(),
                        opts.dimension,
                        opts.scale,
                        opts.map_name
                    ),
                );
            }

            let hwnd_val = handles.hwnd;
            {
                let batch = PENDING_LOG_BATCH.get_or_init(|| Mutex::new((0, None, Vec::new())));
                if let Ok(mut b) = batch.lock() {
                    b.0 = 0;
                    b.1 = None;
                    b.2.clear();
                }
            }
            unsafe {
                SetTimer(hwnd_val as HWND, ID_TIMER_LOG, 50, None);
            }
            let spawn_res = std::thread::Builder::new()
                .name("mcgm-export".to_string())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || {
                let target_hwnd = hwnd_val as HWND;
                let cancel_flag_final = cancel_flag.clone();
                let cancel_in_cb = cancel_flag.clone();
                // A panic inside the export pipeline (including rayon worker panics,
                // which rayon re-raises on the joining thread) must not take down the
                // GUI: catch it and report it through the regular WM_APP_DONE path.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_export_pipeline(opts, |pct, msg| {
                        if cancel_in_cb.load(Ordering::Relaxed) {
                            return;
                        }
                        let batch = PENDING_LOG_BATCH.get_or_init(|| Mutex::new((0, None, Vec::new())));
                        if let Ok(mut b) = batch.lock() {
                            b.0 = pct as i32;
                            b.1 = Some(msg.to_string());
                            b.2.push(format!("[{}%] {}", pct, msg));
                        }
                    })
                }))
                .unwrap_or_else(|panic_payload| Err(panic_message(panic_payload)));
                let msg_res: Result<(String, crate::ExportOutcomeStatus), String> =
                    if cancel_flag_final.load(Ordering::SeqCst) {
                        Err("__CANCELLED__".to_string())
                    } else {
                        match outcome {
                            Ok(summary) => {
                                let status = summary.outcome_status();
                                Ok((format_done_summary(lang, &summary), status))
                            }
                            Err(e) => Err(e),
                        }
                    };
                let ptr = Box::into_raw(Box::new(msg_res));
                unsafe {
                    if PostMessageW(target_hwnd, WM_APP_DONE, 0, ptr as isize) == 0 {
                        drop(Box::from_raw(ptr));
                    }
                }
            });
            if let Err(e) = spawn_res {
                unsafe {
                    KillTimer(hwnd_val as HWND, ID_TIMER_LOG);
                }
                if let Some(lock) = GUI_STATE.get() {
                    if let Ok(mut st) = lock.lock() {
                        st.busy = false;
                        st.status_key = "status_failed";
                        st.custom_status_text = None;
                    }
                }
                if let Some(&hb) = handles.controls.get(&ID_BTN_START) {
                    unsafe { EnableWindow(hb as HWND, 1); }
                    set_ctrl_text(hb, tr_text(lang, "btn_start"));
                }
                if let Some(&hs) = handles.controls.get(&ID_BTN_STOP) {
                    unsafe { ShowWindow(hs as HWND, SW_HIDE); }
                }
                if let Some(&hl) = handles.controls.get(&ID_LBL_STATUS) {
                    set_ctrl_text(hl, tr_text(lang, "status_failed"));
                }
                let wt = to_wide(tr_text(lang, "err_title"));
                let wm = to_wide(&format!("Failed to start export worker thread: {}", e));
                unsafe {
                    MessageBoxW(hwnd_val as HWND, wm.as_ptr(), wt.as_ptr(), MB_OK | MB_ICONERROR);
                }
            }
        }
        Err(msg) => {
            let wt = to_wide(tr_text(lang, "err_title"));
            let wm = to_wide(&msg);
            unsafe {
                MessageBoxW(handles.hwnd as HWND, wm.as_ptr(), wt.as_ptr(), MB_OK | MB_ICONWARNING);
            }
        }
    }
}

fn apply_edit_margins(dpi: u32) {
    let margin = scale_px(6, dpi).clamp(2, 24) as usize;
    let lparam_val = ((margin << 16) | margin) as isize;
    for id in [
        ID_EDIT_COORD_X1,
        ID_EDIT_COORD_Y1,
        ID_EDIT_COORD_Z1,
        ID_EDIT_COORD_X2,
        ID_EDIT_COORD_Y2,
        ID_EDIT_COORD_Z2,
        ID_EDIT_MAP,
        ID_EDIT_WORLD,
        ID_EDIT_SCALE,
        ID_EDIT_NAME,
        ID_EDIT_CAT,
        ID_EDIT_OUTDIR,
    ] {
        if let Some(h) = get_ctrl_hwnd(id) {
            unsafe {
                SendMessageW(h as HWND, EM_SETMARGINS, EC_LEFTMARGIN | EC_RIGHTMARGIN, lparam_val);
            }
        }
    }
}

fn apply_fonts_to_all_controls(fonts: ThemeFonts, dpi: u32) {
    let Some(handles) = GUI_HANDLES.get() else {
        return;
    };
    for (&id, &h) in &handles.controls {
        let f = match id {
            ID_LBL_TITLE => fonts.title,
            ID_LBL_LOG_HDR => fonts.bold,
            ID_LOG => fonts.log,
            _ => fonts.regular,
        };
        if f != 0 {
            unsafe {
                SendMessageW(h as HWND, WM_SETFONT, f as usize, 1);
            }
        }
    }
    apply_edit_margins(dpi);
}

fn apply_window_theme(hwnd: HWND) {
    let pal = light_palette();
    if let Some(lock) = GUI_STATE.get() {
        if let Ok(mut st) = lock.lock() {
            st.brushes.destroy();
            st.brushes = ThemeBrushes::create(&pal);
        }
    }

    unsafe {
        let dark_val: i32 = 0;
        let _ = DwmSetWindowAttribute(
            hwnd,
            20, // DWMWA_USE_IMMERSIVE_DARK_MODE
            (&dark_val as *const i32) as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            19,
            (&dark_val as *const i32) as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        let corner_pref: i32 = 2; // DWMWCP_ROUND
        let _ = DwmSetWindowAttribute(
            hwnd,
            33, // DWMWA_WINDOW_CORNER_PREFERENCE
            (&corner_pref as *const i32) as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        let cap_color: u32 = pal.canvas_bg;
        let _ = DwmSetWindowAttribute(
            hwnd,
            35, // DWMWA_CAPTION_COLOR
            (&cap_color as *const u32) as *const std::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );

        let cfd_theme = to_wide("CFD");
        for id in [
            ID_COMBO_INSTANCES,
            ID_COMBO_WORLDS,
            ID_COMBO_PRESET,
            ID_COMBO_LIGHT,
            ID_COMBO_DIM,
        ] {
            if let Some(h) = get_ctrl_hwnd(id) {
                SetWindowTheme(h as HWND, cfd_theme.as_ptr(), std::ptr::null());
            }
        }
        let exp_theme = to_wide("Explorer");
        if let Some(h) = get_ctrl_hwnd(ID_LOG) {
            SetWindowTheme(h as HWND, exp_theme.as_ptr(), std::ptr::null());
        }

        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
    if let Some(handles) = GUI_HANDLES.get() {
        for &h in handles.controls.values() {
            unsafe {
                InvalidateRect(h as HWND, std::ptr::null(), 1);
            }
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            let hdc = wparam as HDC;
            if !hdc.is_null() {
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let pal = light_palette();
                let canvas_br = GUI_STATE
                    .get()
                    .and_then(|l| l.try_lock().ok())
                    .map(|s| s.brushes.canvas)
                    .unwrap_or(0);
                if canvas_br != 0 {
                    FillRect(hdc, &rc, canvas_br as _);
                } else {
                    let bg_br = CreateSolidBrush(pal.canvas_bg);
                    FillRect(hdc, &rc, bg_br);
                    DeleteObject(bg_br as _);
                }
            }
            return 1;
        }
        WM_PAINT => {
            let pal = light_palette();
            let canvas_br = GUI_STATE
                .get()
                .and_then(|l| l.try_lock().ok())
                .map(|s| s.brushes.canvas)
                .unwrap_or(0);
            let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = std::mem::zeroed();
            let hdc = windows_sys::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            if canvas_br != 0 {
                FillRect(hdc, &ps.rcPaint, canvas_br as _);
            } else {
                let bg_br = CreateSolidBrush(pal.canvas_bg);
                FillRect(hdc, &ps.rcPaint, bg_br);
                DeleteObject(bg_br as _);
            }
            windows_sys::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            return 0;
        }
        WM_DPICHANGED_MSG => {
            let new_dpi = (wparam & 0xFFFF) as u32;
            if new_dpi >= 72 {
                let new_fonts = ThemeFonts::create(new_dpi);
                if let Some(lock) = GUI_STATE.get() {
                    if let Ok(mut st) = lock.lock() {
                        st.dpi = new_dpi;
                        st.fonts.destroy();
                        st.fonts = new_fonts;
                    }
                }
                apply_fonts_to_all_controls(new_fonts, new_dpi);
                if lparam != 0 {
                    let sr = &*(lparam as *const RECT);
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        sr.left,
                        sr.top,
                        sr.right - sr.left,
                        sr.bottom - sr.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                let mut cr: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut cr);
                layout_controls(cr.right - cr.left, cr.bottom - cr.top);
                RedrawWindow(
                    hwnd,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
                );
            }
            return 0;
        }
        WM_GETMINMAXINFO => {
            if lparam != 0 {
                let (mode, dpi) = GUI_STATE
                    .get()
                    .and_then(|l| l.try_lock().ok())
                    .map(|s| (s.layout_mode, s.dpi))
                    .unwrap_or((GuiLayoutMode::default(), 96));
                let (min_cw_96, min_ch_96) = compute_min_client_size(mode);
                let min_cw = scale_px(min_cw_96, dpi);
                let min_ch = scale_px(min_ch_96, dpi);
                let (nc_w, nc_h) = if !hwnd.is_null() {
                    let mut wr: RECT = std::mem::zeroed();
                    let mut cr: RECT = std::mem::zeroed();
                    if GetWindowRect(hwnd, &mut wr) != 0 && GetClientRect(hwnd, &mut cr) != 0 {
                        let cw = cr.right - cr.left;
                        let ch = cr.bottom - cr.top;
                        if cw > 0 && ch > 0 {
                            ((wr.right - wr.left) - cw, (wr.bottom - wr.top) - ch)
                        } else {
                            (16, 39)
                        }
                    } else {
                        (16, 39)
                    }
                } else {
                    (16, 39)
                };
                let mmi = &mut *(lparam as *mut MINMAXINFO);
                mmi.ptMinTrackSize.x = min_cw + nc_w;
                mmi.ptMinTrackSize.y = min_ch + nc_h;
            }
            return 0;
        }
        WM_SIZE => {
            let cw = (lparam & 0xFFFF) as i16 as i32;
            let ch = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            if cw > 0 && ch > 0 {
                layout_controls(cw, ch);
                RedrawWindow(
                    hwnd,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
                );
            }
            return 0;
        }
        WM_CTLCOLORSTATIC => {
            let (canvas_br, log_br) = GUI_STATE
                .get()
                .and_then(|l| l.try_lock().ok())
                .map(|s| (s.brushes.canvas, s.brushes.log))
                .unwrap_or((0, 0));
            let pal = light_palette();
            let ctrl = lparam;
            let hdc = wparam as HDC;
            if let Some(h_log) = get_ctrl_hwnd(ID_LOG) {
                if ctrl == h_log && log_br != 0 {
                    SetBkColor(hdc, pal.log_bg);
                    SetTextColor(hdc, pal.log_text);
                    return log_br;
                }
            }
            let ctrl_id = GetDlgCtrlID(ctrl as HWND);
            let text_col = match ctrl_id {
                ID_LBL_SEL_INFO | ID_LBL_STATUS => pal.text_secondary,
                _ => pal.text_primary,
            };
            SetBkMode(hdc, OPAQUE as i32);
            SetBkColor(hdc, pal.canvas_bg);
            SetTextColor(hdc, text_col);
            if canvas_br != 0 {
                return canvas_br;
            }
            return GetStockObject(NULL_BRUSH) as isize;
        }
        WM_CTLCOLORBTN => {
            let canvas_br = GUI_STATE
                .get()
                .and_then(|l| l.try_lock().ok())
                .map(|s| s.brushes.canvas)
                .unwrap_or(0);
            let pal = light_palette();
            let hdc = wparam as HDC;
            SetBkMode(hdc, OPAQUE as i32);
            SetBkColor(hdc, pal.canvas_bg);
            SetTextColor(hdc, pal.text_primary);
            if canvas_br != 0 {
                return canvas_br;
            }
            return GetStockObject(NULL_BRUSH) as isize;
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let (input_br, log_br) = GUI_STATE
                .get()
                .and_then(|l| l.try_lock().ok())
                .map(|s| (s.brushes.input, s.brushes.log))
                .unwrap_or((0, 0));
            let pal = light_palette();
            let hdc = wparam as HDC;
            if get_ctrl_hwnd(ID_LOG).is_some_and(|h| h == lparam) && log_br != 0 {
                SetBkColor(hdc, pal.log_bg);
                SetTextColor(hdc, pal.log_text);
                return log_br;
            }
            if input_br != 0 {
                SetBkColor(hdc, pal.input_bg);
                SetTextColor(hdc, pal.text_primary);
                return input_br;
            }
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            let code = ((wparam >> 16) & 0xFFFF) as u32;

            if code == EN_CHANGE
                && matches!(
                    id,
                    ID_EDIT_COORD_X1
                        | ID_EDIT_COORD_Y1
                        | ID_EDIT_COORD_Z1
                        | ID_EDIT_COORD_X2
                        | ID_EDIT_COORD_Y2
                        | ID_EDIT_COORD_Z2
                )
            {
                let lang = GUI_STATE
                    .get()
                    .and_then(|l| l.try_lock().ok())
                    .map(|s| s.lang)
                    .unwrap_or(UiLang::En);
                update_selection_info_label(lang);
                return 0;
            }

            if code == EN_CHANGE && id == ID_EDIT_MAP {
                let (suppress, manual) = GUI_STATE
                    .get()
                    .and_then(|l| l.try_lock().ok())
                    .map(|s| (s.suppress_name_sync, s.model_name_manually_edited))
                    .unwrap_or((false, false));
                if !suppress && !manual {
                    if let (Some(hm), Some(hn)) = (get_ctrl_hwnd(ID_EDIT_MAP), get_ctrl_hwnd(ID_EDIT_NAME)) {
                        let derived = derive_model_name_from_map(&get_ctrl_text(hm));
                        if let Some(lock) = GUI_STATE.get() {
                            if let Ok(mut st) = lock.try_lock() {
                                st.suppress_name_sync = true;
                            }
                        }
                        set_ctrl_text(hn, &derived);
                        if let Some(lock) = GUI_STATE.get() {
                            if let Ok(mut st) = lock.try_lock() {
                                st.suppress_name_sync = false;
                            }
                        }
                    }
                }
                return 0;
            }

            if code == EN_CHANGE && id == ID_EDIT_NAME {
                if let Some(lock) = GUI_STATE.get() {
                    if let Ok(mut st) = lock.try_lock() {
                        if !st.suppress_name_sync {
                            st.model_name_manually_edited = true;
                        }
                    }
                }
                return 0;
            }

            if id == ID_BTN_TOGGLE_ADV && code == BN_CLICKED {
                let next = GUI_STATE
                    .get()
                    .and_then(|l| l.lock().ok())
                    .map(|s| GuiLayoutMode {
                        show_advanced: !s.layout_mode.show_advanced,
                        show_log: s.layout_mode.show_log,
                    })
                    .unwrap_or_default();
                set_panel_mode(hwnd, next);
                return 0;
            }

            if id == ID_BTN_TOGGLE_LOG && code == BN_CLICKED {
                let next = GUI_STATE
                    .get()
                    .and_then(|l| l.lock().ok())
                    .map(|s| GuiLayoutMode {
                        show_advanced: s.layout_mode.show_advanced,
                        show_log: !s.layout_mode.show_log,
                    })
                    .unwrap_or_default();
                set_panel_mode(hwnd, next);
                return 0;
            }

            if id == ID_BTN_BROWSE_WORLD && code == BN_CLICKED {
                let (lang, initial_dir) = {
                    let cur_world = get_ctrl_hwnd(ID_EDIT_WORLD)
                        .map(|hw| get_ctrl_text(hw).trim().to_string())
                        .filter(|s| !s.is_empty());
                    GUI_STATE
                        .get()
                        .and_then(|l| l.lock().ok())
                        .map(|s| {
                            let init = resolve_browse_initial_dir(
                                &s.discovered_instances,
                                s.selected_instance_idx,
                                cur_world.as_deref().map(Path::new),
                            );
                            (s.lang, init)
                        })
                        .unwrap_or((UiLang::En, None))
                };
                if let Some(path) = browse_folder(hwnd, tr_text(lang, "browse_world"), initial_dir.as_deref()) {
                    if let Some(hw) = get_ctrl_hwnd(ID_EDIT_WORLD) {
                        set_ctrl_text(hw, &path.to_string_lossy());
                    }
                    sync_worlds_combo(lang, Some(&path));
                    populate_from_world_path(&path, true);
                }
                return 0;
            }
            if id == ID_BTN_ADD_INSTANCE && code == BN_CLICKED {
                let lang = GUI_STATE
                    .get()
                    .and_then(|l| l.lock().ok())
                    .map(|s| s.lang)
                    .unwrap_or(UiLang::En);
                if let Some(dir) = browse_folder(hwnd, tr_text(lang, "browse_instance"), None) {
                    let clean_dir = crate::instances::strip_windows_verbatim_prefix(&dir);
                    let probed = crate::instances::scan_custom_dir(&clean_dir);
                    if probed.is_empty() {
                        let msg = tr_text(lang, "instances_added_none");
                        if let Some(lock) = GUI_STATE.get() {
                            if let Ok(mut st) = lock.lock() {
                                st.custom_status_text = Some(msg.to_string());
                            }
                        }
                        if let Some(hl) = get_ctrl_hwnd(ID_LBL_STATUS) {
                            set_ctrl_text(hl, msg);
                        }
                        return 0;
                    }
                    let first_saves = probed[0].saves_dir.to_string_lossy().into_owned();
                    let (cfg_path, extra_dirs) = {
                        let Some(lock) = GUI_STATE.get() else { return 0 };
                        let Ok(mut st) = lock.lock() else { return 0 };
                        let low = clean_dir.to_string_lossy().to_lowercase();
                        if !st
                            .custom_instance_dirs
                            .iter()
                            .any(|p| p.to_string_lossy().to_lowercase() == low)
                        {
                            st.custom_instance_dirs.push(clean_dir);
                        }
                        st.pending_saved_instance_saves = Some(first_saves);
                        (st.config_path.clone(), st.custom_instance_dirs.clone())
                    };
                    save_gui_state();
                    let bg_hwnd = hwnd as isize;
                    std::thread::spawn(move || {
                        let found = crate::instances::discover_all_instances(
                            cfg_path.as_deref(),
                            &extra_dirs,
                        );
                        let ptr = Box::into_raw(Box::new(found));
                        if PostMessageW(bg_hwnd as HWND, WM_APP_WORLDS_READY, 0, ptr as isize) == 0 {
                            drop(Box::from_raw(ptr));
                        }
                    });
                }
                return 0;
            }
            if id == ID_COMBO_INSTANCES && code == CBN_SELCHANGE {
                let Some(hc) = get_ctrl_hwnd(ID_COMBO_INSTANCES) else {
                    return 0;
                };
                let sel = SendMessageW(hc as HWND, CB_GETCURSEL, 0, 0);
                let (lang, first_world_to_select) = {
                    let Some(lock) = GUI_STATE.get() else { return 0 };
                    let Ok(mut st) = lock.lock() else { return 0 };
                    if st.suppress_instance_combo {
                        return 0;
                    }
                    st.selected_instance_idx = if sel > 0 && ((sel as usize) - 1) < st.discovered_instances.len() {
                        Some((sel as usize) - 1)
                    } else {
                        None
                    };
                    st.pending_saved_instance_saves = st
                        .selected_instance_idx
                        .and_then(|i| st.discovered_instances.get(i))
                        .map(|inst| inst.saves_dir.to_string_lossy().into_owned());
                    let cur_world = get_ctrl_hwnd(ID_EDIT_WORLD)
                        .map(|hw| get_ctrl_text(hw).trim().to_string())
                        .unwrap_or_default();
                    let cur_low = cur_world.to_lowercase();
                    let next_world = st.selected_instance_idx.and_then(|idx| {
                        let inst = st.discovered_instances.get(idx)?;
                        let already_in_inst = !cur_low.is_empty()
                            && inst
                                .worlds
                                .iter()
                                .any(|w| w.path.to_string_lossy().to_lowercase() == cur_low);
                        if already_in_inst {
                            None
                        } else {
                            inst.worlds.first().map(|w| w.path.clone())
                        }
                    });
                    (st.lang, next_world)
                };
                if let Some(ref wp) = first_world_to_select {
                    if let Some(hw) = get_ctrl_hwnd(ID_EDIT_WORLD) {
                        set_ctrl_text(hw, &wp.to_string_lossy());
                    }
                    sync_worlds_combo(lang, Some(wp));
                    populate_from_world_path(wp, true);
                } else {
                    let cur_world = get_ctrl_hwnd(ID_EDIT_WORLD)
                        .map(|hw| get_ctrl_text(hw).trim().to_string())
                        .filter(|s| !s.is_empty());
                    sync_worlds_combo(lang, cur_world.as_deref().map(Path::new));
                }
                return 0;
            }
            if id == ID_BTN_BROWSE_OUT && code == BN_CLICKED {
                let lang = GUI_STATE.get().and_then(|l| l.lock().ok()).map(|s| s.lang).unwrap_or(UiLang::En);
                if let Some(path) = browse_folder(hwnd, tr_text(lang, "browse_out"), None) {
                    if let Some(ho) = get_ctrl_hwnd(ID_EDIT_OUTDIR) {
                        set_ctrl_text(ho, &path.to_string_lossy());
                    }
                }
                return 0;
            }
            if id == ID_COMBO_WORLDS && code == CBN_SELCHANGE {
                let selected_path = GUI_STATE.get().and_then(|l| l.lock().ok()).and_then(|st| {
                    if st.suppress_world_combo {
                        return None;
                    }
                    let hc = get_ctrl_hwnd(ID_COMBO_WORLDS)?;
                    let idx = SendMessageW(hc as HWND, CB_GETCURSEL, 0, 0);
                    if idx >= 0 && (idx as usize) < st.discovered_worlds.len() {
                        Some(st.discovered_worlds[idx as usize].1.clone())
                    } else {
                        None
                    }
                });
                if let Some(path) = selected_path {
                    if let Some(hw) = get_ctrl_hwnd(ID_EDIT_WORLD) {
                        set_ctrl_text(hw, &path.to_string_lossy());
                    }
                    populate_from_world_path(&path, true);
                }
                return 0;
            }
            if id == ID_COMBO_PRESET && code == CBN_SELCHANGE {
                if let Some(hp) = get_ctrl_hwnd(ID_COMBO_PRESET) {
                    let idx = SendMessageW(hp as HWND, CB_GETCURSEL, 0, 0);
                    apply_build_preset_ui(preset_str_from_index(idx));
                }
                return 0;
            }
            if (id == ID_CHK_IMPORT_GMOD
                || id == ID_CHK_EXPORT_VMF
                || id == ID_CHK_HQ_LIGHT
                || id == ID_CHK_FAST_VRAD
                || id == ID_CHK_MC_PHYSICS
                || id == ID_CHK_VMF_BRUSHES)
                && code == BN_CLICKED
            {
                if id == ID_CHK_FAST_VRAD || id == ID_CHK_HQ_LIGHT {
                    sync_preset_from_checkboxes_ui();
                }
                return 0;
            }
            if id == ID_BTN_RAD_32 && code == BN_CLICKED {
                apply_radius_preset(32);
                return 0;
            }
            if id == ID_BTN_RAD_64 && code == BN_CLICKED {
                apply_radius_preset(64);
                return 0;
            }
            if id == ID_BTN_RAD_128 && code == BN_CLICKED {
                apply_radius_preset(128);
                return 0;
            }
            if id == ID_BTN_CLEAR && code == BN_CLICKED {
                if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
                    set_ctrl_text(hlog, "");
                }
                return 0;
            }
            if id == ID_BTN_STOP && code == BN_CLICKED {
                let lang = if let Some(lock) = GUI_STATE.get() {
                    if let Ok(mut st) = lock.lock() {
                        st.cancel_flag.store(true, Ordering::SeqCst);
                        st.status_key = "status_cancelling";
                        st.custom_status_text = None;
                        st.lang
                    } else {
                        UiLang::En
                    }
                } else {
                    UiLang::En
                };
                if let Some(hs) = get_ctrl_hwnd(ID_BTN_STOP) {
                    EnableWindow(hs as HWND, 0);
                }
                if let Some(hl) = get_ctrl_hwnd(ID_LBL_STATUS) {
                    set_ctrl_text(hl, tr_text(lang, "status_cancelling"));
                }
                if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
                    append_log_line(hlog, "[GUI] Cancellation requested — waiting for current stage to finish...");
                }
                return 0;
            }
            if id == ID_BTN_OPEN_OUT && code == BN_CLICKED {
                let raw = get_ctrl_hwnd(ID_EDIT_OUTDIR)
                    .map(|h| get_ctrl_text(h).trim().trim_matches(&['"', '\''][..]).to_string())
                    .unwrap_or_default();
                let mut target = if raw.is_empty() {
                    PathBuf::from("output")
                } else {
                    PathBuf::from(&raw)
                };
                if target.is_relative() {
                    if let Ok(cwd) = std::env::current_dir() {
                        target = cwd.join(&target);
                    }
                }
                if !target.exists() {
                    let _ = std::fs::create_dir_all(&target);
                }
                shell_open_target(hwnd, &target.to_string_lossy());
                return 0;
            }
            if id == ID_BTN_COPY_MAP && code == BN_CLICKED {
                let raw_map = get_ctrl_hwnd(ID_EDIT_MAP)
                    .map(|h| get_ctrl_text(h).trim().to_string())
                    .unwrap_or_default();
                let safe_map = sanitize_identifier(&raw_map, "gm_mc_custom");
                let cmd = format!("map {}", safe_map);
                if copy_text_to_clipboard(hwnd, &cmd) {
                    let lang = GUI_STATE.get().and_then(|l| l.lock().ok()).map(|s| s.lang).unwrap_or(UiLang::En);
                    let status = match lang {
                        UiLang::Ru => format!("Скопировано: {}", cmd),
                        UiLang::En => format!("Copied: {}", cmd),
                    };
                    if let Some(lock) = GUI_STATE.get() {
                        if let Ok(mut st) = lock.lock() {
                            st.custom_status_text = Some(status.clone());
                        }
                    }
                    if let Some(hl) = get_ctrl_hwnd(ID_LBL_STATUS) {
                        set_ctrl_text(hl, &status);
                    }
                    if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
                        append_log_line(hlog, &format!("[GUI] Copied to clipboard: {}", cmd));
                    }
                }
                return 0;
            }
            if id == ID_BTN_LAUNCH_GMOD && code == BN_CLICKED {
                let raw_map = get_ctrl_hwnd(ID_EDIT_MAP)
                    .map(|h| get_ctrl_text(h).trim().to_string())
                    .unwrap_or_default();
                let safe_map = sanitize_identifier(raw_map.strip_suffix(".bsp").unwrap_or(&raw_map), "gm_mc_custom");
                let url = format!("steam://rungameid/4000//+map {}", safe_map);
                shell_open_target(hwnd, &url);
                return 0;
            }
            if id == ID_BTN_LANG && code == BN_CLICKED {
                let (new_lang, busy, status_key, custom_status, preset) = {
                    let Some(lock) = GUI_STATE.get() else { return 0 };
                    let Ok(mut st) = lock.lock() else { return 0 };
                    st.lang = if st.lang == UiLang::Ru { UiLang::En } else { UiLang::Ru };
                    (
                        st.lang,
                        st.busy,
                        st.status_key,
                        st.custom_status_text.clone(),
                        st.build_preset.clone(),
                    )
                };
                apply_language(new_lang, busy, status_key, custom_status.as_deref(), &preset);
                return 0;
            }
            if id == ID_BTN_START && code == BN_CLICKED {
                handle_start_clicked();
                return 0;
            }
        }
        WM_TIMER => {
            if wparam == ID_TIMER_LOG {
                flush_pending_logs();
                return 0;
            }
        }
        WM_APP_WORLDS_READY => {
            let ptr = lparam as *mut Vec<crate::instances::MinecraftInstance>;
            if !ptr.is_null() {
                let found_instances = *Box::from_raw(ptr);
                let (lang, first_world_from_sel) = {
                    let Some(lock) = GUI_STATE.get() else { return 0 };
                    let Ok(mut st) = lock.lock() else { return 0 };
                    st.discovered_instances = found_instances;
                    if let Some(saved_saves) = st.pending_saved_instance_saves.take() {
                        let low = saved_saves.to_lowercase();
                        st.selected_instance_idx = st
                            .discovered_instances
                            .iter()
                            .position(|i| i.saves_dir.to_string_lossy().to_lowercase() == low);
                    } else if st
                        .selected_instance_idx
                        .is_some_and(|i| i >= st.discovered_instances.len())
                    {
                        st.selected_instance_idx = None;
                    }
                    let fw = st
                        .selected_instance_idx
                        .and_then(|i| st.discovered_instances.get(i))
                        .and_then(|inst| inst.worlds.first())
                        .map(|w| w.path.clone())
                        .or_else(|| {
                            st.discovered_instances
                                .iter()
                                .find_map(|inst| inst.worlds.first().map(|w| w.path.clone()))
                        });
                    (st.lang, fw)
                };
                sync_instances_combo(lang);
                let cur_world = get_ctrl_hwnd(ID_EDIT_WORLD)
                    .map(|hw| get_ctrl_text(hw).trim().to_string())
                    .unwrap_or_default();
                if cur_world.is_empty() {
                    sync_worlds_combo(lang, None);
                    if let Some(ref first_world) = first_world_from_sel {
                        if let Some(hw) = get_ctrl_hwnd(ID_EDIT_WORLD) {
                            set_ctrl_text(hw, &first_world.to_string_lossy());
                        }
                        populate_from_world_path(first_world, true);
                        update_selection_info_label(lang);
                    }
                } else {
                    sync_worlds_combo(lang, Some(Path::new(&cur_world)));
                }
            }
            return 0;
        }
        WM_APP_DONE => {
            KillTimer(hwnd, ID_TIMER_LOG);
            flush_pending_logs();
            let res_ptr = lparam as *mut Result<(String, crate::ExportOutcomeStatus), String>;
            if !res_ptr.is_null() {
                let res = *Box::from_raw(res_ptr);
                let (lang, cur_mode) = {
                    let Some(lock) = GUI_STATE.get() else { return 0 };
                    let Ok(mut st) = lock.lock() else { return 0 };
                    st.busy = false;
                    st.custom_status_text = None;
                    st.status_key = match &res {
                        Ok((_, crate::ExportOutcomeStatus::Success)) => "status_done",
                        Ok((_, crate::ExportOutcomeStatus::PartialSuccess)) => "status_partial",
                        Err(e) if e == "__CANCELLED__" => "status_cancelled",
                        Err(_) => "status_failed",
                    };
                    (st.lang, st.layout_mode)
                };

                if let Some(hb) = get_ctrl_hwnd(ID_BTN_START) {
                    EnableWindow(hb as HWND, 1);
                    set_ctrl_text(hb, tr_text(lang, "btn_start"));
                }
                if let Some(hs) = get_ctrl_hwnd(ID_BTN_STOP) {
                    EnableWindow(hs as HWND, 0);
                    ShowWindow(hs as HWND, SW_HIDE);
                }
                let mut cr: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut cr);
                layout_controls(cr.right - cr.left, cr.bottom - cr.top);

                // Prepare optional modal MessageBox parameters AFTER lock is dropped
                let modal: Option<(String, String, u32)> = match res {
                    Ok((msg, status)) => {
                        if let Some(hp) = get_ctrl_hwnd(ID_PROGRESS) {
                            SendMessageW(hp as HWND, PBM_SETPOS, 100, 0);
                        }
                        let is_partial = status == crate::ExportOutcomeStatus::PartialSuccess;
                        if let Some(hl) = get_ctrl_hwnd(ID_LBL_STATUS) {
                            set_ctrl_text(
                                hl,
                                tr_text(
                                    lang,
                                    if is_partial {
                                        "status_partial"
                                    } else {
                                        "status_done"
                                    },
                                ),
                            );
                        }
                        if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
                            append_log_line(hlog, &msg);
                        }
                        if is_partial && !cur_mode.show_log {
                            set_panel_mode(
                                hwnd,
                                GuiLayoutMode {
                                    show_advanced: cur_mode.show_advanced,
                                    show_log: true,
                                },
                            );
                        }
                        Some((
                            tr_text(
                                lang,
                                if is_partial {
                                    "partial_title"
                                } else {
                                    "done_title"
                                },
                            )
                            .to_string(),
                            msg,
                            if is_partial {
                                MB_OK | MB_ICONWARNING
                            } else {
                                MB_OK | MB_ICONINFORMATION
                            },
                        ))
                    }
                    Err(ref err) if err == "__CANCELLED__" => {
                        if let Some(hp) = get_ctrl_hwnd(ID_PROGRESS) {
                            SendMessageW(hp as HWND, PBM_SETPOS, 0, 0);
                        }
                        if let Some(hl) = get_ctrl_hwnd(ID_LBL_STATUS) {
                            set_ctrl_text(hl, tr_text(lang, "status_cancelled"));
                        }
                        if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
                            append_log_line(hlog, &format!("[GUI] {}", tr_text(lang, "status_cancelled")));
                        }
                        None
                    }
                    Err(err) => {
                        if let Some(hl) = get_ctrl_hwnd(ID_LBL_STATUS) {
                            set_ctrl_text(hl, tr_text(lang, "status_failed"));
                        }
                        if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
                            append_log_line(hlog, &format!("ERROR: {}", err));
                        }
                        if !cur_mode.show_log {
                            set_panel_mode(
                                hwnd,
                                GuiLayoutMode {
                                    show_advanced: cur_mode.show_advanced,
                                    show_log: true,
                                },
                            );
                        }
                        Some((
                            tr_text(lang, "export_err_title").to_string(),
                            err,
                            MB_OK | MB_ICONERROR,
                        ))
                    }
                };

                if let Some((title, body, flags)) = modal {
                    let wt = to_wide(&title);
                    let wm = to_wide(&body);
                    MessageBoxW(hwnd, wm.as_ptr(), wt.as_ptr(), flags);
                }
            }
            return 0;
        }
        WM_APP_AUTOSTART => {
            handle_start_clicked();
            return 0;
        }
        WM_CLOSE => {
            let (busy, lang) = GUI_STATE
                .get()
                .and_then(|l| l.lock().ok())
                .map(|s| (s.busy, s.lang))
                .unwrap_or((false, UiLang::En));

            if busy {
                let wt = to_wide(tr_text(lang, "confirm_close_title"));
                let wm = to_wide(tr_text(lang, "confirm_close_msg"));
                let choice = MessageBoxW(hwnd, wm.as_ptr(), wt.as_ptr(), MB_YESNO | MB_ICONQUESTION);
                if choice != IDYES {
                    return 0;
                }
                if let Some(lock) = GUI_STATE.get() {
                    if let Ok(st) = lock.lock() {
                        st.cancel_flag.store(true, Ordering::SeqCst);
                    }
                }
            }
            save_gui_state();
            DestroyWindow(hwnd);
            return 0;
        }
        WM_DESTROY => {
            KillTimer(hwnd, ID_TIMER_LOG);
            PostQuitMessage(0);
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

pub fn run_gui_with_config(cfg: GuiLaunchConfig) -> Result<(), String> {
    let saved_state = load_persisted_state();

    let initial_lang = cfg
        .lang
        .or_else(|| {
            saved_state.as_ref().and_then(|s| s.lang.as_deref()).and_then(|l| {
                if l.eq_ignore_ascii_case("ru") {
                    Some(UiLang::Ru)
                } else if l.eq_ignore_ascii_case("en") {
                    Some(UiLang::En)
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(detect_default_language);

    unsafe {
        let _ = SetProcessDPIAware();

        let icex = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&icex);

        let hinstance = GetModuleHandleW(std::ptr::null());
        let cls_name = to_wide("MCGM_RUST_WIN32_CLASS");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_3DFACE + 1) as _,
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls_name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);

        let initial_pal = light_palette();
        let initial_brushes = ThemeBrushes::create(&initial_pal);

        let initial_mode = GuiLayoutMode {
            show_advanced: saved_state
                .as_ref()
                .and_then(|s| s.show_advanced)
                .unwrap_or(false),
            show_log: saved_state
                .as_ref()
                .and_then(|s| s.show_log)
                .unwrap_or(false),
        };
        let (def_cw, def_ch) = compute_default_client_size(initial_mode);
        let mut adj_rc = RECT {
            left: 0,
            top: 0,
            right: def_cw,
            bottom: def_ch,
        };
        AdjustWindowRectEx(&mut adj_rc, WS_OVERLAPPEDWINDOW, 0, WS_EX_CONTROLPARENT);
        let win_w = adj_rc.right - adj_rc.left;
        let win_h = adj_rc.bottom - adj_rc.top;
        let title_w = to_wide(tr_text(initial_lang, "win_title"));
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            cls_name.as_ptr(),
            title_w.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            win_w,
            win_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err("CreateWindowExW failed".to_string());
        }

        let initial_dpi = get_window_dpi(hwnd);
        if initial_dpi != 96 {
            let mut dpi_rc = RECT {
                left: 0,
                top: 0,
                right: scale_px(def_cw, initial_dpi),
                bottom: scale_px(def_ch, initial_dpi),
            };
            AdjustWindowRectEx(&mut dpi_rc, WS_OVERLAPPEDWINDOW, 0, WS_EX_CONTROLPARENT);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                dpi_rc.right - dpi_rc.left,
                dpi_rc.bottom - dpi_rc.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        let fonts = ThemeFonts::create(initial_dpi);
        let eff_font = fonts.regular;
        let eff_title_font = fonts.title;
        let eff_bold_font = fonts.bold;
        let eff_log_font = fonts.log;

        let mut ctrls: HashMap<i32, isize> = HashMap::new();
        let hi = hinstance as isize;
        let add = |ctrls: &mut HashMap<i32, isize>,
                   cls: &str,
                   txt: &str,
                   st: u32,
                   ex: u32,
                   id: i32,
                   fnt: isize|
         -> HWND {
            let ch = create_control(hwnd, cls, txt, st, ex, 0, 0, 10, 10, id, hi, fnt);
            ctrls.insert(id, ch as isize);
            ch
        };

        let btn_vis = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | BS_PUSHBUTTON as u32;
        let btn_hid = WS_CHILD | WS_TABSTOP | WS_CLIPSIBLINGS | BS_PUSHBUTTON as u32;
        let def_btn_vis =
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | BS_DEFPUSHBUTTON as u32;
        let chk_vis = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | BS_AUTOCHECKBOX as u32;
        let chk_hid = WS_CHILD | WS_TABSTOP | WS_CLIPSIBLINGS | BS_AUTOCHECKBOX as u32;
        let grp_vis = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | BS_GROUPBOX as u32;
        let grp_hid = WS_CHILD | WS_CLIPSIBLINGS | BS_GROUPBOX as u32;
        let edit_vis = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | ES_AUTOHSCROLL as u32;
        let edit_hid = WS_CHILD | WS_TABSTOP | WS_CLIPSIBLINGS | ES_AUTOHSCROLL as u32;
        let lbl_vis = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS;
        let lbl_hid = WS_CHILD | WS_CLIPSIBLINGS;

        // Header
        add(&mut ctrls, "STATIC", tr_text(initial_lang, "ui_title"), lbl_vis, 0, ID_LBL_TITLE, eff_title_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_lang"), btn_vis, 0, ID_BTN_LANG, eff_font);

        // Step 1: Instance & World
        let g_src = add(&mut ctrls, "BUTTON", tr_text(initial_lang, "step_world"), grp_vis, 0, ID_GRP_SRC, eff_font);
        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_instance"), lbl_vis, 0, ID_LBL_INSTANCE, eff_font);
        let h_combo_inst = add(&mut ctrls, "COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | CBS_DROPDOWNLIST as u32 | WS_VSCROLL, 0, ID_COMBO_INSTANCES, eff_font);
        let inst_all_w = to_wide(&format!("{} (0)", tr_text(initial_lang, "instances_all")));
        SendMessageW(h_combo_inst, CB_ADDSTRING, 0, inst_all_w.as_ptr() as isize);
        SendMessageW(h_combo_inst, CB_SETCURSEL, 0, 0);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_add_instance"), btn_vis, 0, ID_BTN_ADD_INSTANCE, eff_font);

        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_world_row"), lbl_vis, 0, ID_LBL_WORLD_ROW, eff_font);
        let h_combo_w = add(&mut ctrls, "COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | CBS_DROPDOWNLIST as u32 | WS_VSCROLL, 0, ID_COMBO_WORLDS, eff_font);
        let wi = to_wide(tr_text(initial_lang, "worlds_none"));
        SendMessageW(h_combo_w, CB_ADDSTRING, 0, wi.as_ptr() as isize);
        SendMessageW(h_combo_w, CB_SETCURSEL, 0, 0);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_browse"), btn_vis, 0, ID_BTN_BROWSE_WORLD, eff_font);

        // Step 2: Area
        let g_meta = add(&mut ctrls, "BUTTON", tr_text(initial_lang, "step_area"), grp_vis, 0, ID_GRP_META, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "rad_small"), btn_vis, 0, ID_BTN_RAD_32, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "rad_medium"), btn_vis, 0, ID_BTN_RAD_64, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "rad_large"), btn_vis, 0, ID_BTN_RAD_128, eff_font);
        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_min"), lbl_vis, 0, ID_LBL_COORDS_MIN, eff_font);
        add(&mut ctrls, "EDIT", "-50", edit_vis, WS_EX_CLIENTEDGE, ID_EDIT_COORD_X1, eff_font);
        add(&mut ctrls, "EDIT", "-64", edit_vis, WS_EX_CLIENTEDGE, ID_EDIT_COORD_Y1, eff_font);
        add(&mut ctrls, "EDIT", "-50", edit_vis, WS_EX_CLIENTEDGE, ID_EDIT_COORD_Z1, eff_font);
        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_max"), lbl_vis, 0, ID_LBL_COORDS_MAX, eff_font);
        add(&mut ctrls, "EDIT", "50", edit_vis, WS_EX_CLIENTEDGE, ID_EDIT_COORD_X2, eff_font);
        add(&mut ctrls, "EDIT", "128", edit_vis, WS_EX_CLIENTEDGE, ID_EDIT_COORD_Y2, eff_font);
        add(&mut ctrls, "EDIT", "50", edit_vis, WS_EX_CLIENTEDGE, ID_EDIT_COORD_Z2, eff_font);
        add(&mut ctrls, "STATIC", "", lbl_vis, 0, ID_LBL_SEL_INFO, eff_font);

        // Step 3: Map settings
        let g_opts = add(&mut ctrls, "BUTTON", tr_text(initial_lang, "step_map"), grp_vis, 0, ID_GRP_OPTS, eff_font);
        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_map"), lbl_vis, 0, ID_LBL_MAP, eff_font);
        add(&mut ctrls, "EDIT", "gm_mc_custom", edit_vis, WS_EX_CLIENTEDGE, ID_EDIT_MAP, eff_font);

        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_preset"), lbl_vis, 0, ID_LBL_PRESET, eff_font);
        let h_preset = add(&mut ctrls, "COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | CBS_DROPDOWNLIST as u32, 0, ID_COMBO_PRESET, eff_font);
        for k in ["preset_preview", "preset_balanced", "preset_hq", "preset_custom"] {
            let wi = to_wide(tr_text(initial_lang, k));
            SendMessageW(h_preset, CB_ADDSTRING, 0, wi.as_ptr() as isize);
        }

        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_light"), lbl_vis, 0, ID_LBL_LIGHT, eff_font);
        let h_light = add(&mut ctrls, "COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | CBS_DROPDOWNLIST as u32, 0, ID_COMBO_LIGHT, eff_font);
        for k in ["light_minecraft", "light_gmod", "light_bright", "light_night"] {
            let wi = to_wide(tr_text(initial_lang, k));
            SendMessageW(h_light, CB_ADDSTRING, 0, wi.as_ptr() as isize);
        }
        SendMessageW(h_light, CB_SETCURSEL, 0, 0);

        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_outdir"), lbl_vis, 0, ID_LBL_OUTDIR, eff_font);
        add(&mut ctrls, "EDIT", "output", edit_vis, WS_EX_CLIENTEDGE, ID_EDIT_OUTDIR, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_browse"), btn_vis, 0, ID_BTN_BROWSE_OUT, eff_font);

        let c1 = add(&mut ctrls, "BUTTON", tr_text(initial_lang, "chk_import"), chk_vis, 0, ID_CHK_IMPORT_GMOD, eff_font);
        set_chk_state(c1 as isize, true);

        // Action & Progress
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_start"), def_btn_vis, 0, ID_BTN_START, eff_font);
        let h_stop = add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_stop"), btn_hid, 0, ID_BTN_STOP, eff_font);
        EnableWindow(h_stop, 0);
        let h_prog = add(&mut ctrls, "msctls_progress32", "", WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS, 0, ID_PROGRESS, eff_font);
        SendMessageW(h_prog, PBM_SETRANGE32, 0, 100);
        add(&mut ctrls, "STATIC", tr_text(initial_lang, "status_ready"), lbl_vis, 0, ID_LBL_STATUS, eff_font);

        // Footer toggles + quick actions
        add(
            &mut ctrls,
            "BUTTON",
            tr_text(initial_lang, if initial_mode.show_advanced { "btn_adv_hide" } else { "btn_adv_show" }),
            btn_vis,
            0,
            ID_BTN_TOGGLE_ADV,
            eff_font,
        );
        add(
            &mut ctrls,
            "BUTTON",
            tr_text(initial_lang, if initial_mode.show_log { "btn_log_hide" } else { "btn_log_show" }),
            btn_vis,
            0,
            ID_BTN_TOGGLE_LOG,
            eff_font,
        );
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_open_out"), btn_vis, 0, ID_BTN_OPEN_OUT, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_launch_gmod"), btn_vis, 0, ID_BTN_LAUNCH_GMOD, eff_font);

        // Advanced Panel (initially hidden unless restored)
        let g_adv = add(&mut ctrls, "BUTTON", tr_text(initial_lang, "grp_adv"), grp_hid, 0, ID_GRP_ADV, eff_font);
        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_world"), lbl_hid, 0, ID_LBL_WORLD, eff_font);
        add(&mut ctrls, "EDIT", "", edit_hid, WS_EX_CLIENTEDGE, ID_EDIT_WORLD, eff_font);

        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_dim"), lbl_hid, 0, ID_LBL_DIM, eff_font);
        let h_dim = add(&mut ctrls, "COMBOBOX", "", WS_CHILD | WS_TABSTOP | WS_CLIPSIBLINGS | CBS_DROPDOWNLIST as u32, 0, ID_COMBO_DIM, eff_font);
        for k in ["dim_overworld", "dim_nether", "dim_end"] {
            let wi = to_wide(tr_text(initial_lang, k));
            SendMessageW(h_dim, CB_ADDSTRING, 0, wi.as_ptr() as isize);
        }
        SendMessageW(h_dim, CB_SETCURSEL, 0, 0);

        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_scale"), lbl_hid, 0, ID_LBL_SCALE, eff_font);
        add(&mut ctrls, "EDIT", "39.37", edit_hid, WS_EX_CLIENTEDGE, ID_EDIT_SCALE, eff_font);

        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_name"), lbl_hid, 0, ID_LBL_NAME, eff_font);
        add(&mut ctrls, "EDIT", "mc_custom", edit_hid, WS_EX_CLIENTEDGE, ID_EDIT_NAME, eff_font);

        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_cat"), lbl_hid, 0, ID_LBL_CAT, eff_font);
        add(&mut ctrls, "EDIT", "custom_mc", edit_hid, WS_EX_CLIENTEDGE, ID_EDIT_CAT, eff_font);

        let c5 = add(&mut ctrls, "BUTTON", tr_text(initial_lang, "chk_phys"), chk_hid, 0, ID_CHK_MC_PHYSICS, eff_font);
        set_chk_state(c5 as isize, true);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "chk_vmf"), chk_hid, 0, ID_CHK_EXPORT_VMF, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "chk_hq"), chk_hid, 0, ID_CHK_HQ_LIGHT, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "chk_fast"), chk_hid, 0, ID_CHK_FAST_VRAD, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "chk_brushes"), chk_hid, 0, ID_CHK_VMF_BRUSHES, eff_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_copy_map"), btn_hid, 0, ID_BTN_COPY_MAP, eff_font);

        // Place groupbox frames at the bottom of Z-order so inner controls paint and receive clicks cleanly
        for g in [g_src, g_meta, g_opts, g_adv] {
            SetWindowPos(
                g,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }

        // Log Panel (initially hidden unless restored)
        add(&mut ctrls, "STATIC", tr_text(initial_lang, "lbl_log_hdr"), lbl_hid, 0, ID_LBL_LOG_HDR, eff_bold_font);
        add(&mut ctrls, "BUTTON", tr_text(initial_lang, "btn_clear"), btn_hid, 0, ID_BTN_CLEAR, eff_font);
        let h_log = add(
            &mut ctrls,
            "EDIT",
            "",
            WS_CHILD
                | WS_TABSTOP
                | WS_CLIPSIBLINGS
                | WS_VSCROLL
                | WS_HSCROLL
                | ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32
                | ES_AUTOHSCROLL as u32
                | ES_READONLY as u32,
            WS_EX_CLIENTEDGE,
            ID_LOG,
            eff_log_font,
        );
        SendMessageW(h_log, EM_SETLIMITTEXT, 0x7FFF_FFFFusize, 0);

        let initial_custom_dirs: Vec<PathBuf> = saved_state
            .as_ref()
            .map(|s| {
                s.custom_instance_dirs
                    .iter()
                    .map(|p| p.trim())
                    .filter(|p| !p.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        let initial_pending_saves = saved_state
            .as_ref()
            .and_then(|s| s.selected_instance_saves.clone())
            .filter(|s| !s.trim().is_empty());

        let _ = GUI_HANDLES.set(GuiHandles {
            hwnd: hwnd as isize,
            controls: ctrls,
        });
        let _ = GUI_STATE.set(Mutex::new(GuiState {
            lang: initial_lang,
            dpi: initial_dpi,
            brushes: initial_brushes,
            fonts,
            busy: false,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            discovered_instances: Vec::new(),
            selected_instance_idx: None,
            custom_instance_dirs: initial_custom_dirs.clone(),
            pending_saved_instance_saves: initial_pending_saves,
            discovered_worlds: Vec::new(),
            anchor_center: None,
            build_preset: "balanced".to_string(),
            status_key: "status_ready",
            custom_status_text: None,
            config_path: cfg.config_path.clone(),
            syncing_preset: false,
            suppress_world_combo: false,
            suppress_instance_combo: false,
            layout_mode: initial_mode,
            last_auto_map_name: None,
            model_name_manually_edited: false,
            suppress_name_sync: false,
            cli_chunk_size: cfg.chunk_size,
            cli_max_workers: cfg.max_workers,
            cli_max_convex: cfg.max_convex,
            cli_reuse_cache: cfg.reuse_cache,
            cli_debug_log: cfg.debug_log.clone(),
            custom_dimension: None,
        }));

        apply_edit_margins(initial_dpi);
        apply_window_theme(hwnd);
        apply_panel_visibility(initial_mode, false);

        // Initial layout from actual client rect
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        layout_controls(rc.right - rc.left, rc.bottom - rc.top);

        // 1. Restore persisted gui_state.json
        let mut has_world_or_coords = false;
        if let Some(ref ps) = saved_state {
            if let Some(ref wd) = ps.world_dir {
                if !wd.is_empty() {
                    if let Some(hw) = get_ctrl_hwnd(ID_EDIT_WORLD) {
                        set_ctrl_text(hw, wd);
                    }
                }
            }
            if let (Some(x1), Some(y1), Some(z1), Some(x2), Some(y2), Some(z2)) =
                (ps.min_x, ps.min_y, ps.min_z, ps.max_x, ps.max_y, ps.max_z)
            {
                set_coord_boxes(x1, y1, z1, x2, y2, z2);
                has_world_or_coords = true;
            }
            if let Some(ref dim) = ps.dimension {
                set_dim_combo_value(dim, initial_lang);
            }
            if let Some(lock) = GUI_STATE.get() {
                if let Ok(mut st) = lock.lock() {
                    st.suppress_name_sync = true;
                }
            }
            if let Some(ref n) = ps.name {
                if !n.is_empty() {
                    if let Some(hn) = get_ctrl_hwnd(ID_EDIT_NAME) {
                        set_ctrl_text(hn, n);
                    }
                }
            }
            if let Some(ref c) = ps.category {
                if !c.is_empty() {
                    if let Some(hc) = get_ctrl_hwnd(ID_EDIT_CAT) {
                        set_ctrl_text(hc, c);
                    }
                }
            }
            if let Some(ref m) = ps.map_name {
                if !m.is_empty() {
                    if let Some(hm) = get_ctrl_hwnd(ID_EDIT_MAP) {
                        set_ctrl_text(hm, m);
                    }
                }
            }
            if let Some(lock) = GUI_STATE.get() {
                if let Ok(mut st) = lock.lock() {
                    st.suppress_name_sync = false;
                    if let (Some(n), Some(m)) = (ps.name.as_deref(), ps.map_name.as_deref()) {
                        if !n.is_empty() && !m.is_empty() && n != derive_model_name_from_map(m) {
                            st.model_name_manually_edited = true;
                        }
                    }
                }
            }
            if let Some(ref o) = ps.output_dir {
                if !o.is_empty() {
                    if let Some(ho) = get_ctrl_hwnd(ID_EDIT_OUTDIR) {
                        set_ctrl_text(ho, o);
                    }
                }
            }
            if let Some(ref l) = ps.lighting_style {
                if let Some(hl) = get_ctrl_hwnd(ID_COMBO_LIGHT) {
                    SendMessageW(hl as HWND, CB_SETCURSEL, light_index_from_str(l), 0);
                }
            }
            if let Some(ref sc) = ps.scale {
                if !sc.is_empty() {
                    if let Some(hs) = get_ctrl_hwnd(ID_EDIT_SCALE) {
                        set_ctrl_text(hs, sc);
                    }
                }
            }
            if let Some(v) = ps.import_to_gmod {
                if let Some(h) = get_ctrl_hwnd(ID_CHK_IMPORT_GMOD) {
                    set_chk_state(h, v);
                }
            }
            if let Some(v) = ps.export_vmf {
                if let Some(h) = get_ctrl_hwnd(ID_CHK_EXPORT_VMF) {
                    set_chk_state(h, v);
                }
            }
            if let Some(v) = ps.hq_light {
                if let Some(h) = get_ctrl_hwnd(ID_CHK_HQ_LIGHT) {
                    set_chk_state(h, v);
                }
            }
            if let Some(v) = ps.fast_vrad {
                if let Some(h) = get_ctrl_hwnd(ID_CHK_FAST_VRAD) {
                    set_chk_state(h, v);
                }
            }
            if let Some(v) = ps.mc_physics {
                if let Some(h) = get_ctrl_hwnd(ID_CHK_MC_PHYSICS) {
                    set_chk_state(h, v);
                }
            }
            if let Some(v) = ps.force_vmf_brushes {
                if let Some(h) = get_ctrl_hwnd(ID_CHK_VMF_BRUSHES) {
                    set_chk_state(h, v);
                }
            }
            if let Some(ref p) = ps.build_preset {
                if p != "custom" {
                    apply_build_preset_ui(p);
                }
            }
        }

        // 3. Apply GuiLaunchConfig overrides (e.g. from Minecraft mod CLI)
        if let Some(ref w) = cfg.world {
            if let Some(hw) = get_ctrl_hwnd(ID_EDIT_WORLD) {
                set_ctrl_text(hw, &w.to_string_lossy());
            }
            sync_worlds_combo(initial_lang, Some(w));
            if cfg.coords.is_none() && cfg.startup_error.is_none() {
                populate_from_world_path(w, true);
                has_world_or_coords = true;
            }
        } else if let Some(ref ps) = saved_state {
            if let Some(ref wd) = ps.world_dir {
                if !wd.is_empty() {
                    sync_worlds_combo(initial_lang, Some(Path::new(wd)));
                }
            }
        }
        if let Some(c) = cfg.coords {
            let norm = c.normalized();
            set_coord_boxes(norm.min_x, norm.min_y, norm.min_z, norm.max_x, norm.max_y, norm.max_z);
            if let Some(lock) = GUI_STATE.get() {
                if let Ok(mut st) = lock.lock() {
                    st.anchor_center = Some([
                        (norm.min_x + norm.max_x) / 2,
                        (norm.min_y + norm.max_y) / 2,
                        (norm.min_z + norm.max_z) / 2,
                    ]);
                }
            }
            has_world_or_coords = true;
        }
        if let Some(ref dim) = cfg.dimension {
            set_dim_combo_value(dim, initial_lang);
        }
        if let Some(ref n) = cfg.name {
            if let Some(lock) = GUI_STATE.get() {
                if let Ok(mut st) = lock.lock() {
                    st.suppress_name_sync = true;
                    st.model_name_manually_edited = true;
                }
            }
            if let Some(hn) = get_ctrl_hwnd(ID_EDIT_NAME) {
                set_ctrl_text(hn, n);
            }
            if cfg.map_name.is_none() {
                if let Some(hm) = get_ctrl_hwnd(ID_EDIT_MAP) {
                    let derived = if n.starts_with("gm_") {
                        n.clone()
                    } else {
                        format!("gm_{}", n)
                    };
                    set_ctrl_text(hm, &derived);
                }
            }
            if let Some(lock) = GUI_STATE.get() {
                if let Ok(mut st) = lock.lock() {
                    st.suppress_name_sync = false;
                }
            }
        }
        if let Some(ref cat) = cfg.category {
            if let Some(hc) = get_ctrl_hwnd(ID_EDIT_CAT) {
                set_ctrl_text(hc, cat);
            }
        }
        if let Some(ref m) = cfg.map_name {
            if let Some(hm) = get_ctrl_hwnd(ID_EDIT_MAP) {
                set_ctrl_text(hm, m);
            }
        }
        if let Some(ref o) = cfg.output_dir {
            if let Some(ho) = get_ctrl_hwnd(ID_EDIT_OUTDIR) {
                set_ctrl_text(ho, &o.to_string_lossy());
            }
        }
        if let Some(ref l) = cfg.lighting_style {
            if let Some(hl) = get_ctrl_hwnd(ID_COMBO_LIGHT) {
                SendMessageW(hl as HWND, CB_SETCURSEL, light_index_from_str(l), 0);
            }
        }
        if let Some(sc) = cfg.scale {
            if sc.is_finite() && sc > 0.0 {
                if let Some(hs) = get_ctrl_hwnd(ID_EDIT_SCALE) {
                    set_ctrl_text(hs, &format!("{}", sc));
                }
            }
        }
        if let Some(v) = cfg.import_to_gmod {
            if let Some(h) = get_ctrl_hwnd(ID_CHK_IMPORT_GMOD) {
                set_chk_state(h, v);
            }
        }
        if let Some(v) = cfg.export_vmf {
            if let Some(h) = get_ctrl_hwnd(ID_CHK_EXPORT_VMF) {
                set_chk_state(h, v);
            }
        }
        if let Some(v) = cfg.hq_light {
            if let Some(h) = get_ctrl_hwnd(ID_CHK_HQ_LIGHT) {
                set_chk_state(h, v);
            }
        }
        if let Some(v) = cfg.fast_vrad {
            if let Some(h) = get_ctrl_hwnd(ID_CHK_FAST_VRAD) {
                set_chk_state(h, v);
            }
        }
        if let Some(v) = cfg.mc_physics {
            if let Some(h) = get_ctrl_hwnd(ID_CHK_MC_PHYSICS) {
                set_chk_state(h, v);
            }
        }
        if let Some(v) = cfg.force_vmf_brushes {
            if let Some(h) = get_ctrl_hwnd(ID_CHK_VMF_BRUSHES) {
                set_chk_state(h, v);
            }
        }

        let _ = has_world_or_coords;
        sync_preset_from_checkboxes_ui();
        update_selection_info_label(initial_lang);
        if let Some(ref err) = cfg.startup_error {
            if let Some(lock) = GUI_STATE.get() {
                if let Ok(mut st) = lock.lock() {
                    st.status_key = "status_failed";
                    st.custom_status_text = Some(err.clone());
                }
            }
            if let Some(hl) = get_ctrl_hwnd(ID_LBL_STATUS) {
                set_ctrl_text(hl, err);
            }
            if let Some(hlog) = get_ctrl_hwnd(ID_LOG) {
                append_log_line(hlog, &format!("ERROR: {}", err));
            }
            if !initial_mode.show_log {
                set_panel_mode(
                    hwnd,
                    GuiLayoutMode {
                        show_advanced: initial_mode.show_advanced,
                        show_log: true,
                    },
                );
            }
        }
        ShowWindow(hwnd, SW_SHOW);

        if let Some(ref err) = cfg.startup_error {
            let wt = to_wide(tr_text(initial_lang, "err_title"));
            let wm = to_wide(err);
            MessageBoxW(hwnd, wm.as_ptr(), wt.as_ptr(), MB_OK | MB_ICONERROR);
        } else if cfg.auto_start {
            PostMessageW(hwnd, WM_APP_AUTOSTART, 0, 0);
        }

        {
            let bg_hwnd = hwnd as isize;
            let bg_cfg = cfg.config_path.clone();
            let bg_extra = initial_custom_dirs;
            std::thread::spawn(move || {
                let found = crate::instances::discover_all_instances(bg_cfg.as_deref(), &bg_extra);
                let ptr = Box::into_raw(Box::new(found));
                if PostMessageW(bg_hwnd as HWND, WM_APP_WORLDS_READY, 0, ptr as isize) == 0 {
                    drop(Box::from_raw(ptr));
                }
            });
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if let Some(lock) = GUI_STATE.get() {
            if let Ok(mut st) = lock.lock() {
                st.fonts.destroy();
                st.brushes.destroy();
            }
        }
    }
    Ok(())
}
