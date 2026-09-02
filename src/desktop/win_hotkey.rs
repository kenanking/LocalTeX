//! Windows global chords. `RegisterHotKey` does not see later L/M presses
//! while Ctrl+Alt stay down and the GPUI window is focused (those become
//! SYSKEY). WeChat-style repeat uses a low-level hook and fires on each
//! keydown after a short anti-repeat gap.

use std::sync::mpsc::Sender;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
    VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, LLKHF_INJECTED, LLKHF_UP,
    WH_KEYBOARD_LL,
};

use super::DesktopCmd;
use crate::identity::APP_SLUG;

const REPEAT_GAP: Duration = Duration::from_millis(40);
const SYNTHETIC_MOD_UP: Duration = Duration::from_millis(20);

#[derive(Clone, Copy)]
struct WinChord {
    vk: u16,
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
    cmd: DesktopCmd,
}

struct HookState {
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
    last_fire: Option<(u16, Instant)>,
    last_inj_down: Option<(u8, Instant)>,
    eating: Option<u16>,
}

static WIN_CHORDS: RwLock<Vec<WinChord>> = RwLock::new(Vec::new());
static HOOK: Mutex<Option<isize>> = Mutex::new(None);
static HOOK_TX: Mutex<Option<Sender<DesktopCmd>>> = Mutex::new(None);
static HOOK_STATE: Mutex<HookState> = Mutex::new(HookState {
    ctrl: false,
    alt: false,
    shift: false,
    win: false,
    last_fire: None,
    last_inj_down: None,
    eating: None,
});

fn gpui_key_to_vk(key: &str) -> Option<u16> {
    if key.len() == 1 {
        let c = key.chars().next()?.to_ascii_uppercase();
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u16);
        }
    }
    Some(match key {
        "," => 0xBC,
        "." => 0xBE,
        "/" => 0xBF,
        ";" => 0xBA,
        "'" => 0xDE,
        "[" => 0xDB,
        "]" => 0xDD,
        "\\" => 0xDC,
        "-" => 0xBD,
        "=" => 0xBB,
        "`" => 0xC0,
        "delete" => 0x2E,
        "backspace" => 0x08,
        "enter" => 0x0D,
        "tab" => 0x09,
        "space" => 0x20,
        "escape" => 0x1B,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        _ => return None,
    })
}

fn apply_modifier(vk: u16, down: bool, state: &mut HookState) {
    match vk {
        v if v == VK_CONTROL.0 || v == VK_LCONTROL.0 || v == VK_RCONTROL.0 => state.ctrl = down,
        v if v == VK_MENU.0 || v == VK_LMENU.0 || v == VK_RMENU.0 => state.alt = down,
        v if v == VK_SHIFT.0 || v == VK_LSHIFT.0 || v == VK_RSHIFT.0 => state.shift = down,
        v if v == VK_LWIN.0 || v == VK_RWIN.0 => state.win = down,
        _ => {}
    }
}

fn modifier_kind(vk: u16) -> Option<u8> {
    match vk {
        v if v == VK_CONTROL.0 || v == VK_LCONTROL.0 || v == VK_RCONTROL.0 => Some(1),
        v if v == VK_MENU.0 || v == VK_LMENU.0 || v == VK_RMENU.0 => Some(2),
        v if v == VK_SHIFT.0 || v == VK_LSHIFT.0 || v == VK_RSHIFT.0 => Some(3),
        v if v == VK_LWIN.0 || v == VK_RWIN.0 => Some(4),
        _ => None,
    }
}

/// Restoring a focused window synthesizes an injected Alt down+up pair (~1ms)
/// so the menu bar does not stick. Ignore that pair or Ctrl+Alt+L dies after
/// the first show while the user is still holding Alt. Honor a later injected
/// up (SendInput release) so the latch does not outlive the chord.
fn apply_modifier_event(vk: u16, is_up: bool, injected: bool, now: Instant, state: &mut HookState) {
    if injected && !is_up {
        if let Some(kind) = modifier_kind(vk) {
            state.last_inj_down = Some((kind, now));
        }
        apply_modifier(vk, true, state);
        return;
    }
    if injected && is_up {
        let synthetic = matches!(
            (modifier_kind(vk), state.last_inj_down),
            (Some(kind), Some((prev, at)))
                if prev == kind && now.saturating_duration_since(at) < SYNTHETIC_MOD_UP
        );
        if synthetic {
            return;
        }
        apply_modifier(vk, false, state);
        return;
    }
    apply_modifier(vk, !is_up, state);
}

fn should_emit_keydown(last: Option<(u16, Instant)>, vk: u16, now: Instant) -> bool {
    match last {
        Some((prev, at)) if prev == vk && now.saturating_duration_since(at) < REPEAT_GAP => false,
        _ => true,
    }
}

pub(crate) fn set_chords(desired: &[(String, DesktopCmd)]) {
    let grabs: Vec<WinChord> = desired
        .iter()
        .filter_map(|(chord, cmd)| {
            let parts = crate::keymap::chord_parts(chord)?;
            let vk = gpui_key_to_vk(&parts.key)?;
            Some(WinChord {
                vk,
                control: parts.control,
                alt: parts.alt,
                shift: parts.shift,
                platform: parts.platform,
                cmd: *cmd,
            })
        })
        .collect();
    match WIN_CHORDS.write() {
        Ok(mut slot) => *slot = grabs,
        Err(poisoned) => *poisoned.into_inner() = grabs,
    }
}

pub(crate) fn install_chord_hook(tx: Sender<DesktopCmd>) {
    match HOOK_TX.lock() {
        Ok(mut slot) => *slot = Some(tx),
        Err(poisoned) => *poisoned.into_inner() = Some(tx),
    }
    let mut hook = match HOOK.lock() {
        Ok(h) => h,
        Err(poisoned) => poisoned.into_inner(),
    };
    if hook.is_some() {
        return;
    }
    let hmod = unsafe { GetModuleHandleW(None) }
        .ok()
        .map(|m| HINSTANCE(m.0));
    match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(chord_hook), hmod, 0) } {
        Ok(h) => {
            *hook = Some(h.0 as isize);
            eprintln!("{APP_SLUG}: windows chord hook installed");
        }
        Err(err) => eprintln!("{APP_SLUG}: windows chord hook: {err:#}"),
    }
}

unsafe extern "system" fn chord_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && dispatch_chord(lparam) {
        return LRESULT(1);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn dispatch_chord(lparam: LPARAM) -> bool {
    let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let vk = info.vkCode as u16;
    let is_up = info.flags.contains(LLKHF_UP);
    let injected = info.flags.contains(LLKHF_INJECTED);
    let mut state = match HOOK_STATE.lock() {
        Ok(s) => s,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    apply_modifier_event(vk, is_up, injected, now, &mut state);
    if !is_up && info.flags.contains(LLKHF_ALTDOWN) {
        state.alt = true;
    }

    let grabs = match WIN_CHORDS.read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(grab) = grabs.iter().copied().find(|g| g.vk == vk) else {
        return false;
    };
    drop(grabs);

    let mods_ok = state.ctrl == grab.control
        && state.alt == grab.alt
        && state.shift == grab.shift
        && state.win == grab.platform;

    if is_up {
        // Swallowing key-up leaves the OS key state stuck down, so later taps
        // never generate another key-down until modifiers are released.
        let _ = state.eating.take();
        return false;
    }
    if !mods_ok {
        state.eating = None;
        return false;
    }
    if should_emit_keydown(state.last_fire, vk, now) {
        state.last_fire = Some((vk, now));
        let tx = match HOOK_TX.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(tx) = tx {
            let _ = tx.send(grab.cmd);
        }
    }
    state.eating = Some(vk);
    true
}

#[cfg(test)]
mod tests {
    use super::{
        apply_modifier_event, gpui_key_to_vk, set_chords, should_emit_keydown, HookState,
        REPEAT_GAP, SYNTHETIC_MOD_UP, WIN_CHORDS,
    };
    use crate::desktop::DesktopCmd;
    use std::time::{Duration, Instant};

    #[test]
    fn letters_map_to_virtual_keys() {
        assert_eq!(gpui_key_to_vk("l"), Some(0x4C));
        assert_eq!(gpui_key_to_vk("m"), Some(0x4D));
        assert_eq!(gpui_key_to_vk(","), Some(0xBC));
    }

    #[test]
    fn set_chords_keeps_ctrl_alt_l_and_m() {
        set_chords(&[
            ("ctrl-alt-l".into(), DesktopCmd::Show),
            ("ctrl-alt-m".into(), DesktopCmd::Capture),
        ]);
        let grabs = WIN_CHORDS.read().unwrap();
        assert_eq!(grabs.len(), 2);
        assert!(grabs
            .iter()
            .any(|g| g.vk == 0x4C && g.cmd == DesktopCmd::Show));
        assert!(grabs
            .iter()
            .any(|g| g.vk == 0x4D && g.cmd == DesktopCmd::Capture));
    }

    #[test]
    fn injected_modifier_up_does_not_clear_held_alt() {
        let t0 = Instant::now();
        let mut state = HookState {
            ctrl: true,
            alt: true,
            shift: false,
            win: false,
            last_fire: None,
            last_inj_down: None,
            eating: None,
        };
        apply_modifier_event(0xA4, false, true, t0, &mut state);
        apply_modifier_event(0xA4, true, true, t0 + Duration::from_millis(1), &mut state);
        assert!(state.alt);
        apply_modifier_event(
            0xA4,
            true,
            true,
            t0 + SYNTHETIC_MOD_UP + Duration::from_millis(1),
            &mut state,
        );
        assert!(!state.alt);
    }

    #[test]
    fn keydown_emits_again_after_repeat_gap() {
        let t0 = Instant::now();
        assert!(should_emit_keydown(None, 0x4C, t0));
        assert!(!should_emit_keydown(
            Some((0x4C, t0)),
            0x4C,
            t0 + Duration::from_millis(20)
        ));
        assert!(should_emit_keydown(
            Some((0x4C, t0)),
            0x4C,
            t0 + REPEAT_GAP + Duration::from_millis(1)
        ));
    }
}
