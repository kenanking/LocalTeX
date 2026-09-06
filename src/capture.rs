use anyhow::{Context, Result, anyhow};
use image::RgbaImage;

/// One monitor grab in physical (device) pixels.
pub struct Grab {
    pub image: RgbaImage,
    /// Top-left of this grab on the virtual desktop, in physical pixels.
    pub origin_x: i32,
    pub origin_y: i32,
}

/// Stitched virtual-desktop freeze-frame in physical pixels.
pub struct DesktopShot {
    pub image: RgbaImage,
    /// Top-left of this canvas on the virtual desktop, in physical pixels.
    pub origin_x: i32,
    pub origin_y: i32,
    /// Physical rects of the grabs that built this canvas: `(x, y, w, h)`.
    /// Overlay placement must use these, not a second `Monitor::all()`.
    /// Linux snip uses the stitched canvas; Windows reads this for per-monitor HWNDs.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub monitors: Vec<(i32, i32, i32, i32)>,
}

/// Minimum drag (pixels) that counts as a snip. Smaller = cancel.
pub const MIN_SELECTION: i32 = 8;

/// Grab every monitor via xcap (Windows WGC, macOS, Linux X11).
pub fn grab_all() -> Result<Vec<Grab>> {
    let monitors = xcap::Monitor::all().context("xcap Monitor::all")?;
    if monitors.is_empty() {
        return Err(anyhow!("no monitor found"));
    }
    monitors.iter().map(grab_monitor).collect()
}

/// Stitch monitor grabs into one virtual-desktop canvas (gaps stay black).
pub fn stitch(grabs: &[Grab]) -> Result<DesktopShot> {
    let first = grabs.first().ok_or_else(|| anyhow!("no monitor found"))?;
    let mut min_x = first.origin_x;
    let mut min_y = first.origin_y;
    let mut max_x = first.origin_x.saturating_add_unsigned(first.image.width());
    let mut max_y = first.origin_y.saturating_add_unsigned(first.image.height());
    for grab in grabs.iter().skip(1) {
        min_x = min_x.min(grab.origin_x);
        min_y = min_y.min(grab.origin_y);
        max_x = max_x.max(grab.origin_x.saturating_add_unsigned(grab.image.width()));
        max_y = max_y.max(grab.origin_y.saturating_add_unsigned(grab.image.height()));
    }
    let width = u32::try_from(max_x.saturating_sub(min_x))
        .unwrap_or(0)
        .max(1);
    let height = u32::try_from(max_y.saturating_sub(min_y))
        .unwrap_or(0)
        .max(1);
    let mut image = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    for grab in grabs {
        // min_x/min_y are the infimum of origins, so offsets are non-negative.
        let dx = i64::from(grab.origin_x) - i64::from(min_x);
        let dy = i64::from(grab.origin_y) - i64::from(min_y);
        image::imageops::replace(&mut image, &grab.image, dx, dy);
    }
    let monitors = grabs
        .iter()
        .filter_map(|grab| {
            let w = i32::try_from(grab.image.width()).ok()?;
            let h = i32::try_from(grab.image.height()).ok()?;
            (w > 0 && h > 0).then_some((grab.origin_x, grab.origin_y, w, h))
        })
        .collect();
    Ok(DesktopShot {
        image,
        origin_x: min_x,
        origin_y: min_y,
        monitors,
    })
}

/// Crop a drag in canvas coordinates. Tiny clicks return `None` (cancel).
pub fn crop_selection(image: &RgbaImage, ax: i32, ay: i32, bx: i32, by: i32) -> Option<RgbaImage> {
    let w = i32::try_from(image.width()).ok()?;
    let h = i32::try_from(image.height()).ok()?;
    let x0 = ax.min(bx).clamp(0, w);
    let y0 = ay.min(by).clamp(0, h);
    let x1 = ax.max(bx).clamp(0, w);
    let y1 = ay.max(by).clamp(0, h);
    let cw = x1 - x0;
    let ch = y1 - y0;
    if cw < MIN_SELECTION || ch < MIN_SELECTION {
        return None;
    }
    Some(crate::imgutil::crop(
        image, x0 as u32, y0 as u32, cw as u32, ch as u32,
    ))
}

pub fn grab_desktop() -> Result<DesktopShot> {
    shot_from_grabs(grab_all()?)
}

fn shot_from_grabs(grabs: Vec<Grab>) -> Result<DesktopShot> {
    match grabs.len() {
        0 => stitch(&[]),
        1 => {
            let grab = grabs.into_iter().next().expect("len == 1");
            let w = i32::try_from(grab.image.width()).unwrap_or(0);
            let h = i32::try_from(grab.image.height()).unwrap_or(0);
            Ok(DesktopShot {
                origin_x: grab.origin_x,
                origin_y: grab.origin_y,
                monitors: vec![(grab.origin_x, grab.origin_y, w, h)],
                image: grab.image,
            })
        }
        _ => stitch(&grabs),
    }
}

/// BT.601 luma scaled to ~55% (overlay dim).
pub fn dim_luma(r: u8, g: u8, b: u8) -> u8 {
    let y = (u32::from(r) * 77 + u32::from(g) * 150 + u32::from(b) * 29) >> 8;
    (y * 140 / 255) as u8
}

/// Grayscale + dim for the unselected overlay (Mathpix-style). The
/// selected region is painted from the original freeze-frame.
#[cfg(any(target_os = "linux", test))]
pub fn dim_copy(image: &RgbaImage) -> RgbaImage {
    let (w, h) = image.dimensions();
    let src = image.as_raw();
    let mut out = Vec::with_capacity(src.len());
    for px in src.as_chunks::<4>().0 {
        let d = dim_luma(px[0], px[1], px[2]);
        out.extend_from_slice(&[d, d, d, px[3]]);
    }
    RgbaImage::from_raw(w, h, out).expect("dim_copy preserves pixel count")
}

fn grab_monitor(monitor: &xcap::Monitor) -> Result<Grab> {
    let image = monitor.capture_image().context("xcap capture_image")?;
    let x = monitor.x().unwrap_or(0);
    let y = monitor.y().unwrap_or(0);
    // Linux xcap reports x/y in logical pixels (Xft.dpi) while the bitmap is
    // physical. Windows WGC `dmPosition` is already physical — do not scale.
    #[cfg(target_os = "linux")]
    let (origin_x, origin_y) = {
        let scale = monitor.scale_factor().unwrap_or(1.0).max(0.01);
        (
            (x as f32 * scale).round() as i32,
            (y as f32 * scale).round() as i32,
        )
    };
    #[cfg(not(target_os = "linux"))]
    let (origin_x, origin_y) = (x, y);
    Ok(Grab {
        image,
        origin_x,
        origin_y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn grab(x: i32, y: i32, w: u32, h: u32, r: u8) -> Grab {
        Grab {
            image: RgbaImage::from_pixel(w, h, Rgba([r, 0, 0, 255])),
            origin_x: x,
            origin_y: y,
        }
    }

    #[test]
    fn stitch_rejects_empty() {
        assert!(stitch(&[]).is_err());
    }

    #[test]
    fn stitch_single_is_identity() {
        let g = grab(12, 34, 4, 2, 9);
        let shot = stitch(&[g]).unwrap();
        assert_eq!((shot.origin_x, shot.origin_y), (12, 34));
        assert_eq!(shot.image.dimensions(), (4, 2));
        assert_eq!(shot.image.get_pixel(0, 0).0, [9, 0, 0, 255]);
    }

    #[test]
    fn stitch_side_by_side_and_gap_is_black() {
        let left = grab(0, 0, 2, 2, 10);
        let right = grab(4, 0, 2, 2, 20);
        let shot = stitch(&[left, right]).unwrap();
        assert_eq!((shot.origin_x, shot.origin_y), (0, 0));
        assert_eq!(shot.image.dimensions(), (6, 2));
        assert_eq!(shot.image.get_pixel(0, 0).0, [10, 0, 0, 255]);
        assert_eq!(shot.image.get_pixel(4, 0).0, [20, 0, 0, 255]);
        assert_eq!(shot.image.get_pixel(2, 0).0, [0, 0, 0, 255]);
        assert_eq!(shot.monitors, vec![(0, 0, 2, 2), (4, 0, 2, 2)]);
    }

    #[test]
    fn stitch_negative_origin_normalizes_canvas() {
        let left = grab(-10, 0, 4, 2, 1);
        let right = grab(0, 0, 4, 2, 2);
        let shot = stitch(&[left, right]).unwrap();
        assert_eq!((shot.origin_x, shot.origin_y), (-10, 0));
        assert_eq!(shot.image.dimensions(), (14, 2));
        assert_eq!(shot.image.get_pixel(0, 0).0, [1, 0, 0, 255]);
        assert_eq!(shot.image.get_pixel(10, 0).0, [2, 0, 0, 255]);
    }

    #[test]
    fn crop_selection_tiny_click_cancels() {
        let img = RgbaImage::from_pixel(100, 80, Rgba([1, 2, 3, 255]));
        assert!(crop_selection(&img, 10, 10, 12, 12).is_none());
        assert!(crop_selection(&img, 10, 10, 10 + MIN_SELECTION - 1, 10 + 40).is_none());
    }

    #[test]
    fn crop_selection_returns_normalized_rect() {
        let mut img = RgbaImage::from_pixel(20, 20, Rgba([0, 0, 0, 255]));
        for y in 5..15 {
            for x in 3..13 {
                img.put_pixel(x, y, Rgba([7, 0, 0, 255]));
            }
        }
        let crop = crop_selection(&img, 12, 14, 3, 5).unwrap();
        assert_eq!(crop.dimensions(), (9, 9));
        assert_eq!(crop.get_pixel(0, 0).0, [7, 0, 0, 255]);
    }

    #[test]
    fn dim_copy_is_gray_and_darker() {
        let img = RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255]));
        let dim = dim_copy(&img);
        let p = dim.get_pixel(0, 0).0;
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
        assert!(p[0] > 0 && p[0] < 80, "dimmed luma, got {}", p[0]);
        assert_eq!(p[3], 255);
        assert_ne!(p, [255, 0, 0, 255]);
    }

    #[test]
    fn single_grab_does_not_restitch() {
        let g = grab(12, 34, 4, 2, 9);
        let ptr = g.image.as_ptr();
        let shot = shot_from_grabs(vec![g]).unwrap();
        assert_eq!(shot.image.as_ptr(), ptr);
        assert_eq!((shot.origin_x, shot.origin_y), (12, 34));
        assert_eq!(shot.monitors, vec![(12, 34, 4, 2)]);
    }
}
