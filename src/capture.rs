use anyhow::{anyhow, Context, Result};
use image::RgbaImage;

/// A primary-monitor grab in physical (device) pixels.
pub struct Grab {
    pub image: RgbaImage,
    /// Top-left of this grab on the virtual desktop, in physical pixels.
    pub origin_x: i32,
    pub origin_y: i32,
}

/// Grab the primary monitor via xcap (Windows WGC, macOS, Linux X11).
pub fn grab_primary() -> Result<Grab> {
    let monitors = xcap::Monitor::all().context("xcap Monitor::all")?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .ok_or_else(|| anyhow!("no monitor found"))?;

    let image = monitor.capture_image().context("xcap capture_image")?;
    // Some backends report x/y in logical pixels while the bitmap is physical
    // (X11 Xft.dpi). Convert origin back to physical so overlay placement can
    // line up 1:1 with the desktop. Overlay drawing uses GPUI's scale_factor,
    // not a second manual DPI scale.
    let scale = monitor.scale_factor().unwrap_or(1.0).max(0.01);
    let origin_x = ((monitor.x().unwrap_or(0) as f32) * scale).round() as i32;
    let origin_y = ((monitor.y().unwrap_or(0) as f32) * scale).round() as i32;

    Ok(Grab {
        image,
        origin_x,
        origin_y,
    })
}
