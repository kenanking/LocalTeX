use std::sync::Arc;

#[cfg(any(test, target_os = "windows"))]
use anyhow::{anyhow, Result};
use gpui::RenderImage;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::{imageops, ColorType, ImageEncoder, Rgba, RgbaImage};

pub fn rgba_to_render(img: &RgbaImage) -> Arc<RenderImage> {
    let mut bgra = img.clone();
    for pixel in bgra.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let frame = image::Frame::new(bgra);
    Arc::new(RenderImage::new(smallvec::smallvec![frame]))
}

/// GPU display texture. Blade's Linux atlas grows to the bitmap size with
/// no clamp (Metal/DX cap at 16k). Keep the original `RgbaImage` for
/// cropping / OCR; this is display only.
const GPU_DISPLAY_MAX_EDGE: u32 = 1024;

pub fn gpu_display_image(img: &RgbaImage) -> Arc<RenderImage> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return rgba_to_render(img);
    }
    if w <= GPU_DISPLAY_MAX_EDGE && h <= GPU_DISPLAY_MAX_EDGE {
        return rgba_to_render(img);
    }
    rgba_to_render(&thumbnail(img, GPU_DISPLAY_MAX_EDGE, GPU_DISPLAY_MAX_EDGE))
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

/// Bound decoded snips before OCR so layout boxes stay in the stored-image
/// coordinate system. 24 MP is the ocr-pipeline recommendation for LocalTeX.
pub const OCR_MAX_PIXELS: u64 = 24_000_000;

pub fn cap_megapixels(img: RgbaImage) -> RgbaImage {
    cap_megapixels_at(img, OCR_MAX_PIXELS)
}

pub fn cap_megapixels_at(img: RgbaImage, max_px: u64) -> RgbaImage {
    let (w, h) = img.dimensions();
    let px = u64::from(w) * u64::from(h);
    if px <= max_px || w == 0 || h == 0 {
        return img;
    }
    let scale = (max_px as f64 / px as f64).sqrt();
    let tw = ((w as f64) * scale).round().max(1.0) as u32;
    let th = ((h as f64) * scale).round().max(1.0) as u32;
    imageops::resize(&img, tw, th, imageops::FilterType::Triangle)
}

/// Windows CF_DIB / CF_DIBV5 payload: BITMAPINFO header + bits, no BITMAPFILEHEADER.
/// 32-bit high byte is unused on BI_RGB clipboard dumps, so alpha is forced opaque.
#[cfg(any(test, target_os = "windows"))]
pub fn decode_dib(data: &[u8]) -> Result<RgbaImage> {
    if data.len() < 40 {
        return Err(anyhow!("DIB too small"));
    }
    let bi_size = u32::from_le_bytes(data[0..4].try_into()?);
    if !(40..=data.len() as u32).contains(&bi_size) {
        return Err(anyhow!("bad BITMAPINFOHEADER size {bi_size}"));
    }
    let width = i32::from_le_bytes(data[4..8].try_into()?);
    let height_s = i32::from_le_bytes(data[8..12].try_into()?);
    let bit_count = u16::from_le_bytes(data[14..16].try_into()?);
    let compression = u32::from_le_bytes(data[16..20].try_into()?);
    if width <= 0 || height_s == 0 {
        return Err(anyhow!("bad DIB geometry {width}x{height_s}"));
    }
    let top_down = height_s < 0;
    let height = height_s.unsigned_abs();
    let width = width as u32;

    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    if compression != BI_RGB && compression != BI_BITFIELDS {
        return Err(anyhow!("unsupported DIB compression {compression}"));
    }

    let mut pix_off = bi_size as usize;
    if compression == BI_BITFIELDS {
        if bi_size != 40 && bi_size < 52 {
            return Err(anyhow!("DIB bitfield header is incomplete"));
        }
        let masks = data
            .get(40..52)
            .ok_or_else(|| anyhow!("DIB masks are missing"))?;
        for (bytes, expected) in
            masks
                .as_chunks::<4>()
                .0
                .iter()
                .zip([0x00ff0000u32, 0x0000ff00, 0x000000ff])
        {
            if u32::from_le_bytes(*bytes) != expected {
                return Err(anyhow!("unsupported DIB bitfield masks"));
            }
        }
    }
    // BITMAPINFO + BI_BITFIELDS stores three DWORD masks after a 40-byte header.
    if compression == BI_BITFIELDS && bi_size == 40 {
        pix_off = pix_off.saturating_add(12);
    }
    let pixels = data
        .get(pix_off..)
        .ok_or_else(|| anyhow!("DIB pixel offset past end"))?;

    match bit_count {
        32 => decode_dib_bgra(width, height, top_down, 4, pixels),
        24 => decode_dib_bgra(width, height, top_down, 3, pixels),
        other => Err(anyhow!("unsupported DIB bit count {other}")),
    }
}

#[cfg(any(test, target_os = "windows"))]
fn decode_dib_bgra(
    width: u32,
    height: u32,
    top_down: bool,
    bpp: usize,
    pixels: &[u8],
) -> Result<RgbaImage> {
    let stride = (width as usize * bpp).div_ceil(4) * 4;
    let need = stride.saturating_mul(height as usize);
    if pixels.len() < need {
        return Err(anyhow!("DIB pixel buffer short: {} < {need}", pixels.len()));
    }
    let mut img = RgbaImage::new(width, height);
    for y in 0..height {
        let src_y = if top_down { y } else { height - 1 - y };
        let row = &pixels[(src_y as usize) * stride..];
        for x in 0..width {
            let o = x as usize * bpp;
            img.put_pixel(x, y, Rgba([row[o + 2], row[o + 1], row[o], 255]));
        }
    }
    Ok(img)
}

pub fn encode_png_fast(img: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut buf, CompressionType::Fast, PngFilter::Adaptive);
    encoder.write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        ColorType::Rgba8.into(),
    )?;
    Ok(buf)
}

pub fn encode_thumb_jpeg(img: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let thumb = thumbnail(img, 112, 80);
    let rgb = image::DynamicImage::ImageRgba8(thumb).to_rgb8();
    let mut buf = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut buf, 70);
    encoder.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        ColorType::Rgb8.into(),
    )?;
    Ok(buf)
}

pub fn jpeg_to_render(jpeg: &[u8]) -> Option<Arc<RenderImage>> {
    if jpeg.is_empty() {
        return None;
    }
    let img = image::load_from_memory(jpeg).ok()?.to_rgba8();
    Some(rgba_to_render(&img))
}

pub fn decode_png_file(path: &std::path::Path) -> anyhow::Result<RgbaImage> {
    Ok(image::open(path)?.to_rgba8())
}

pub fn crop(img: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
    let x = x.min(img.width().saturating_sub(1));
    let y = y.min(img.height().saturating_sub(1));
    let w = w.min(img.width() - x).max(1);
    let h = h.min(img.height() - y).max(1);
    imageops::crop_imm(img, x, y, w, h).to_image()
}

/// Rasterize window-space polylines to a tight white crop for the library PNG.
pub fn traces_xy(traces: &[Vec<[f32; 3]>]) -> Vec<Vec<(f32, f32)>> {
    traces
        .iter()
        .map(|line| line.iter().map(|p| (p[0], p[1])).collect())
        .collect()
}

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
    #[test]
    fn dib_bitfields_accept_standard_masks_and_reject_other_layouts() {
        let mut dib = vec![0u8; 56];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[16..20].copy_from_slice(&3u32.to_le_bytes());
        for (offset, mask) in [(40, 0xff0000u32), (44, 0xff00), (48, 0xff)] {
            dib[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
        }
        dib[52..56].copy_from_slice(&[10, 20, 30, 0]);
        assert_eq!(
            super::decode_dib(&dib).unwrap().get_pixel(0, 0).0,
            [30, 20, 10, 255]
        );
        dib[40..44].copy_from_slice(&0xffu32.to_le_bytes());
        assert!(super::decode_dib(&dib).is_err());
    }
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

    #[test]
    fn traces_xy_drops_time() {
        let xy = traces_xy(&[vec![[1.0, 2.0, 9.0], [3.0, 4.0, 10.0]]]);
        assert_eq!(xy, vec![vec![(1.0, 2.0), (3.0, 4.0)]]);
    }

    #[test]
    fn gpu_display_image_caps_long_edge() {
        let img = RgbaImage::from_pixel(1920, 1080, Rgba([10, 20, 30, 255]));
        let render = gpu_display_image(&img);
        let size = render.size(0);
        let w = u32::from(size.width);
        let h = u32::from(size.height);
        assert!(w <= GPU_DISPLAY_MAX_EDGE, "w={w}");
        assert!(h <= GPU_DISPLAY_MAX_EDGE, "h={h}");
        assert!((w as f32 / h as f32 - 1920.0 / 1080.0).abs() < 0.02);
    }

    #[test]
    fn gpu_display_image_keeps_small_shots() {
        let img = RgbaImage::from_pixel(64, 48, Rgba([1, 2, 3, 255]));
        let render = gpu_display_image(&img);
        let size = render.size(0);
        assert_eq!(u32::from(size.width), 64);
        assert_eq!(u32::from(size.height), 48);
    }

    #[test]
    fn cap_megapixels_keeps_1080p() {
        let img = RgbaImage::from_pixel(1920, 1080, Rgba([1, 2, 3, 255]));
        let out = cap_megapixels(img);
        assert_eq!(out.dimensions(), (1920, 1080));
    }

    #[test]
    fn cap_megapixels_shrinks_huge_scans() {
        let img = RgbaImage::from_pixel(400, 400, Rgba([1, 2, 3, 255]));
        let out = cap_megapixels_at(img, 90_000);
        let px = u64::from(out.width()) * u64::from(out.height());
        assert!(px <= 90_000);
        assert!(out.width() > 200 && out.height() > 200);
    }

    fn packed_dib32(
        width: i32,
        height: i32,
        compression: u32,
        extra: &[u8],
        pixels: &[u8],
    ) -> Vec<u8> {
        let mut h = vec![0u8; 40];
        h[0..4].copy_from_slice(&40u32.to_le_bytes());
        h[4..8].copy_from_slice(&width.to_le_bytes());
        h[8..12].copy_from_slice(&height.to_le_bytes());
        h[12..14].copy_from_slice(&1u16.to_le_bytes());
        h[14..16].copy_from_slice(&32u16.to_le_bytes());
        h[16..20].copy_from_slice(&compression.to_le_bytes());
        let mut out = h;
        out.extend_from_slice(extra);
        out.extend_from_slice(pixels);
        out
    }

    #[test]
    fn decode_dib32_bottom_up_bgr() {
        // One bottom-up row: blue then red (Windows clipboard CF_DIB).
        let dib = packed_dib32(2, 1, 0, &[], &[255, 0, 0, 255, 0, 0, 255, 255]);
        let img = decode_dib(&dib).unwrap();
        assert_eq!(img.dimensions(), (2, 1));
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn decode_dib32_bitfields_skips_masks() {
        let masks = [0x00FF0000u32, 0x0000FF00, 0x000000FF]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let dib = packed_dib32(1, -1, 3, &masks, &[0, 255, 0, 255]);
        let img = decode_dib(&dib).unwrap();
        assert_eq!(img.dimensions(), (1, 1));
        assert_eq!(img.get_pixel(0, 0).0, [0, 255, 0, 255]);
    }

    #[test]
    fn decode_dib24_row_padding() {
        let mut h = vec![0u8; 40];
        h[0..4].copy_from_slice(&40u32.to_le_bytes());
        h[4..8].copy_from_slice(&1i32.to_le_bytes());
        h[8..12].copy_from_slice(&1i32.to_le_bytes());
        h[12..14].copy_from_slice(&1u16.to_le_bytes());
        h[14..16].copy_from_slice(&24u16.to_le_bytes());
        h.extend_from_slice(&[0, 0, 255, 0]); // BGR + pad to 4
        let img = decode_dib(&h).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }
}
