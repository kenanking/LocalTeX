use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::RwLock;
#[cfg(not(target_os = "windows"))]
use std::thread;

use global_hotkey::hotkey::HotKey;
use global_hotkey::GlobalHotKeyManager;
#[cfg(not(target_os = "windows"))]
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};

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
mod win_hotkey;
#[cfg(target_os = "windows")]
mod win_snip;
#[cfg(target_os = "linux")]
mod x11_snip;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopCmd {
    Capture,
    Show,
    Reveal,
    Quit,
}

impl From<crate::keymap::GlobalCmd> for DesktopCmd {
    fn from(cmd: crate::keymap::GlobalCmd) -> Self {
        match cmd {
            crate::keymap::GlobalCmd::Capture => Self::Capture,
            crate::keymap::GlobalCmd::Show => Self::Show,
        }
    }
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
struct Grab {
    chord: String,
    hotkey: HotKey,
    cmd: DesktopCmd,
}

struct GrabSet {
    by_chord: Vec<Grab>,
}

/// Keeps OS handles alive on the GPUI UI thread (Windows needs a win32 loop
/// on that thread; macOS needs the main thread).
struct Services {
    hotkey: Option<GlobalHotKeyManager>,
    grabs: GrabSet,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    _tray: Option<tray_icon::TrayIcon>,
}

thread_local! {
    static SERVICES: RefCell<Option<Services>> = const { RefCell::new(None) };
}

static GRABS: RwLock<Vec<(u32, DesktopCmd)>> = RwLock::new(Vec::new());

/// Register hotkey + tray. Must be called from the GPUI UI thread.
pub fn spawn() -> (Sender<DesktopCmd>, Receiver<DesktopCmd>) {
    let (tx, rx) = mpsc::channel();
    let hotkey = start_hotkey_manager(tx.clone());
    #[cfg(target_os = "windows")]
    win::install_session_end_hook(tx.clone());
    #[cfg(target_os = "linux")]
    linux::start_tray(tx.clone());
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let tray = other::start_tray(tx.clone());
    SERVICES.with(|slot| {
        *slot.borrow_mut() = Some(Services {
            hotkey,
            grabs: GrabSet {
                by_chord: Vec::new(),
            },
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            _tray: tray,
        });
    });
    (tx, rx)
}

/// Project catalog OS rows through Overrides onto the OS. Call on the GPUI UI thread.
/// `None` effective chord ⇒ that id is not grabbed. On register failure, previous grabs remain.
pub fn rebind_globals(over: &crate::keymap::Overrides) {
    SERVICES.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(svc) = slot.as_mut() else {
            return;
        };
        let Some(manager) = svc.hotkey.as_ref() else {
            return;
        };

        let desired: Vec<(String, DesktopCmd)> = crate::keymap::global_bindings(over)
            .into_iter()
            .filter_map(|(_, chord, cmd)| {
                if crate::keymap::to_global_hotkey(&chord).is_none() {
                    eprintln!("{APP_SLUG}: chord {chord:?} is not a global hotkey");
                    return None;
                }
                Some((chord, DesktopCmd::from(cmd)))
            })
            .collect();

        #[cfg(target_os = "windows")]
        {
            win_hotkey::set_chords(&desired);
            svc.grabs.by_chord.clear();
            match GRABS.write() {
                Ok(mut g) => g.clear(),
                Err(poisoned) => poisoned.into_inner().clear(),
            }
            let _ = manager;
        }

        #[cfg(not(target_os = "windows"))]
        {
            let mut added: Vec<Grab> = Vec::new();
            for (chord, cmd) in &desired {
                if svc.grabs.by_chord.iter().any(|g| &g.chord == chord) {
                    continue;
                }
                let Some(hotkey) = crate::keymap::to_global_hotkey(chord) else {
                    continue;
                };
                if let Err(err) = manager.register(hotkey) {
                    eprintln!("{APP_SLUG}: register global hotkey {chord}: {err}");
                    for g in &added {
                        if let Err(err) = manager.unregister(g.hotkey) {
                            eprintln!("{APP_SLUG}: unregister global hotkey: {err}");
                        }
                    }
                    return;
                }
                eprintln!("{APP_SLUG}: global hotkey {chord} registered");
                added.push(Grab {
                    chord: chord.clone(),
                    hotkey,
                    cmd: *cmd,
                });
            }

            let mut next = Vec::new();
            for grab in svc.grabs.by_chord.drain(..) {
                if let Some((_, cmd)) = desired.iter().find(|(c, _)| c == &grab.chord) {
                    next.push(Grab {
                        chord: grab.chord,
                        hotkey: grab.hotkey,
                        cmd: *cmd,
                    });
                } else if let Err(err) = manager.unregister(grab.hotkey) {
                    eprintln!("{APP_SLUG}: unregister global hotkey: {err}");
                }
            }
            next.extend(added);
            svc.grabs.by_chord = next;

            let published: Vec<(u32, DesktopCmd)> = svc
                .grabs
                .by_chord
                .iter()
                .map(|g| (g.hotkey.id(), g.cmd))
                .collect();
            match GRABS.write() {
                Ok(mut g) => *g = published,
                Err(poisoned) => {
                    *poisoned.into_inner() = published;
                }
            }
        }
    });
}

fn start_hotkey_manager(tx: Sender<DesktopCmd>) -> Option<GlobalHotKeyManager> {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(err) => {
            eprintln!("{APP_SLUG}: global hotkey manager failed: {err}");
            return None;
        }
    };
    #[cfg(target_os = "windows")]
    {
        // RegisterHotKey + MOD_NOREPEAT stops posting WM_HOTKEY once the
        // main window is focused with Alt still down (L becomes SYSKEY).
        win_hotkey::install_chord_hook(tx);
        return Some(manager);
    }
    #[cfg(not(target_os = "windows"))]
    {
        thread::spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();
            while let Ok(event) = receiver.recv() {
                if event.state != HotKeyState::Pressed {
                    continue;
                }
                let cmd = match GRABS.read() {
                    Ok(g) => g
                        .iter()
                        .find(|(id, _)| *id == event.id)
                        .map(|(_, cmd)| *cmd),
                    Err(poisoned) => poisoned
                        .into_inner()
                        .iter()
                        .find(|(id, _)| *id == event.id)
                        .map(|(_, cmd)| *cmd),
                };
                let Some(cmd) = cmd else {
                    continue;
                };
                if let Err(err) = tx.send(cmd) {
                    eprintln!("{APP_SLUG}: hotkey send: {err}");
                    break;
                }
            }
        });
        Some(manager)
    }
}

#[cfg(target_os = "windows")]
pub fn read_clipboard_image() -> Vec<Vec<u8>> {
    win_clipboard::read()
}

#[cfg(any(test, target_os = "windows"))]
pub fn decode_clipboard_image(bytes: Vec<u8>) -> anyhow::Result<image::RgbaImage> {
    match image::load_from_memory(&bytes) {
        Ok(img) => Ok(img.to_rgba8()),
        Err(_) => crate::imgutil::decode_dib(&bytes),
    }
}

/// Write PNG bytes to the system clipboard so other apps can paste the image.
///
/// GPUI 0.2's X11 `write_to_clipboard` only calls `set_text`.
pub fn write_clipboard_png(png: Vec<u8>, cx: &mut gpui::App) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let _ = cx;
        linux::write_clipboard_png(png)
    }
    #[cfg(not(target_os = "linux"))]
    {
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            png,
        )));
        Ok(())
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

/// Close-to-tray / tray toggle. Windows uses `SW_HIDE` so the taskbar button
/// goes away; capture hide still uses [`iconify_main_window`] plus minimize.
pub fn hide_main_to_tray() {
    #[cfg(target_os = "linux")]
    linux::iconify_main_window();
    #[cfg(target_os = "windows")]
    win::hide_main_window();
}

pub fn deiconify_main_window() {
    #[cfg(target_os = "linux")]
    linux::deiconify_main_window();
    #[cfg(target_os = "windows")]
    win::show_main_window();
}

pub fn window_open_focus(activate: bool) -> bool {
    activate && cfg!(not(target_os = "windows"))
}

pub fn focus_new_main<V: 'static>(
    handle: gpui::WindowHandle<V>,
    cx: &mut gpui::App,
) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let _ = (handle, cx);
        deiconify_main_window();
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        handle.update(cx, |_, window, _| window.activate_window())?;
        Ok(())
    }
}

pub fn gpui_activate_main<V: 'static>(handle: gpui::WindowHandle<V>, cx: &mut gpui::App) {
    #[cfg(not(target_os = "windows"))]
    if let Err(err) = handle.update(cx, |_, window, _| {
        window.activate_window();
    }) {
        eprintln!("{APP_SLUG}: activate window: {err}");
    }
    #[cfg(target_os = "windows")]
    {
        let _ = (handle, cx);
    }
}

pub fn wait_until_iconified() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    linux::wait_until_iconified()?;
    #[cfg(target_os = "windows")]
    {
        win::wait_until_main_iconified();
    }
    Ok(())
}

/// Call from the GPUI thread before a snip so the overlay can become
/// foreground and any leftover mouse capture is dropped.
pub fn prepare_snip_input() {
    #[cfg(target_os = "windows")]
    win::prepare_snip_input();
}

/// Hide or show the OS pointer over the GPUI window. Used by the draw-board
/// eraser so the painted ring is the only cursor. Idempotent.
pub fn set_os_cursor_visible(visible: bool) {
    #[cfg(target_os = "linux")]
    linux::set_os_cursor_visible(visible);
    #[cfg(target_os = "windows")]
    win::set_os_cursor_visible(visible);
}

/// Re-apply the blank X cursor after GPUI's `reset_cursor_style` (Arrow).
pub fn reassert_hidden_os_cursor() {
    #[cfg(target_os = "linux")]
    linux::reassert_hidden_os_cursor();
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
