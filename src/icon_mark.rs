//! App-tile raster shared by the tray / Linux hicolor path and the Windows
//! PE `build.rs` icon. Keep this file free of GPUI so the build script can
//! `include` it.

use image::{imageops, Rgba, RgbaImage};

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
