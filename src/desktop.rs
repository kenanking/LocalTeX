use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::identity::APP_SLUG;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod other;
#[cfg(target_os = "windows")]
mod win;
#[cfg(target_os = "windows")]
mod win_clipboard;
#[cfg(target_os = "windows")]
mod win_cursor;
#[cfg(target_os = "windows")]
mod win_snip;
#[cfg(target_os = "linux")]
mod x11_snip;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopCmd {
    Capture,
    Show,
    Quit,
}

/// Keeps OS handles alive on the GPUI UI thread (Windows needs a win32 loop
/// on that thread; macOS needs the main thread).
struct Services {
    _hotkey: Option<GlobalHotKeyManager>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    _tray: Option<tray_icon::TrayIcon>,
}

thread_local! {
    static SERVICES: RefCell<Option<Services>> = const { RefCell::new(None) };
}

/// Register hotkey + tray. Must be called from the GPUI UI thread.
pub fn spawn() -> Receiver<DesktopCmd> {
    let (tx, rx) = mpsc::channel();
    let hotkey = register_hotkey(tx.clone());
    #[cfg(target_os = "linux")]
    linux::start_tray(tx);
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let tray = other::start_tray(tx);
    SERVICES.with(|slot| {
        *slot.borrow_mut() = Some(Services {
            _hotkey: hotkey,
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            _tray: tray,
        });
    });
    rx
}

fn register_hotkey(tx: Sender<DesktopCmd>) -> Option<GlobalHotKeyManager> {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(err) => {
            eprintln!("{APP_SLUG}: global hotkey manager failed: {err}");
            return None;
        }
    };
    let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyS);
    if let Err(err) = manager.register(hotkey) {
        eprintln!("{APP_SLUG}: register Ctrl+Shift+S failed: {err}");
        return None;
    }
    eprintln!("{APP_SLUG}: global hotkey Ctrl+Shift+S registered");
    let id = hotkey.id();
    thread::spawn(move || {
        let receiver = GlobalHotKeyEvent::receiver();
        while let Ok(event) = receiver.recv() {
            if event.id == id && event.state == HotKeyState::Pressed {
                if let Err(err) = tx.send(DesktopCmd::Capture) {
                    eprintln!("{APP_SLUG}: hotkey send: {err}");
                    break;
                }
            }
        }
    });
    Some(manager)
}

pub fn read_clipboard_image() -> Vec<Vec<u8>> {
    #[cfg(target_os = "windows")]
    {
        win_clipboard::read()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

pub fn decode_clipboard_image(bytes: Vec<u8>) -> anyhow::Result<image::RgbaImage> {
    match image::load_from_memory(&bytes) {
        Ok(img) => Ok(img.to_rgba8()),
        Err(_) => crate::imgutil::decode_dib(&bytes),
    }
}

/// Native snip overlay. Returns `Ok(None)` if the user cancelled.
/// Must run off the GPUI thread. Never opens a second GPUI/Vulkan window.
pub fn select_region(
    shot: &crate::capture::DesktopShot,
) -> anyhow::Result<Option<image::RgbaImage>> {
    #[cfg(target_os = "linux")]
    {
        x11_snip::select_region(shot)
    }
    #[cfg(target_os = "windows")]
    {
        win_snip::select_region(shot)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = shot;
        anyhow::bail!("snip overlay is not implemented on macOS yet")
    }
}

/// Iconify the main window so it is gone from the freeze-frame (Linux: EWMH
/// `_NET_WM_STATE_HIDDEN`; GPUI's ICCCM minimize is not enough on GNOME).
pub fn iconify_main_window() {
    #[cfg(target_os = "linux")]
    linux::iconify_main_window();
}

pub fn deiconify_main_window() {
    #[cfg(target_os = "linux")]
    linux::deiconify_main_window();
}

pub fn wait_until_iconified() {
    #[cfg(target_os = "linux")]
    linux::wait_until_iconified();
    #[cfg(target_os = "windows")]
    {
        win::wait_until_main_iconified();
    }
}

/// Call from the GPUI thread before a snip so the overlay can become
/// foreground and any leftover mouse capture is dropped.
pub fn prepare_snip_input() {
    #[cfg(target_os = "windows")]
    win::prepare_snip_input();
}

#[cfg(test)]
mod clipboard_decode_tests {
    use super::decode_clipboard_image;
    use image::{Rgba, RgbaImage};

    #[test]
    fn decode_prefers_png_magic() {
        let src = RgbaImage::from_pixel(2, 1, Rgba([10, 20, 30, 255]));
        let png = crate::imgutil::encode_png_fast(&src).unwrap();
        let out = decode_clipboard_image(png).unwrap();
        assert_eq!(out.dimensions(), (2, 1));
        assert_eq!(out.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn decode_falls_back_to_dib() {
        let mut h = vec![0u8; 40];
        h[0..4].copy_from_slice(&40u32.to_le_bytes());
        h[4..8].copy_from_slice(&2i32.to_le_bytes());
        h[8..12].copy_from_slice(&1i32.to_le_bytes());
        h[12..14].copy_from_slice(&1u16.to_le_bytes());
        h[14..16].copy_from_slice(&32u16.to_le_bytes());
        h.extend_from_slice(&[255, 0, 0, 255, 0, 0, 255, 255]);
        let out = decode_clipboard_image(h).unwrap();
        assert_eq!(out.dimensions(), (2, 1));
        assert_eq!(out.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(out.get_pixel(1, 0).0, [255, 0, 0, 255]);
    }
}
