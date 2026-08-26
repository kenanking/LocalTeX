use std::io::Cursor;

use image::{imageops, ImageFormat, Rgba, RgbaImage};

use crate::identity::{APP_ID, APP_NAME, APP_SLUG};

/// App tile (rounded square + mark). Same bytes as `assets/icon.svg`.
pub const APP_ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");

const SRC: u32 = 256;
const WHITE: [u8; 4] = [255, 255, 255, 255];
const TILE: [u8; 4] = [0x11, 0x11, 0x11, 255];

const TOP_BEAM: &[(f32, f32)] = &[
    (66.0, 48.0),
    (98.0, 48.0),
    (98.0, 58.0),
    (166.0, 58.0),
    (166.0, 48.0),
    (198.0, 48.0),
    (198.0, 82.0),
    (178.0, 82.0),
    (178.0, 70.0),
    (86.0, 70.0),
    (86.0, 82.0),
    (66.0, 82.0),
];

const BOTTOM_BEAM: &[(f32, f32)] = &[
    (66.0, 174.0),
    (86.0, 174.0),
    (86.0, 186.0),
    (178.0, 186.0),
    (178.0, 174.0),
    (198.0, 174.0),
    (198.0, 208.0),
    (166.0, 208.0),
    (166.0, 198.0),
    (98.0, 198.0),
    (98.0, 208.0),
    (66.0, 208.0),
];

const UPPER_CHEVRON: &[(f32, f32)] = &[(88.0, 73.0), (120.0, 73.0), (160.0, 126.0), (128.0, 126.0)];
const LOWER_CHEVRON: &[(f32, f32)] = &[
    (88.0, 183.0),
    (120.0, 183.0),
    (160.0, 130.0),
    (128.0, 130.0),
];

/// Rasterize the app tile to `size × size` RGBA (premultiplied not required).
pub fn raster(size: u32) -> RgbaImage {
    let size = size.max(1);
    let mut src = RgbaImage::new(SRC, SRC);
    fill_round_rect(&mut src, SRC, 56, TILE);
    fill_poly(&mut src, TOP_BEAM, WHITE);
    fill_poly(&mut src, BOTTOM_BEAM, WHITE);
    fill_poly(&mut src, UPPER_CHEVRON, WHITE);
    fill_poly(&mut src, LOWER_CHEVRON, WHITE);
    if size == SRC {
        src
    } else {
        imageops::resize(&src, size, size, imageops::FilterType::Lanczos3)
    }
}

pub fn rgba_bytes(size: u32) -> (u32, u32, Vec<u8>) {
    let img = raster(size);
    (img.width(), img.height(), img.into_raw())
}

/// ARGB32 in network byte order (StatusNotifierItem / ksni).
pub fn argb_bytes(size: u32) -> (i32, i32, Vec<u8>) {
    let (w, h, mut data) = rgba_bytes(size);
    for px in data.chunks_exact_mut(4) {
        px.rotate_right(1);
    }
    (w as i32, h as i32, data)
}

pub fn png_bytes(size: u32) -> Result<Vec<u8>, image::ImageError> {
    let img = raster(size);
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
    Ok(out)
}

/// Drop a `.desktop` + hicolor icons so GNOME can match `WM_CLASS` = `APP_ID`.
/// GPUI 0.2 has no window-icon API; the desktop file is the Linux identity.
#[cfg(target_os = "linux")]
pub fn install_desktop_identity() {
    if let Err(err) = install_desktop_identity_inner() {
        eprintln!("{APP_SLUG}: desktop identity: {err}");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn install_desktop_identity() {}

#[cfg(target_os = "linux")]
fn install_desktop_identity_inner() -> Result<(), String> {
    let data = dirs::data_dir().ok_or_else(|| "no XDG data dir".to_string())?;
    let icon_dir = data.join("icons/hicolor/scalable/apps");
    let png_dir = data.join("icons/hicolor/128x128/apps");
    let app_dir = data.join("applications");
    std::fs::create_dir_all(&icon_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&png_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;

    std::fs::write(icon_dir.join(format!("{APP_ID}.svg")), APP_ICON_SVG)
        .map_err(|e| e.to_string())?;
    let png = png_bytes(128).map_err(|e| e.to_string())?;
    std::fs::write(png_dir.join(format!("{APP_ID}.png")), png).map_err(|e| e.to_string())?;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exec = exe.display().to_string().replace('"', "\\\"");
    let desktop = format!(
        "\
[Desktop Entry]
Type=Application
Version=1.0
Name={APP_NAME}
Comment=Screenshot to TeX and Markdown
Exec=\"{exec}\"
Icon={APP_ID}
Terminal=false
Categories=Utility;Graphics;Office;
StartupWMClass={APP_ID}
StartupNotify=true
"
    );
    std::fs::write(app_dir.join(format!("{APP_ID}.desktop")), desktop)
        .map_err(|e| e.to_string())?;

    let _ = std::process::Command::new("update-desktop-database")
        .arg(&app_dir)
        .status();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(data.join("icons/hicolor"))
        .status();
    Ok(())
}

fn fill_round_rect(img: &mut RgbaImage, size: u32, radius: u32, color: [u8; 4]) {
    let s = size as f32;
    let r = radius as f32;
    let color = Rgba(color);
    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let cx = px.clamp(r, s - r);
            let cy = py.clamp(r, s - r);
            let dx = px - cx;
            let dy = py - cy;
            if dx * dx + dy * dy <= r * r {
                img.put_pixel(x, y, color);
            }
        }
    }
}

fn fill_poly(img: &mut RgbaImage, pts: &[(f32, f32)], color: [u8; 4]) {
    let color = Rgba(color);
    let (w, h) = img.dimensions();
    for y in 0..h {
        for x in 0..w {
            if point_in_poly(x as f32 + 0.5, y as f32 + 0.5, pts) {
                img.put_pixel(x, y, color);
            }
        }
    }
}

fn point_in_poly(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        let intersect = ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi);
        if intersect {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_is_square_and_tiled() {
        let img = raster(64);
        assert_eq!(img.dimensions(), (64, 64));
        // Right of the chevron tip; avoid the Lanczos-softened seam at the center.
        let tile = img.get_pixel(54, 32).0;
        assert!(
            tile[0] < 40 && tile[1] < 40 && tile[2] < 40,
            "tile should be dark: {tile:?}"
        );
        let mark = img.get_pixel(32, 16).0;
        assert!(mark[0] > 200, "top bar should be white: {mark:?}");
    }

    #[test]
    fn svg_is_embedded() {
        let text = std::str::from_utf8(APP_ICON_SVG).expect("utf8");
        assert!(text.contains("viewBox=\"0 0 256 256\""));
    }
}
