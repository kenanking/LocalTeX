use std::borrow::Cow;
#[cfg(target_os = "linux")]
use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use gpui::{AssetSource, Image as GpuiImage, ImageFormat as GpuiImageFormat, SharedString};
#[cfg(target_os = "linux")]
use image::ImageFormat;

#[cfg(target_os = "linux")]
use crate::identity::{APP_ID, APP_NAME, APP_SLUG};

#[path = "icon_mark.rs"]
mod icon_mark;

pub use icon_mark::raster;

/// App tile (rounded square + mark). Same bytes as `assets/icon.svg`.
pub const APP_ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");

/// Polychrome app tile for `img()`. `img("icon.svg")` does not paint on this
/// NVIDIA/Vulkan host (toolbar `img("icons/*.svg")` does). `from_bytes` is
/// the path that actually shows the tile.
pub fn app_tile_image() -> Arc<GpuiImage> {
    static TILE: OnceLock<Arc<GpuiImage>> = OnceLock::new();
    TILE.get_or_init(|| {
        Arc::new(GpuiImage::from_bytes(
            GpuiImageFormat::Svg,
            APP_ICON_SVG.to_vec(),
        ))
    })
    .clone()
}

/// Embedded UI + app-tile SVGs. GPUI `img("icons/foo.svg")` and `svg().path`
/// both go through this map — no disk I/O on the render path.
pub struct Assets;

macro_rules! bundled {
    ($($path:literal),+ $(,)?) => {
        &[$((
            $path,
            include_bytes!(concat!("../assets/", $path)).as_slice(),
        )),+]
    };
}

const BUNDLED: &[(&str, &[u8])] = bundled![
    "icon.svg",
    "icons/snip.svg",
    "icons/upload.svg",
    "icons/draw.svg",
    "icons/delete.svg",
    "icons/settings.svg",
    "icons/copy.svg",
    "icons/zoom.svg",
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(BUNDLED
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(BUNDLED
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

pub fn rgba_bytes(size: u32) -> (u32, u32, Vec<u8>) {
    let img = raster(size);
    (img.width(), img.height(), img.into_raw())
}

/// ARGB32 in network byte order (StatusNotifierItem / ksni).
#[cfg(target_os = "linux")]
pub fn argb_bytes(size: u32) -> (i32, i32, Vec<u8>) {
    let (w, h, mut data) = rgba_bytes(size);
    for px in data.as_chunks_mut::<4>().0 {
        px.rotate_right(1);
    }
    (w as i32, h as i32, data)
}

#[cfg(target_os = "linux")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource;

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
        assert!(
            text.contains(r#"width="256" height="256""#),
            "app tile svg must declare size so img() raster is not empty"
        );
        assert_eq!(app_tile_image().format(), GpuiImageFormat::Svg);
    }

    #[test]
    fn toolbar_icon_files_are_embedded_and_hidpi() {
        for path in [
            "icons/snip.svg",
            "icons/upload.svg",
            "icons/draw.svg",
            "icons/delete.svg",
            "icons/settings.svg",
            "icons/copy.svg",
            "icons/zoom.svg",
        ] {
            let bytes = Assets
                .load(path)
                .expect("asset load")
                .unwrap_or_else(|| panic!("missing embedded icon: {path}"));
            let text = std::str::from_utf8(&bytes).expect("utf8");
            assert!(
                text.contains(r#"width="96" height="96""#),
                "{path} must declare 96px so img() raster is ~192px after GPUI's 2× pass"
            );
            assert!(
                text.contains(r#"viewBox="0 0 24 24""#),
                "{path} must stay on a 24-unit Lucide viewBox"
            );
        }
        assert!(
            Assets.load("icon.svg").expect("asset load").is_some(),
            "app tile svg must be on the asset map"
        );
    }
}
