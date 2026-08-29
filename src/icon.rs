use std::borrow::Cow;
#[cfg(any(test, target_os = "linux"))]
use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use gpui::{AssetSource, Image as GpuiImage, ImageFormat as GpuiImageFormat, SharedString};
#[cfg(any(test, target_os = "linux"))]
use image::ImageFormat;

#[cfg(any(test, target_os = "linux"))]
use crate::identity::APP_ID;
#[cfg(target_os = "linux")]
use crate::identity::{APP_NAME, APP_SLUG};

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
    "icons/paste.svg",
    "icons/draw.svg",
    "icons/word.svg",
    "icons/delete.svg",
    "icons/settings.svg",
    "icons/copy.svg",
    "icons/check.svg",
    "icons/zoom.svg",
    "icons/reset.svg",
    "icons/collapse.svg",
    "icons/expand.svg",
    "icons/corners.svg",
    "icons/close.svg",
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

/// Bitmap sizes dropped into hicolor. xfwm's theme fallback does not scale:
/// a 128-only install is clipped into the ~22×29 titlebar slot (black fan).
/// 22 and 24 are stock hicolor dirs; 16/32/48 match the Windows ICO ladder.
#[cfg(any(test, target_os = "linux"))]
const HICOLOR_PNG_SIZES: &[u32] = &[16, 22, 24, 32, 48, 128];

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

#[cfg(any(test, target_os = "linux"))]
pub fn png_bytes(size: u32) -> Result<Vec<u8>, image::ImageError> {
    let img = raster(size);
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
    Ok(out)
}

/// Drop a `.desktop` + hicolor icons so the WM can match `WM_CLASS` = `APP_ID`.
/// GPUI 0.2 has no window-icon API and does not set `_NET_WM_ICON`.
#[cfg(target_os = "linux")]
pub fn install_desktop_identity() {
    if let Err(err) = install_desktop_identity_inner() {
        eprintln!("{APP_SLUG}: desktop identity: {err}");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn install_desktop_identity() {}

#[cfg(any(test, target_os = "linux"))]
fn write_hicolor_icons(hicolor: &std::path::Path) -> Result<(), String> {
    let scalable = hicolor.join("scalable/apps");
    std::fs::create_dir_all(&scalable).map_err(|e| e.to_string())?;
    std::fs::write(scalable.join(format!("{APP_ID}.svg")), APP_ICON_SVG)
        .map_err(|e| e.to_string())?;
    for &size in HICOLOR_PNG_SIZES {
        let dir = hicolor.join(format!("{size}x{size}/apps"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let png = png_bytes(size).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{APP_ID}.png")), png).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_desktop_identity_inner() -> Result<(), String> {
    let data = dirs::data_dir().ok_or_else(|| "no XDG data dir".to_string())?;
    let app_dir = data.join("applications");
    std::fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    write_hicolor_icons(&data.join("icons/hicolor"))?;

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
            "icons/paste.svg",
            "icons/draw.svg",
            "icons/word.svg",
            "icons/delete.svg",
            "icons/settings.svg",
            "icons/copy.svg",
            "icons/check.svg",
            "icons/zoom.svg",
            "icons/reset.svg",
            "icons/collapse.svg",
            "icons/expand.svg",
            "icons/corners.svg",
            "icons/close.svg",
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

    #[test]
    fn hicolor_png_sizes_cover_titlebar_slots() {
        assert_eq!(HICOLOR_PNG_SIZES, &[16, 22, 24, 32, 48, 128]);
        for &size in HICOLOR_PNG_SIZES {
            let img = raster(size);
            assert_eq!(img.dimensions(), (size, size), "{size}px tile");
            let center = img.get_pixel(size / 2, size / 2).0;
            assert_eq!(
                center[3], 255,
                "{size}px center must be opaque (full tile, not a corner crop): {center:?}"
            );
        }
    }

    #[test]
    fn writes_hicolor_png_ladder() {
        let root = std::env::temp_dir().join(format!(
            "localtex-hicolor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("temp hicolor dir");
        write_hicolor_icons(&root).expect("write hicolor");
        assert!(root.join(format!("scalable/apps/{APP_ID}.svg")).is_file());
        for &size in HICOLOR_PNG_SIZES {
            let path = root.join(format!("{size}x{size}/apps/{APP_ID}.png"));
            assert!(path.is_file(), "missing {path:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
