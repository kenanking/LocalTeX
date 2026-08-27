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

/// Native snip overlay (Linux X11: override-redirect freeze-frame).
/// Returns `Ok(None)` if the user cancelled. Must run off the GPUI thread.
pub fn select_region(
    shot: &crate::capture::DesktopShot,
) -> anyhow::Result<Option<image::RgbaImage>> {
    #[cfg(target_os = "linux")]
    {
        x11_snip::select_region(shot)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = shot;
        anyhow::bail!(
            "snip overlay is Linux X11 only for now (Windows/macOS: do not open a second GPUI window)"
        )
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
}
