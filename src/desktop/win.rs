//! Windows window-manager helpers (hide-before-WGC). Overlay lives in
//! `win_snip`; this file is the analogue of `linux.rs` hide/wait.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetActiveWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, BringWindowToTop, ClipCursor, EnumWindows, GWL_EXSTYLE,
    GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SW_HIDE, SW_RESTORE, SetForegroundWindow, ShowCursor, ShowWindow, WM_ENDSESSION,
    WM_QUERYENDSESSION, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use super::DesktopCmd;
use crate::identity::APP_SLUG;

/// DWM still composites the main window after `IsIconic`. WGC will photograph
/// it unless we wait — same ~280ms trap as before the poll optimization.
const WGC_SETTLE: Duration = Duration::from_millis(280);
const ICONIFY_POLL: Duration = Duration::from_millis(8);
const ICONIFY_DEADLINE: Duration = Duration::from_millis(700);
const ICONIFY_FALLBACK: Duration = Duration::from_millis(280);

static OS_CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
static SESSION_HOOK: Mutex<Option<isize>> = Mutex::new(None);
static SESSION_TX: Mutex<Option<Sender<DesktopCmd>>> = Mutex::new(None);

static SESSION_WRITERS: Mutex<
    Option<(crate::store::StoreWriter, Option<crate::prefs::PrefsWriter>)>,
> = Mutex::new(None);
static SESSION_PENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_session_writers(
    writer: crate::store::StoreWriter,
    prefs: Option<crate::prefs::PrefsWriter>,
) {
    *SESSION_WRITERS.lock().expect("session writers") = Some((writer, prefs));
}

pub(crate) fn session_pending() -> bool {
    SESSION_PENDING.load(Ordering::Acquire)
}

pub(crate) fn install_session_end_hook(tx: Sender<DesktopCmd>) {
    *SESSION_TX.lock().expect("session sender") = Some(tx);
    attach_main_window();
}

pub(crate) fn attach_main_window() {
    use windows::Win32::UI::Shell::SetWindowSubclass;
    if let Some(hwnd) = taskbar_main_windows().first().copied() {
        if unsafe { SetWindowSubclass(hwnd, Some(session_proc), 0x4c54, 0) }.as_bool() {
            *SESSION_HOOK.lock().expect("main window") = Some(hwnd.0 as isize);
        } else {
            eprintln!("{APP_SLUG}: could not install session handler");
        }
    }
}

fn flush_session() -> bool {
    let deadline = Instant::now() + Duration::from_secs(4);
    let writers = SESSION_WRITERS.lock().expect("session writers").clone();
    let result = (|| -> anyhow::Result<()> {
        if let Some((writer, prefs)) = writers {
            writer.flush_timeout(deadline.saturating_duration_since(Instant::now()))?;
            if let Some(prefs) = prefs {
                prefs.flush_timeout(deadline.saturating_duration_since(Instant::now()))?;
            }
        }
        Ok(())
    })();
    if let Err(err) = result {
        eprintln!("{APP_SLUG}: session save: {err:#}");
        return false;
    }
    true
}

unsafe extern "system" fn session_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::WM_NCDESTROY;
    match message {
        WM_QUERYENDSESSION => {
            SESSION_PENDING.store(true, Ordering::Release);
            let saved = flush_session();
            if !saved {
                SESSION_PENDING.store(false, Ordering::Release);
            }
            return LRESULT(saved as isize);
        }
        WM_ENDSESSION => {
            if session_confirmed(message, wparam) {
                flush_session();
                if let Some(tx) = SESSION_TX.lock().expect("session sender").as_ref() {
                    let _ = tx.send(DesktopCmd::Quit);
                }
            }
            SESSION_PENDING.store(false, Ordering::Release);
            return LRESULT(0);
        }
        WM_NCDESTROY => {
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(session_proc), 0x4c54) };
            *SESSION_HOOK.lock().expect("main window") = None;
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn session_confirmed(message: u32, wparam: WPARAM) -> bool {
    message == WM_ENDSESSION && wparam.0 != 0
}

pub(crate) fn main_is_visible() -> bool {
    SESSION_HOOK
        .lock()
        .ok()
        .and_then(|slot| *slot)
        .is_some_and(|raw| {
            let hwnd = HWND(raw as *mut core::ffi::c_void);
            unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() }
        })
}
/// `SW_HIDE` removes the window from the taskbar. `SW_MINIMIZE` does not.
pub(crate) fn hide_main_window() {
    if let Some(raw) = *SESSION_HOOK.lock().expect("main window") {
        unsafe {
            let _ = ShowWindow(HWND(raw as *mut core::ffi::c_void), SW_HIDE);
        }
    }
}

pub(crate) fn show_main_window() {
    let raw = *SESSION_HOOK.lock().expect("main window");
    if let Some(raw) = raw {
        focus_hwnd(HWND(raw as *mut core::ffi::c_void));
    }
}
/// Restore and focus without GPUI's `activate()`, which SendInput's an Alt
/// pair and desyncs chord matching from physical modifier keys.
fn focus_hwnd(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        let fg = GetForegroundWindow();
        let fg_tid = GetWindowThreadProcessId(fg, None);
        let us = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != us && AttachThreadInput(fg_tid, us, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        if attached {
            let _ = AttachThreadInput(fg_tid, us, false);
        }
    }
}

/// Process-wide ShowCursor counter. Idempotent.
pub(crate) fn set_os_cursor_visible(visible: bool) {
    let hide = !visible;
    if OS_CURSOR_HIDDEN.swap(hide, Ordering::SeqCst) == hide {
        return;
    }
    unsafe {
        ShowCursor(visible);
    }
}

pub(crate) fn wait_until_main_iconified() {
    let started = Instant::now();
    let pending = visible_main_windows();
    if pending.is_empty() {
        std::thread::sleep(ICONIFY_FALLBACK);
        eprintln!(
            "{APP_SLUG}: win iconify fallback {}ms",
            started.elapsed().as_millis()
        );
        return;
    }
    let deadline = started + ICONIFY_DEADLINE;
    loop {
        // `IsWindowVisible` goes false during DWM's minimize animation while
        // the frame is still on screen — WGC would photograph LocalTeX.
        let gone = pending
            .iter()
            .all(|hwnd| unsafe { IsIconic(*hwnd).as_bool() });
        if gone {
            std::thread::sleep(WGC_SETTLE);
            eprintln!(
                "{APP_SLUG}: win iconify wait {}ms",
                started.elapsed().as_millis()
            );
            return;
        }
        if Instant::now() >= deadline {
            std::thread::sleep(WGC_SETTLE);
            eprintln!(
                "{APP_SLUG}: win iconify timeout {}ms",
                started.elapsed().as_millis()
            );
            return;
        }
        std::thread::sleep(ICONIFY_POLL);
    }
}

/// GPUI thread: unlock foreground for the overlay and drop leftover capture
/// from the Snip button. Do not call `AllowSetForegroundWindow` again on the
/// overlay thread.
pub(crate) fn prepare_snip_input() {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        reset_pointer_state();
    }
}

pub(crate) fn reset_pointer_state() {
    unsafe {
        let _ = ReleaseCapture();
        let _ = ClipCursor(None);
    }
}

unsafe extern "system" fn enum_taskbar_main(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid != unsafe { GetCurrentProcessId() } {
        return BOOL(1);
    }
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 {
        return BOOL(1);
    }
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1);
    }
    out.push(hwnd);
    BOOL(1)
}

fn taskbar_main_windows() -> Vec<HWND> {
    let mut out = Vec::new();
    if unsafe {
        EnumWindows(
            Some(enum_taskbar_main),
            LPARAM(&mut out as *mut Vec<HWND> as isize),
        )
    }
    .is_err()
    {
        return Vec::new();
    }
    out
}

unsafe extern "system" fn enum_visible_main(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid != unsafe { GetCurrentProcessId() } {
        return BOOL(1);
    }
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 {
        return BOOL(1);
    }
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1);
    }
    if unsafe { IsIconic(hwnd).as_bool() } {
        return BOOL(1);
    }
    out.push(hwnd);
    BOOL(1)
}

fn visible_main_windows() -> Vec<HWND> {
    let mut out = Vec::new();
    if unsafe {
        EnumWindows(
            Some(enum_visible_main),
            LPARAM(&mut out as *mut Vec<HWND> as isize),
        )
    }
    .is_err()
    {
        return Vec::new();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::session_confirmed;
    use windows::Win32::Foundation::WPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{WM_CLOSE, WM_ENDSESSION, WM_QUERYENDSESSION};

    #[test]
    fn only_confirmed_session_end_requests_quit() {
        assert!(!session_confirmed(WM_QUERYENDSESSION, WPARAM(1)));
        assert!(!session_confirmed(WM_ENDSESSION, WPARAM(0)));
        assert!(session_confirmed(WM_ENDSESSION, WPARAM(1)));
        assert!(!session_confirmed(WM_CLOSE, WPARAM(1)));
    }

    #[test]
    fn session_query_flushes_without_quitting_and_cancel_resumes() {
        use super::*;
        let root = std::env::temp_dir().join(format!("localtex-session-{}", uuid::Uuid::new_v4()));
        let store = std::sync::Arc::new(crate::store::Store::open(root.clone()).unwrap());
        let (writer, _) = crate::store::StoreWriter::start(store.clone()).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        *SESSION_TX.lock().unwrap() = Some(tx);
        set_session_writers(writer.clone(), None);
        let doc = crate::doc::Document::pending(std::sync::Arc::new(image::RgbaImage::new(2, 2)));
        writer.insert(doc).unwrap();
        let send = |msg, value| unsafe {
            session_proc(HWND::default(), msg, WPARAM(value), LPARAM(0), 0, 0)
        };
        assert_eq!(send(WM_QUERYENDSESSION, 0).0, 1);
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(session_pending());
        assert!(rx.try_recv().is_err());
        send(WM_ENDSESSION, 0);
        assert!(!session_pending());
        assert!(rx.try_recv().is_err());
        send(WM_ENDSESSION, 1);
        assert_eq!(rx.recv().unwrap(), DesktopCmd::Quit);
        let mut bad =
            crate::doc::Document::pending(std::sync::Arc::new(image::RgbaImage::new(2, 2)));
        let id = bad.id;
        bad.image = crate::doc::ImageSlot::Missing;
        writer.insert(bad).unwrap();
        assert_eq!(send(WM_QUERYENDSESSION, 0).0, 0);
        assert!(!session_pending());
        writer.delete(id).unwrap();
        writer.shutdown().unwrap();
        *SESSION_WRITERS.lock().unwrap() = None;
        *SESSION_TX.lock().unwrap() = None;
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
