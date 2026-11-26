use image::{ImageBuffer, Rgba, RgbaImage};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct SkyPalette {
    pub zenith: [u8; 3],
    pub horizon: [u8; 3],
    pub fog: [u8; 3],
    pub is_night: bool,
}

// Authentic Minecraft Color Palettes
const DAY_ZENITH: [u8; 3] = [120, 167, 255]; // #78A7FF
const DAY_HORIZON: [u8; 3] = [192, 216, 255]; // #C0D8FF
const DAY_FOG: [u8; 3] = [176, 200, 240]; // #B0C8F0

const NIGHT_ZENITH: [u8; 3] = [5, 7, 18]; // #050712
const NIGHT_HORIZON: [u8; 3] = [13, 21, 44]; // #0D152C
const NIGHT_FOG: [u8; 3] = [13, 21, 44]; // #0D152C

// Cloud colors
const CLOUD_WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
const CLOUD_SHADOW: Rgba<u8> = Rgba([200, 214, 232, 255]);

// Sun colors (Minecraft authentic square pixel sun)
const SUN_CORE: Rgba<u8> = Rgba([255, 255, 255, 255]);
const SUN_INNER: Rgba<u8> = Rgba([255, 255, 170, 255]); // #FFFFAA
const SUN_OUTER: Rgba<u8> = Rgba([255, 213, 74, 255]); // #FFD54A

// Moon colors (Minecraft authentic square pixel moon)
const MOON_BODY: Rgba<u8> = Rgba([240, 244, 252, 255]);
const MOON_CORE: Rgba<u8> = Rgba([255, 255, 255, 255]);
const MOON_RIM: Rgba<u8> = Rgba([150, 165, 185, 255]);
const MOON_CRATER_LIGHT: Rgba<u8> = Rgba([170, 182, 200, 255]);
const MOON_CRATER_DARK: Rgba<u8> = Rgba([135, 148, 168, 255]);

pub fn palette_for_style(style: &str) -> SkyPalette {
    match style.trim().to_lowercase().as_str() {
        "night" => SkyPalette {
            zenith: NIGHT_ZENITH,
            horizon: NIGHT_HORIZON,
            fog: NIGHT_FOG,
            is_night: true,
        },
        "bright" => SkyPalette {
            zenith: DAY_ZENITH,
            horizon: DAY_HORIZON,
            fog: DAY_FOG,
            is_night: false,
        },
        "gmod" => SkyPalette {
            zenith: DAY_ZENITH,
            horizon: DAY_HORIZON,
            fog: DAY_FOG,
            is_night: false,
        },
        _ => SkyPalette {
            zenith: DAY_ZENITH,
            horizon: DAY_HORIZON,
            fog: DAY_FOG,
            is_night: false,
        },
    }
}

fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> Rgba<u8> {
    let tc = t.clamp(0.0, 1.0);
    Rgba([
        (a[0] as f32 * (1.0 - tc) + b[0] as f32 * tc).round() as u8,
        (a[1] as f32 * (1.0 - tc) + b[1] as f32 * tc).round() as u8,
        (a[2] as f32 * (1.0 - tc) + b[2] as f32 * tc).round() as u8,
        255,
    ])
}

pub fn generate_minecraft_skybox(style: &str, out_dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    generate_minecraft_skybox_with_options(style, out_dir, 512, "mc_sky_")
}

pub fn generate_minecraft_skybox_with_options(
    style: &str,
    out_dir: &Path,
    size: u32,
    prefix: &str,
) -> Result<Vec<(String, PathBuf)>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("Failed creating skybox dir: {}", e))?;
    let size = size.clamp(64, 4096);
    let pal = palette_for_style(style);
    let clean_style = match style.trim().to_lowercase().as_str() {
        "night" => "night",
        "gmod" => "gmod",
        "bright" => "bright",
        _ => "minecraft",
    };
    let sky_prefix = format!("{}{}", prefix, clean_style);

    // 1. UP Face (Zenith) — render Sun or Moon + Zenith Stars directly overhead
    let mut up_img: RgbaImage = ImageBuffer::from_pixel(
        size,
        size,
        Rgba([pal.zenith[0], pal.zenith[1], pal.zenith[2], 255]),
    );
    if pal.is_night {
        let avoid_r = ((90.0 * size as f32) / 512.0).round() as i32;
        let pad_y = ((10.0 * size as f32) / 512.0).round().max(2.0) as u32;
        draw_pixel_stars(
            &mut up_img,
            60,
            1001,
            pad_y,
            size.saturating_sub(pad_y).max(pad_y + 1),
            avoid_r,
        );
        draw_pixel_moon(&mut up_img, size);
    } else {
        draw_pixel_sun(&mut up_img, size, clean_style);
    }

    // 2. DN Face (Nadir / Fog)
    let dn_img: RgbaImage =
        ImageBuffer::from_pixel(size, size, Rgba([pal.fog[0], pal.fog[1], pal.fog[2], 255]));

    // 3. Side Faces: ft, rt, bk, lf (360-degree panoramic horizon)
    let pano_w = size.checked_mul(4).unwrap_or(4096 * 4);
    let pano_h = size;
    let mut pano: RgbaImage = ImageBuffer::new(pano_w, pano_h);
    let denom = (pano_h.saturating_sub(1)).max(1) as f32;
    for y in 0..pano_h {
        let t = y as f32 / denom;
        let col = lerp_rgb(pal.zenith, pal.horizon, t);
        for x in 0..pano_w {
            pano.put_pixel(x, y, col);
        }
    }

    if pal.is_night {
        let min_y = ((15.0 * size as f32) / 512.0).round().max(1.0) as u32;
        let max_y = ((380.0 * size as f32) / 512.0).round().min((size - 2) as f32) as u32;
        for face_idx in 0..4u32 {
            let sub_seed = 2000u64 + (face_idx as u64) * 17;
            let x_off = face_idx * size;
            let mut slice = image::imageops::crop_imm(&pano, x_off, 0, size, size).to_image();
            draw_pixel_stars(&mut slice, 35, sub_seed, min_y, max_y, 0);
            image::imageops::replace(&mut pano, &slice, x_off as i64, 0);
        }
    } else {
        draw_horizon_clouds(&mut pano, size);
    }

    let faces: [(&str, RgbaImage); 6] = [
        ("up", up_img),
        ("dn", dn_img),
        ("lf", image::imageops::crop_imm(&pano, size * 3, 0, size, size).to_image()),
        ("rt", image::imageops::crop_imm(&pano, size, 0, size, size).to_image()),
        ("ft", image::imageops::crop_imm(&pano, 0, 0, size, size).to_image()),
        ("bk", image::imageops::crop_imm(&pano, size * 2, 0, size, size).to_image()),
    ];

    let mut written = Vec::with_capacity(6);
    for (suffix, img) in faces {
        let p = out_dir.join(format!("{}{}.png", sky_prefix, suffix));
        crate::atlas::save_png_fast(&img, &p)
            .map_err(|e| format!("Failed saving skybox face {}: {}", p.display(), e))?;
        written.push((format!("{}{}", sky_prefix, suffix), p));
    }

    Ok(written)
}

fn fill_rect(img: &mut RgbaImage, x1: i32, y1: i32, x2: i32, y2: i32, col: Rgba<u8>) {
    let w = img.width() as i32;
    let h = img.height() as i32;
    let sx = x1.max(0);
    let sy = y1.max(0);
    let ex = x2.min(w - 1);
    let ey = y2.min(h - 1);
    if sx > ex || sy > ey {
        return;
    }
    for y in sy..=ey {
        for x in sx..=ex {
            img.put_pixel(x as u32, y as u32, col);
        }
    }
}

fn draw_pixel_sun(img: &mut RgbaImage, size: u32, style: &str) {
    let cx = (size / 2) as i32;
    let cy = (size / 2) as i32;
    let scale = size as f32 / 512.0;

    let (outer_base, inner_base, core_base) = if style == "bright" {
        (64.0f32, 48.0f32, 36.0f32)
    } else {
        (56.0f32, 40.0f32, 28.0f32)
    };
    let outer_half = (outer_base * scale).round().max(3.0) as i32;
    let inner_half = (inner_base * scale).round().max(2.0) as i32;
    let core_half = (core_base * scale).round().max(1.0) as i32;

    fill_rect(img, cx - outer_half, cy - outer_half, cx + outer_half - 1, cy + outer_half - 1, SUN_OUTER);
    fill_rect(img, cx - inner_half, cy - inner_half, cx + inner_half - 1, cy + inner_half - 1, SUN_INNER);
    fill_rect(img, cx - core_half, cy - core_half, cx + core_half - 1, cy + core_half - 1, SUN_CORE);
}

fn draw_pixel_moon(img: &mut RgbaImage, size: u32) {
    let cx = (size / 2) as i32;
    let cy = (size / 2) as i32;
    let scale = size as f32 / 512.0;
    let half = (48.0 * scale).round().max(6.0) as i32;
    let rim = (4.0 * scale).round().max(1.0) as i32;
    let block = (8.0 * scale).round().max(1.0) as i32;

    let x1 = cx - half;
    let y1 = cy - half;
    let x2 = cx + half - 1;
    let y2 = cy + half - 1;

    fill_rect(img, x1, y1, x2, y2, MOON_RIM);
    fill_rect(img, x1 + rim, y1 + rim, x2 - rim, y2 - rim, MOON_BODY);

    let craters: [(i32, i32, i32, i32, bool); 8] = [
        (2, 2, 3, 2, true),
        (2, 4, 2, 2, false),
        (6, 3, 2, 3, true),
        (7, 2, 2, 2, false),
        (3, 7, 3, 2, false),
        (4, 8, 2, 2, true),
        (7, 7, 3, 3, true),
        (8, 6, 2, 2, false),
    ];

    for (gx, gy, gw, gh, is_dark) in craters {
        let col = if is_dark { MOON_CRATER_DARK } else { MOON_CRATER_LIGHT };
        let px = x1 + rim + gx * block;
        let py = y1 + rim + gy * block;
        fill_rect(img, px, py, px + gw * block - 1, py + gh * block - 1, col);
    }

    let hl_lo = (8.0 * scale).round().max(1.0) as i32;
    let hl_hi = ((24.0 * scale).round() as i32).max(hl_lo + 1);
    fill_rect(img, x1 + hl_lo, y1 + hl_lo, x1 + hl_hi, y1 + hl_hi, MOON_CORE);
}

fn draw_pixel_stars(
    img: &mut RgbaImage,
    count: usize,
    seed: u64,
    min_y: u32,
    max_y: u32,
    avoid_center_radius: i32,
) {
    let size = img.width() as i32;
    let cx = size / 2;
    let cy = size / 2;

    const STAR_COLORS: [Rgba<u8>; 4] = [
        Rgba([255, 255, 255, 255]), // Pure white
        Rgba([220, 235, 255, 255]), // Cool diamond white
        Rgba([255, 250, 215, 255]), // Warm star
        Rgba([190, 210, 240, 255]), // Dim bluish star
    ];

    let mut rng = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut next_u64 = || -> u64 {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        rng
    };

    let x_lo = 4i32;
    let x_hi = (size - 5).max(x_lo);
    let x_span = (x_hi - x_lo + 1) as u64;
    let y_lo = (min_y as i32).min(size - 2);
    let y_hi = (max_y as i32).max(y_lo).min(size - 2);
    let y_span = (y_hi - y_lo + 1) as u64;

    let mut placed = 0usize;
    let mut attempts = 0usize;
    while placed < count && attempts < count * 10 {
        attempts += 1;
        let x = x_lo + ((next_u64() >> 16) % x_span) as i32;
        let y = y_lo + ((next_u64() >> 16) % y_span) as i32;

        if avoid_center_radius > 0 {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy < avoid_center_radius * avoid_center_radius {
                continue;
            }
        }

        let col = STAR_COLORS[((next_u64() >> 24) % 4) as usize];
        let star_roll = ((next_u64() >> 20) % 100) as u32;
        if star_roll < 65 {
            img.put_pixel(x as u32, y as u32, col);
        } else if star_roll < 90 {
            fill_rect(img, x, y, x + 1, y + 1, col);
        } else {
            img.put_pixel(x as u32, y as u32, col);
            img.put_pixel((x - 1).max(0) as u32, y as u32, col);
            img.put_pixel((x + 1).min(size - 1) as u32, y as u32, col);
            img.put_pixel(x as u32, (y - 1).max(0) as u32, col);
            img.put_pixel(x as u32, (y + 1).min(size - 1) as u32, col);
        }

        placed += 1;
    }
}

fn draw_horizon_clouds(pano: &mut RgbaImage, face_size: u32) {
    let width = pano.width() as i32;
    let scale = face_size as f32 / 512.0;

    let draw_wrapped_rect = |img: &mut RgbaImage, rx: i32, ry: i32, rw: i32, rh: i32, fill: Rgba<u8>| {
        if rw <= 0 || rh <= 0 {
            return;
        }
        let rx = rx.rem_euclid(width);
        if rx + rw <= width {
            fill_rect(img, rx, ry, rx + rw - 1, ry + rh - 1, fill);
        } else {
            let first_w = width - rx;
            let second_w = rw - first_w;
            fill_rect(img, rx, ry, width - 1, ry + rh - 1, fill);
            fill_rect(img, 0, ry, second_w - 1, ry + rh - 1, fill);
        }
    };

    // Authentic Minecraft horizon cloud bands along the 2048x512 panorama
    const CLOUD_DEFS: [(i32, i32, i32, i32); 15] = [
        (40, 340, 192, 28),
        (120, 332, 128, 16),
        (320, 360, 224, 32),
        (400, 352, 96, 16),
        (620, 344, 256, 32),
        (720, 336, 160, 16),
        (960, 368, 176, 28),
        (1040, 356, 96, 20),
        (1200, 340, 240, 32),
        (1300, 332, 112, 16),
        (1500, 364, 208, 30),
        (1580, 352, 128, 20),
        (1780, 348, 224, 32),
        (1880, 336, 144, 16),
        (1980, 360, 180, 28), // Wraps around 2048 -> 0 seamlessly
    ];

    let snap = (8.0 * scale).round().max(1.0) as i32;
    let shadow_h = snap;

    for (raw_x, raw_y, raw_w, raw_h) in CLOUD_DEFS {
        let cx = (((raw_x as f32 * scale).round() as i32) / snap) * snap;
        let cy = (((raw_y as f32 * scale).round() as i32) / snap) * snap;
        let cw = ((((raw_w as f32 * scale).round() as i32) / snap) * snap).max(snap);
        let ch = ((((raw_h as f32 * scale).round() as i32) / snap) * snap).max(snap * 2);

        let body_h = (ch - shadow_h).max(snap);
        draw_wrapped_rect(pano, cx, cy, cw, body_h, CLOUD_WHITE);
        draw_wrapped_rect(pano, cx, cy + body_h, cw, shadow_h, CLOUD_SHADOW);
    }
}
