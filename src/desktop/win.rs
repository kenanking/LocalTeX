//! Windows window-manager helpers (hide-before-WGC). Overlay lives in
//! `win_snip`; this file is the analogue of `linux.rs` hide/wait.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CallNextHookEx, ClipCursor, EnumWindows, GetWindowLongPtrW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetWindowsHookExW, ShowCursor, ShowWindow,
    ASFW_ANY, CWPSTRUCT, GWL_EXSTYLE, SW_HIDE, SW_RESTORE, WH_CALLWNDPROC, WM_ENDSESSION,
    WM_QUERYENDSESSION, WS_EX_TOOLWINDOW,
};

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

thread_local! {
    static TRAY_HIDDEN: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

fn is_session_end(message: u32) -> bool {
    message == WM_QUERYENDSESSION || message == WM_ENDSESSION
}

/// Restart Manager (Inno `CloseApplications`) sends `WM_QUERYENDSESSION`.
/// GPUI never quits on that, and close-to-tray eats `WM_CLOSE`, so Setup
/// freezes on "Closing applications..." until the 30s timeout.
pub(crate) fn install_session_end_hook(tx: Sender<DesktopCmd>) {
    match SESSION_TX.lock() {
        Ok(mut slot) => *slot = Some(tx),
        Err(poisoned) => *poisoned.into_inner() = Some(tx),
    }
    let mut hook = match SESSION_HOOK.lock() {
        Ok(h) => h,
        Err(poisoned) => poisoned.into_inner(),
    };
    if hook.is_some() {
        return;
    }
    match unsafe {
        SetWindowsHookExW(
            WH_CALLWNDPROC,
            Some(session_end_hook),
            None,
            GetCurrentThreadId(),
        )
    } {
        Ok(h) => {
            *hook = Some(h.0 as isize);
            eprintln!("{APP_SLUG}: windows session-end hook installed");
        }
        Err(err) => eprintln!("{APP_SLUG}: windows session-end hook: {err:#}"),
    }
}

unsafe extern "system" fn session_end_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
        if is_session_end(msg.message) {
            let slot = match SESSION_TX.lock() {
                Ok(slot) => slot,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(tx) = slot.as_ref() {
                let _ = tx.send(DesktopCmd::Quit);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// `SW_HIDE` removes the window from the taskbar. `SW_MINIMIZE` does not.
pub(crate) fn hide_main_window() {
    let hwnds = taskbar_main_windows();
    if hwnds.is_empty() {
        return;
    }
    let stored: Vec<isize> = hwnds.iter().map(|hwnd| hwnd.0 as isize).collect();
    for hwnd in hwnds {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    TRAY_HIDDEN.with(|slot| slot.replace(stored));
}

pub(crate) fn show_main_window() {
    let stored = TRAY_HIDDEN.with(|slot| slot.replace(Vec::new()));
    for raw in stored {
        let hwnd = HWND(raw as *mut core::ffi::c_void);
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
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
    use super::is_session_end;
    use windows::Win32::UI::WindowsAndMessaging::{WM_CLOSE, WM_ENDSESSION, WM_QUERYENDSESSION};

    #[test]
    fn session_end_is_query_or_end_not_close() {
        assert!(is_session_end(WM_QUERYENDSESSION));
        assert!(is_session_end(WM_ENDSESSION));
        assert!(!is_session_end(WM_CLOSE));
    }
}
