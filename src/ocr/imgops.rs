//! Image helpers: RGB in-memory images, luma/invert, SIMD resize, crops,
//! vertical merging and formula margin cropping.

use anyhow::{anyhow, Result};
use fast_image_resize::images::Image as FirImage;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

#[derive(Clone, Debug)]
pub struct RgbImg {
    pub w: u32,
    pub h: u32,
    /// Interleaved RGB8, row-major.
    pub data: Vec<u8>,
}

impl RgbImg {
    pub fn new(w: u32, h: u32, data: Vec<u8>) -> Result<Self> {
        if data.len() != (w as usize) * (h as usize) * 3 {
            return Err(anyhow!("bad buffer size for {}x{}", w, h));
        }
        Ok(Self { w, h, data })
    }

    pub fn blank(w: u32, h: u32, fill: u8) -> Self {
        Self {
            w,
            h,
            data: vec![fill; (w as usize) * (h as usize) * 3],
        }
    }
}

/// cv2 COLOR_BGR2GRAY equivalent per pixel.
#[inline]
pub fn luma_of(r: u8, g: u8, b: u8) -> u8 {
    ((77u32 * r as u32 + 150u32 * g as u32 + 29u32 * b as u32 + 128) >> 8) as u8
}

/// Arithmetic mean of per-pixel luma.
pub fn mean_luma(img: &RgbImg) -> f64 {
    let mut acc: u64 = 0;
    for px in img.data.as_chunks::<3>().0 {
        acc += luma_of(px[0], px[1], px[2]) as u64;
    }
    acc as f64 / (img.w as f64 * img.h as f64)
}

/// Invert in place if mean luma < 128. Returns (inverted, luma).
pub fn invert_if_dark(img: &mut RgbImg) -> (bool, f64) {
    let luma = mean_luma(img);
    if luma < 128.0 {
        for v in img.data.iter_mut() {
            *v = 255 - *v;
        }
        (true, luma)
    } else {
        (false, luma)
    }
}

/// SIMD resize via fast_image_resize convolution filters.
pub fn resize(img: &RgbImg, w: u32, h: u32, filter: FilterType) -> Result<RgbImg> {
    if img.w == w && img.h == h {
        return Ok(img.clone());
    }
    let src = FirImage::from_vec_u8(img.w, img.h, img.data.clone(), PixelType::U8x3)
        .map_err(|e| anyhow!("fir src: {}", e))?;
    let mut dst = FirImage::new(w, h, PixelType::U8x3);
    let mut resizer = Resizer::new();
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter));
    resizer
        .resize(&src, &mut dst, &opts)
        .map_err(|e| anyhow!("fir resize: {}", e))?;
    RgbImg::new(w, h, dst.into_vec())
}

/// Crop [x1,y1,x2,y2) with int() truncation semantics; None if empty.
pub fn crop(img: &RgbImg, x1: f32, y1: f32, x2: f32, y2: f32) -> Option<RgbImg> {
    let ix1 = x1 as i64;
    let iy1 = y1 as i64;
    let ix2 = x2 as i64;
    let iy2 = y2 as i64;
    if ix2 <= ix1 || iy2 <= iy1 {
        return None;
    }
    // Python numpy slicing clamps to bounds; coords are pre-clipped anyway.
    let ix1 = ix1.max(0) as u32;
    let iy1 = iy1.max(0) as u32;
    let ix2 = (ix2.max(0) as u32).min(img.w);
    let iy2 = (iy2.max(0) as u32).min(img.h);
    if ix2 <= ix1 || iy2 <= iy1 {
        return None;
    }
    let (w, h) = (ix2 - ix1, iy2 - iy1);
    let mut out = vec![0u8; (w * h * 3) as usize];
    for row in 0..h {
        let src_o = (((iy1 + row) * img.w + ix1) * 3) as usize;
        let dst_o = (row * w * 3) as usize;
        out[dst_o..dst_o + (w * 3) as usize]
            .copy_from_slice(&img.data[src_o..src_o + (w * 3) as usize]);
    }
    Some(RgbImg { w, h, data: out })
}

/// Paste src onto dst at (x, y); both must fit.
fn paste(dst: &mut RgbImg, src: &RgbImg, x: u32, y: u32) {
    for row in 0..src.h {
        let so = (row * src.w * 3) as usize;
        let doff = (((y + row) * dst.w + x) * 3) as usize;
        dst.data[doff..doff + (src.w * 3) as usize]
            .copy_from_slice(&src.data[so..so + (src.w * 3) as usize]);
    }
}

/// Center a crop wider than 7.5:1 on a white canvas. This moves a
/// width-limited UniRec input from the 64 px height bucket to 128 px.
pub fn pad_wide_unirec_crop(img: &RgbImg) -> RgbImg {
    let target_h = img.w.saturating_mul(2).div_ceil(15);
    if target_h <= img.h {
        return img.clone();
    }
    let mut canvas = RgbImg::blank(img.w, target_h, 255);
    paste(&mut canvas, img, 0, (target_h - img.h) / 2);
    canvas
}

pub fn calc_merged_wh(imgs: &[&RgbImg]) -> (u32, u32) {
    let w = imgs.iter().map(|i| i.w).max().unwrap_or(0);
    let h = imgs.iter().map(|i| i.h).sum();
    (w, h)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Center,
    Right,
    Left,
}

/// Merge images vertically with per-step alignment (utils.py merge_images).
pub fn merge_images(imgs: &[&RgbImg], aligns: &[Align]) -> Option<RgbImg> {
    if imgs.is_empty() {
        return None;
    }
    if imgs.len() == 1 {
        return Some(imgs[0].clone());
    }
    let mut merged = imgs[0].clone();
    for (i, img2) in imgs.iter().enumerate().skip(1) {
        let align = aligns.get(i - 1).copied().unwrap_or(Align::Center);
        let w = merged.w.max(img2.w);
        let h = merged.h + img2.h;
        let (x1, x2) = match align {
            Align::Center => ((w - merged.w) / 2, (w - img2.w) / 2),
            Align::Right => (w - merged.w, w - img2.w),
            Align::Left => (0, 0),
        };
        let mut canvas = RgbImg::blank(w, h, 255);
        paste(&mut canvas, &merged, x1, 0);
        let mh = merged.h;
        paste(&mut canvas, img2, x2, mh);
        merged = canvas;
    }
    Some(merged)
}

/// utils.py crop_margin: crop to bounding box of dark-ish pixels after
/// min-max normalization of the grayscale image.
pub fn crop_margin(img: &RgbImg) -> RgbImg {
    let n = (img.w * img.h) as usize;
    let mut gray = vec![0u8; n];
    for (i, px) in img.data.as_chunks::<3>().0.iter().enumerate() {
        gray[i] = luma_of(px[0], px[1], px[2]);
    }
    let max_val = *gray.iter().max().unwrap_or(&0);
    let min_val = *gray.iter().min().unwrap_or(&0);
    if max_val == min_val {
        return img.clone();
    }
    let range = (max_val - min_val) as f64;
    // data = (gray - min) / (max - min) * 255, truncated to u8
    // binary (THRESH_BINARY_INV @ 200): nonzero where data <= 200
    let mut x_min = img.w;
    let mut y_min = img.h;
    let mut x_max = 0u32;
    let mut y_max = 0u32;
    let mut found = false;
    for y in 0..img.h {
        for x in 0..img.w {
            let g = gray[(y * img.w + x) as usize] as f64;
            let d = ((g - min_val as f64) / range * 255.0) as u8;
            if d <= 200 {
                found = true;
                x_min = x_min.min(x);
                y_min = y_min.min(y);
                x_max = x_max.max(x);
                y_max = y_max.max(y);
            }
        }
    }
    if !found {
        return img.clone();
    }
    // cv2.boundingRect: x, y, w, h with w = x_max - x_min + 1
    crop(
        img,
        x_min as f32,
        y_min as f32,
        (x_max + 1) as f32,
        (y_max + 1) as f32,
    )
    .unwrap_or_else(|| img.clone())
}
