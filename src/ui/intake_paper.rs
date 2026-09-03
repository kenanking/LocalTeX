use image::{imageops, GrayImage, Luma, Rgba, RgbaImage};

pub(crate) const INTAKE_PAPER_SIZE: u32 = 82;
const INTAKE_PAPER_SCALE: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct IntakePaperSpec {
    pub degrees: f32,
    pub edge_seed: u64,
}

pub(crate) fn intake_paper_spec(batch_gen: u64, item_key: u64, slot: usize) -> IntakePaperSpec {
    let seed = mix64(
        batch_gen
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(item_key.wrapping_mul(0xbf58_476d_1ce4_e5b9))
            .wrapping_add((slot as u64).wrapping_mul(0x94d0_49bb_1331_11eb)),
    );
    let magnitude = 2.5 + unit_f32(seed.rotate_right(11)) * 6.5;
    let sign = if seed & 1 == 0 { -1.0 } else { 1.0 };
    IntakePaperSpec {
        degrees: sign * magnitude,
        edge_seed: mix64(seed ^ 0xd1b5_4a32_d192_ed03),
    }
}

pub(crate) fn intake_paper_image(img: &RgbaImage, spec: IntakePaperSpec) -> RgbaImage {
    let (base, polygon) = intake_paper_base(img, spec.edge_seed, spec.degrees);
    finish_intake_paper(
        &base,
        &polygon,
        spec.degrees,
        Rgba([201, 197, 188, 255]),
        0.9,
    )
}

fn intake_paper_base(img: &RgbaImage, seed: u64, degrees: f32) -> (RgbaImage, Vec<(f32, f32)>) {
    let scale = INTAKE_PAPER_SCALE as f32;
    let size = INTAKE_PAPER_SIZE * INTAKE_PAPER_SCALE;
    let margin = intake_paper_margin(degrees);
    let polygon = torn_paper_polygon(seed, scale, margin);
    let mut mask = GrayImage::new(size, size);
    for y in 0..size {
        for x in 0..size {
            if point_in_polygon(&polygon, x as f32 + 0.5, y as f32 + 0.5) {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
    }

    let blurred = imageops::blur(&mask, 1.45 * scale);
    let mut paper = RgbaImage::new(size, size);
    let shadow_shift = (2.2 * scale).round() as u32;
    for y in shadow_shift..size {
        for x in 0..size {
            let alpha = u16::from(blurred.get_pixel(x, y - shadow_shift).0[0]) * 30 / 255;
            if alpha > 0 {
                paper.put_pixel(x, y, Rgba([30, 30, 34, alpha as u8]));
            }
        }
    }
    for (x, y, pixel) in mask.enumerate_pixels() {
        if pixel.0[0] != 0 {
            paper.put_pixel(x, y, Rgba([255, 254, 250, 255]));
        }
    }

    let inset = ((margin + 3.2) * scale).round() as u32;
    let inner = size - inset * 2;
    for y in inset..inset + inner {
        for x in inset..inset + inner {
            paper.put_pixel(x, y, Rgba([248, 248, 248, 255]));
        }
    }
    let thumb = crate::imgutil::thumbnail(img, inner, inner);
    imageops::overlay(
        &mut paper,
        &thumb,
        i64::from((size - thumb.width()) / 2),
        i64::from((size - thumb.height()) / 2),
    );
    paint_rect_stroke(
        &mut paper,
        inset,
        inset,
        inner,
        inner,
        INTAKE_PAPER_SCALE.max(2),
        Rgba([229, 227, 223, 255]),
    );
    (paper, polygon)
}

fn finish_intake_paper(
    base: &RgbaImage,
    polygon: &[(f32, f32)],
    degrees: f32,
    border: Rgba<u8>,
    border_width: f32,
) -> RgbaImage {
    let mut framed = base.clone();
    paint_polygon_stroke(
        &mut framed,
        polygon,
        border_width * INTAKE_PAPER_SCALE as f32,
        border,
    );
    let rotated = rotate_about_center_fixed(&framed, degrees);
    imageops::resize(
        &rotated,
        INTAKE_PAPER_SIZE,
        INTAKE_PAPER_SIZE,
        imageops::FilterType::Lanczos3,
    )
}

fn intake_paper_margin(degrees: f32) -> f32 {
    const BASE_MARGIN: f32 = 3.8;
    const MAX_JITTER: f32 = 1.7;
    const EDGE_CLEARANCE: f32 = 1.5;

    let radians = degrees.to_radians();
    let rotated_span = radians.cos().abs() + radians.sin().abs();
    let half_canvas = INTAKE_PAPER_SIZE as f32 * 0.5;
    let safe_half_extent = (half_canvas - EDGE_CLEARANCE) / rotated_span;
    BASE_MARGIN.max(half_canvas + MAX_JITTER - safe_half_extent)
}

fn torn_paper_polygon(seed: u64, scale: f32, margin: f32) -> Vec<(f32, f32)> {
    let mut rng = PaperRng(seed);
    let lo = margin * scale;
    let hi = (INTAKE_PAPER_SIZE as f32 - margin) * scale;
    let mut points = Vec::new();
    for x in edge_positions(&mut rng, lo, hi, scale) {
        points.push((x, lo + paper_jitter(&mut rng, scale)));
    }
    for y in edge_positions(&mut rng, lo, hi, scale).into_iter().skip(1) {
        points.push((hi + paper_jitter(&mut rng, scale), y));
    }
    for x in edge_positions(&mut rng, lo, hi, scale)
        .into_iter()
        .skip(1)
        .rev()
    {
        points.push((x, hi + paper_jitter(&mut rng, scale)));
    }
    let left = edge_positions(&mut rng, lo, hi, scale);
    for &y in left[1..left.len() - 1].iter().rev() {
        points.push((lo + paper_jitter(&mut rng, scale), y));
    }
    points
}

fn edge_positions(rng: &mut PaperRng, start: f32, end: f32, scale: f32) -> Vec<f32> {
    let mut out = vec![start];
    let mut value = start;
    while value < end - 5.0 * scale {
        value = (value + (5.0 + rng.unit() * 5.0) * scale).min(end);
        out.push(value);
    }
    if out.last().is_none_or(|value| *value < end) {
        out.push(end);
    }
    out
}

fn paper_jitter(rng: &mut PaperRng, scale: f32) -> f32 {
    let sign = if rng.next_u64() & 1 == 0 { -1.0 } else { 1.0 };
    sign * (0.35 + rng.unit() * 1.35) * scale
}

fn point_in_polygon(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let (xi, yi) = points[current];
        let (xj, yj) = points[previous];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn paint_polygon_stroke(image: &mut RgbaImage, points: &[(f32, f32)], width: f32, color: Rgba<u8>) {
    let radius_sq = (width * 0.5).powi(2);
    let radius = width * 0.5;
    for (index, &a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        let min_x = (a.0.min(b.0) - radius).floor().max(0.0) as u32;
        let max_x = (a.0.max(b.0) + radius)
            .ceil()
            .min(image.width().saturating_sub(1) as f32) as u32;
        let min_y = (a.1.min(b.1) - radius).floor().max(0.0) as u32;
        let max_y = (a.1.max(b.1) + radius)
            .ceil()
            .min(image.height().saturating_sub(1) as f32) as u32;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                if distance_to_segment_sq(p, a, b) <= radius_sq {
                    image.put_pixel(x, y, color);
                }
            }
        }
    }
}

fn distance_to_segment_sq(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let ab = (b.0 - a.0, b.1 - a.1);
    let ap = (p.0 - a.0, p.1 - a.1);
    let len_sq = ab.0 * ab.0 + ab.1 * ab.1;
    let t = if len_sq == 0.0 {
        0.0
    } else {
        ((ap.0 * ab.0 + ap.1 * ab.1) / len_sq).clamp(0.0, 1.0)
    };
    let dx = p.0 - (a.0 + ab.0 * t);
    let dy = p.1 - (a.1 + ab.1 * t);
    dx * dx + dy * dy
}

fn paint_rect_stroke(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    stroke: u32,
    color: Rgba<u8>,
) {
    for py in y..y + height {
        for px in x..x + width {
            if px < x + stroke
                || px >= x + width - stroke
                || py < y + stroke
                || py >= y + height - stroke
            {
                image.put_pixel(px, py, color);
            }
        }
    }
}

fn rotate_about_center_fixed(src: &RgbaImage, degrees: f32) -> RgbaImage {
    let rad = degrees.to_radians();
    let (cos, sin) = (rad.cos(), rad.sin());
    let (w, h) = src.dimensions();
    let mut out = RgbaImage::new(w, h);
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let sx = cos * dx + sin * dy + cx;
            let sy = -sin * dx + cos * dy + cy;
            if let Some(pixel) = sample_bilinear(src, sx, sy) {
                out.put_pixel(x, y, pixel);
            }
        }
    }
    out
}

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit_f32(value: u64) -> f32 {
    ((value >> 40) as f32) / ((1u32 << 24) as f32)
}

struct PaperRng(u64);

impl PaperRng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix64(self.0)
    }

    fn unit(&mut self) -> f32 {
        unit_f32(self.next_u64())
    }
}

fn sample_bilinear(src: &RgbaImage, x: f32, y: f32) -> Option<Rgba<u8>> {
    let (w, h) = src.dimensions();
    if x < -0.5 || y < -0.5 || x > w as f32 - 0.5 || y > h as f32 - 0.5 {
        return None;
    }
    let x0 = x.floor().clamp(0.0, (w - 1) as f32);
    let y0 = y.floor().clamp(0.0, (h - 1) as f32);
    let x1 = (x0 + 1.0).min((w - 1) as f32);
    let y1 = (y0 + 1.0).min((h - 1) as f32);
    let tx = (x - x0).clamp(0.0, 1.0);
    let ty = (y - y0).clamp(0.0, 1.0);
    let p00 = src.get_pixel(x0 as u32, y0 as u32).0;
    let p10 = src.get_pixel(x1 as u32, y0 as u32).0;
    let p01 = src.get_pixel(x0 as u32, y1 as u32).0;
    let p11 = src.get_pixel(x1 as u32, y1 as u32).0;
    let mut out = [0u8; 4];
    for i in 0..4 {
        let a = p00[i] as f32 * (1.0 - tx) + p10[i] as f32 * tx;
        let b = p01[i] as f32 * (1.0 - tx) + p11[i] as f32 * tx;
        out[i] = (a * (1.0 - ty) + b * ty).round() as u8;
    }
    Some(Rgba(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intake_paper_image_is_stable() {
        let img = RgbaImage::from_pixel(120, 80, Rgba([12, 40, 90, 255]));
        let spec = intake_paper_spec(4, 7, 2);
        let first = intake_paper_image(&img, spec);
        let second = intake_paper_image(&img, spec);
        assert_eq!(first.dimensions(), (INTAKE_PAPER_SIZE, INTAKE_PAPER_SIZE));
        assert_eq!(first, second);
        assert!(first.pixels().any(|pixel| pixel.0[3] == 0));
        assert!(first
            .pixels()
            .any(|pixel| pixel.0[0] > 245 && pixel.0[3] == 255));
    }

    #[test]
    fn intake_paper_specs_are_stable_varied_and_bounded() {
        let first = intake_paper_spec(3, 9, 1);
        assert_eq!(first, intake_paper_spec(3, 9, 1));
        assert_ne!(first, intake_paper_spec(3, 10, 1));
        let specs: Vec<_> = (0..32)
            .map(|key| intake_paper_spec(8, key, key as usize % 5))
            .collect();
        assert!(specs.iter().all(|spec| {
            let magnitude = spec.degrees.abs();
            (2.5..=9.0).contains(&magnitude)
        }));
        assert!(specs.iter().any(|spec| spec.degrees < 0.0));
        assert!(specs.iter().any(|spec| spec.degrees > 0.0));
    }

    #[test]
    fn intake_paper_edge_changes_with_seed() {
        let img = RgbaImage::from_pixel(120, 80, Rgba([12, 40, 90, 255]));
        let first = intake_paper_image(&img, intake_paper_spec(1, 1, 0));
        let second = intake_paper_image(&img, intake_paper_spec(1, 2, 0));
        assert_ne!(first, second);
    }

    #[test]
    fn intake_paper_keeps_opaque_edges_inside_canvas_at_max_tilt() {
        let img = RgbaImage::from_pixel(120, 80, Rgba([12, 40, 90, 255]));
        for edge_seed in 0..8 {
            for degrees in [-9.0, 9.0] {
                let rendered = intake_paper_image(&img, IntakePaperSpec { degrees, edge_seed });
                let last = rendered.width() - 1;
                assert!((0..rendered.width()).all(|position| {
                    rendered.get_pixel(position, 0).0[3] < 128
                        && rendered.get_pixel(position, last).0[3] < 128
                        && rendered.get_pixel(0, position).0[3] < 128
                        && rendered.get_pixel(last, position).0[3] < 128
                }));
            }
        }
    }
}
