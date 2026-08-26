use std::sync::Arc;

use gpui::RenderImage;
use image::{imageops, Rgba, RgbaImage};

pub fn rgba_to_render(img: &RgbaImage) -> Arc<RenderImage> {
    let mut bgra = img.clone();
    for pixel in bgra.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let frame = image::Frame::new(bgra);
    Arc::new(RenderImage::new(smallvec::smallvec![frame]))
}

pub fn thumbnail(img: &RgbaImage, max_w: u32, max_h: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img.clone();
    }
    let scale = (max_w as f32 / w as f32)
        .min(max_h as f32 / h as f32)
        .min(1.0);
    let tw = (w as f32 * scale).round().max(1.0) as u32;
    let th = (h as f32 * scale).round().max(1.0) as u32;
    imageops::thumbnail(img, tw, th)
}

pub fn crop(img: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
    let x = x.min(img.width().saturating_sub(1));
    let y = y.min(img.height().saturating_sub(1));
    let w = w.min(img.width() - x).max(1);
    let h = h.min(img.height() - y).max(1);
    imageops::crop_imm(img, x, y, w, h).to_image()
}

/// Rasterize window-space polylines to a tight white crop for OCR.
pub fn rasterize_strokes(lines: &[Vec<(f32, f32)>], stroke: u32) -> Option<RgbaImage> {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut any = false;
    for line in lines {
        for &(x, y) in line {
            any = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if !any {
        return None;
    }
    let pad = 24.0;
    let x0 = (min_x - pad).floor();
    let y0 = (min_y - pad).floor();
    let w = ((max_x - min_x) + pad * 2.0).ceil().max(64.0) as u32;
    let h = ((max_y - min_y) + pad * 2.0).ceil().max(64.0) as u32;
    let mut img = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
    let ink = Rgba([16, 16, 16, 255]);
    for line in lines {
        for pair in line.windows(2) {
            let ax = (pair[0].0 - x0) as i32;
            let ay = (pair[0].1 - y0) as i32;
            let bx = (pair[1].0 - x0) as i32;
            let by = (pair[1].1 - y0) as i32;
            draw_thick_line(&mut img, ax, ay, bx, by, stroke, ink);
        }
    }
    Some(img)
}

fn draw_thick_line(
    img: &mut RgbaImage,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    stroke: u32,
    color: Rgba<u8>,
) {
    let r = (stroke / 2).max(1) as i32;
    let mut x = x0;
    let mut y = y0;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        stamp(img, x, y, r, color);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn stamp(img: &mut RgbaImage, cx: i32, cy: i32, r: i32, color: Rgba<u8>) {
    let w = img.width() as i32;
    let h = img.height() as i32;
    for y in (cy - r)..=(cy + r) {
        for x in (cx - r)..=(cx + r) {
            if x >= 0
                && y >= 0
                && x < w
                && y < h
                && (x - cx) * (x - cx) + (y - cy) * (y - cy) <= r * r
            {
                img.put_pixel(x as u32, y as u32, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_strokes_yield_none() {
        assert!(rasterize_strokes(&[], 3).is_none());
        assert!(rasterize_strokes(&[vec![]], 3).is_none());
    }

    #[test]
    fn line_produces_ink() {
        let img = rasterize_strokes(&[vec![(10.0, 10.0), (40.0, 12.0)]], 3).unwrap();
        assert!(img.width() >= 64);
        assert!(img.pixels().any(|p| p.0[0] < 40));
    }
}
