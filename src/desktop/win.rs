//! Windows window-manager helpers (hide-before-WGC). Overlay lives in
//! `win_snip`; this file is the analogue of `linux.rs` hide/wait.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, ClipCursor, EnumWindows, GetWindowLongPtrW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, ShowCursor, ASFW_ANY, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

use crate::identity::APP_SLUG;

/// DWM still composites the main window after `IsIconic`. WGC will photograph
/// it unless we wait — same ~280ms trap as before the poll optimization.
const WGC_SETTLE: Duration = Duration::from_millis(280);
const ICONIFY_POLL: Duration = Duration::from_millis(8);
const ICONIFY_DEADLINE: Duration = Duration::from_millis(700);
const ICONIFY_FALLBACK: Duration = Duration::from_millis(280);

static OS_CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);

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
