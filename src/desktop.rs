use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use gpui::{Bounds, Pixels, Window, WindowBounds};

use crate::identity::APP_SLUG;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod other;

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
    static SERVICES: RefCell<Option<Services>> = RefCell::new(None);
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

/// Linux: keep one GPUI overlay window (unmap/map). Win/mac: destroy each snip.
pub fn overlay_keeps_window() -> bool {
    cfg!(target_os = "linux")
}

/// Allow `raise_overlay` to map/restack. Call once per snip session.
pub fn arm_overlay() {
    #[cfg(target_os = "linux")]
    linux::arm_overlay();
}

/// Raise the snip overlay if the session is armed. No-op after `park_overlay`.
pub fn raise_overlay() {
    #[cfg(target_os = "linux")]
    linux::raise_overlay();
}

/// Disarm and unmap. In-flight raises become no-ops.
pub fn park_overlay() {
    #[cfg(target_os = "linux")]
    linux::park_overlay();
}

/// Win/mac: GPUI activate. Linux: skip SetInputFocus (breaks overlay input).
pub fn focus_native_overlay(window: &mut Window) {
    #[cfg(not(target_os = "linux"))]
    window.activate_window();
    #[cfg(target_os = "linux")]
    {
        let _ = window;
    }
}

/// Linux fullscreen is `_NET_WM_STATE ADD` in `raise_overlay`. GPUI's
/// `WindowBounds::Fullscreen` is a TOGGLE and races with that ADD.
pub fn overlay_window_bounds(bounds: Bounds<Pixels>) -> WindowBounds {
    #[cfg(target_os = "linux")]
    {
        WindowBounds::Windowed(bounds)
    }
    #[cfg(not(target_os = "linux"))]
    {
        WindowBounds::Fullscreen(bounds)
    }
}
