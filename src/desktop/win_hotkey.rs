//! Windows global chords. `RegisterHotKey` does not see later L/M presses
//! while Ctrl+Alt stay down and the GPUI window is focused (those become
//! SYSKEY).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, RwLock};
use std::thread;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
    VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
    LLKHF_LOWER_IL_INJECTED, LLKHF_UP, MSG, SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL,
};

use super::DesktopCmd;
use crate::identity::APP_SLUG;

#[derive(Clone, Copy)]
struct WinChord {
    vk: u16,
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
    cmd: DesktopCmd,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LiveMods {
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
    altgr: bool,
}

#[derive(Default)]
struct PhysicalMods {
    left_ctrl: bool,
    right_ctrl: bool,
    left_alt: bool,
    right_alt: bool,
    left_shift: bool,
    right_shift: bool,
    left_win: bool,
    right_win: bool,
}

struct HookState {
    mods: PhysicalMods,
    eating: Vec<u16>,
}

struct Consider {
    eat: bool,
    cmd: Option<DesktopCmd>,
}

static WIN_CHORDS: RwLock<Vec<WinChord>> = RwLock::new(Vec::new());
static SUSPENDED: AtomicBool = AtomicBool::new(false);
static HOOK: Mutex<Option<isize>> = Mutex::new(None);
static HOOK_TX: Mutex<Option<Sender<DesktopCmd>>> = Mutex::new(None);
static HOOK_STATE: Mutex<HookState> = Mutex::new(HookState {
    mods: PhysicalMods {
        left_ctrl: false,
        right_ctrl: false,
        left_alt: false,
        right_alt: false,
        left_shift: false,
        right_shift: false,
        left_win: false,
        right_win: false,
    },
    eating: Vec::new(),
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

impl PhysicalMods {
    fn update(&mut self, vk: u16, down: bool) {
        match vk {
            v if v == VK_CONTROL.0 || v == VK_LCONTROL.0 => self.left_ctrl = down,
            v if v == VK_RCONTROL.0 => self.right_ctrl = down,
            v if v == VK_MENU.0 || v == VK_LMENU.0 => self.left_alt = down,
            v if v == VK_RMENU.0 => self.right_alt = down,
            v if v == VK_SHIFT.0 || v == VK_LSHIFT.0 => self.left_shift = down,
            v if v == VK_RSHIFT.0 => self.right_shift = down,
            v if v == VK_LWIN.0 => self.left_win = down,
            v if v == VK_RWIN.0 => self.right_win = down,
            _ => {}
        }
    }

    fn live(&self) -> LiveMods {
        let ctrl = self.left_ctrl || self.right_ctrl;
        LiveMods {
            ctrl,
            alt: self.left_alt || self.right_alt,
            shift: self.left_shift || self.right_shift,
            win: self.left_win || self.right_win,
            // Windows reports AltGr as left Ctrl + right Alt.
            altgr: self.right_alt && self.left_ctrl && !self.right_ctrl,
        }
    }
}

fn mods_match(grab: &WinChord, live: LiveMods) -> bool {
    !((grab.control && grab.alt) && live.altgr)
        && live.ctrl == grab.control
        && live.alt == grab.alt
        && live.shift == grab.shift
        && live.win == grab.platform
}

fn consider_event(
    vk: u16,
    is_up: bool,
    injected: bool,
    suspended: bool,
    grabs: &[WinChord],
    state: &mut HookState,
) -> Consider {
    if injected {
        return Consider {
            eat: false,
            cmd: None,
        };
    }
    state.mods.update(vk, !is_up);
    if is_up {
        state.eating.retain(|key| *key != vk);
        return Consider {
            eat: false,
            cmd: None,
        };
    }
    if suspended {
        return Consider {
            eat: false,
            cmd: None,
        };
    }
    if state.eating.contains(&vk) {
        return Consider {
            eat: true,
            cmd: None,
        };
    }
    let live = state.mods.live();
    let Some(grab) = grabs
        .iter()
        .copied()
        .find(|g| g.vk == vk && mods_match(g, live))
    else {
        return Consider {
            eat: false,
            cmd: None,
        };
    };
    state.eating.push(vk);
    Consider {
        eat: true,
        cmd: Some(grab.cmd),
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

pub(crate) fn set_suspended(suspended: bool) {
    SUSPENDED.store(suspended, Ordering::SeqCst);
}

pub(crate) fn install_chord_hook(tx: Sender<DesktopCmd>) -> Result<(), String> {
    match HOOK_TX.lock() {
        Ok(mut slot) => *slot = Some(tx),
        Err(poisoned) => *poisoned.into_inner() = Some(tx),
    }
    {
        let hook = match HOOK.lock() {
            Ok(h) => h,
            Err(poisoned) => poisoned.into_inner(),
        };
        if hook.is_some() {
            return Ok(());
        }
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    if let Err(err) = thread::Builder::new()
        .name("localtex-hotkey".into())
        .spawn(move || hook_thread_main(ready_tx))
    {
        return Err(format!("spawn chord hook thread: {err}"));
    }
    match ready_rx.recv() {
        Ok(result) => result,
        Err(err) => Err(format!("chord hook thread: {err}")),
    }
}

fn hook_thread_main(ready: Sender<Result<(), String>>) {
    let hmod = unsafe { GetModuleHandleW(None) }
        .ok()
        .map(|m| HINSTANCE(m.0));
    match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(chord_hook), hmod, 0) } {
        Ok(h) => {
            match HOOK.lock() {
                Ok(mut slot) => *slot = Some(h.0 as isize),
                Err(poisoned) => *poisoned.into_inner() = Some(h.0 as isize),
            }
            let _ = ready.send(Ok(()));
        }
        Err(err) => {
            let _ = ready.send(Err(format!("{err:#}")));
            return;
        }
    }
    let mut msg = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if status.0 == 0 {
            break;
        }
        if status.0 == -1 {
            eprintln!("{APP_SLUG}: windows chord hook message loop failed");
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
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
    let injected =
        info.flags.contains(LLKHF_INJECTED) || info.flags.contains(LLKHF_LOWER_IL_INJECTED);
    let grabs = match WIN_CHORDS.read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut state = match HOOK_STATE.lock() {
        Ok(s) => s,
        Err(poisoned) => poisoned.into_inner(),
    };
    let result = consider_event(
        vk,
        is_up,
        injected,
        SUSPENDED.load(Ordering::SeqCst),
        &grabs,
        &mut state,
    );
    if let Some(cmd) = result.cmd {
        let tx = match HOOK_TX.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(tx) = tx {
            let _ = tx.send(cmd);
        }
    }
    result.eat
}

#[cfg(test)]
mod tests {
    use super::{
        HookState, PhysicalMods, WIN_CHORDS, WinChord, consider_event, gpui_key_to_vk, set_chords,
    };
    use crate::desktop::DesktopCmd;

    const VK_L: u16 = 0x4C;
    const VK_M: u16 = 0x4D;
    const LCTRL: u16 = 0xA2;
    const RCTRL: u16 = 0xA3;
    const LSHIFT: u16 = 0xA0;
    const LALT: u16 = 0xA4;
    const RALT: u16 = 0xA5;

    fn show_l() -> WinChord {
        WinChord {
            vk: VK_L,
            control: true,
            alt: true,
            shift: false,
            platform: false,
            cmd: DesktopCmd::Show,
        }
    }

    fn capture_m() -> WinChord {
        WinChord {
            vk: VK_M,
            control: true,
            alt: true,
            shift: false,
            platform: false,
            cmd: DesktopCmd::Capture,
        }
    }

    fn capture_shift_l() -> WinChord {
        WinChord {
            vk: VK_L,
            control: true,
            alt: false,
            shift: true,
            platform: false,
            cmd: DesktopCmd::Capture,
        }
    }

    fn state() -> HookState {
        HookState {
            mods: PhysicalMods::default(),
            eating: Vec::new(),
        }
    }

    #[test]
    fn overlapping_chords_each_fire_once_until_their_own_keyup() {
        let mut state = state();
        send(&mut state, LCTRL, false, false);
        send(&mut state, LALT, false, false);
        assert_eq!(
            send(&mut state, VK_L, false, false).cmd,
            Some(DesktopCmd::Show)
        );
        assert_eq!(
            send(&mut state, VK_M, false, false).cmd,
            Some(DesktopCmd::Capture)
        );
        assert_eq!(send(&mut state, VK_L, false, false).cmd, None);
        send(&mut state, VK_M, true, false);
        assert_eq!(send(&mut state, VK_L, false, false).cmd, None);
        send(&mut state, VK_L, true, false);
        assert_eq!(
            send(&mut state, VK_L, false, false).cmd,
            Some(DesktopCmd::Show)
        );
    }

    fn send(state: &mut HookState, vk: u16, is_up: bool, injected: bool) -> super::Consider {
        consider_event(vk, is_up, injected, false, &[show_l(), capture_m()], state)
    }

    #[test]
    fn letters_map_to_virtual_keys() {
        assert_eq!(gpui_key_to_vk("l"), Some(VK_L));
        assert_eq!(gpui_key_to_vk("m"), Some(VK_M));
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
        assert!(
            grabs
                .iter()
                .any(|g| g.vk == VK_L && g.cmd == DesktopCmd::Show)
        );
        assert!(
            grabs
                .iter()
                .any(|g| g.vk == VK_M && g.cmd == DesktopCmd::Capture)
        );
    }

    #[test]
    fn bare_l_does_not_fire_or_eat() {
        let mut state = state();
        let r = send(&mut state, VK_L, false, false);
        assert!(r.cmd.is_none());
        assert!(!r.eat);
    }

    #[test]
    fn physical_ctrl_alt_l_fires_once_until_keyup() {
        let mut state = state();
        send(&mut state, LCTRL, false, false);
        send(&mut state, LALT, false, false);
        let down = send(&mut state, VK_L, false, false);
        assert_eq!(down.cmd, Some(DesktopCmd::Show));
        assert!(down.eat);
        let repeat = send(&mut state, VK_L, false, false);
        assert!(repeat.cmd.is_none());
        assert!(repeat.eat);
        let up = send(&mut state, VK_L, true, false);
        assert!(up.cmd.is_none());
        assert!(!up.eat);
        let again = send(&mut state, VK_L, false, false);
        assert_eq!(again.cmd, Some(DesktopCmd::Show));
        assert!(again.eat);
    }

    #[test]
    fn injected_alt_pair_then_bare_l_does_not_fire() {
        let mut state = state();
        send(&mut state, LCTRL, false, false);
        send(&mut state, LALT, false, false);
        let first = send(&mut state, VK_L, false, false);
        assert_eq!(first.cmd, Some(DesktopCmd::Show));
        send(&mut state, VK_L, true, false);
        send(&mut state, LCTRL, true, false);
        send(&mut state, LALT, true, false);
        let inj_down = send(&mut state, LALT, false, true);
        assert!(inj_down.cmd.is_none());
        let inj_up = send(&mut state, LALT, true, true);
        assert!(inj_up.cmd.is_none());
        let bare = send(&mut state, VK_L, false, false);
        assert!(bare.cmd.is_none());
        assert!(!bare.eat);
    }

    #[test]
    fn injected_l_does_not_fire_even_with_mods_down() {
        let mut state = state();
        send(&mut state, LCTRL, false, false);
        send(&mut state, LALT, false, false);
        let r = send(&mut state, VK_L, false, true);
        assert!(r.cmd.is_none());
        assert!(!r.eat);
    }

    #[test]
    fn same_key_with_different_modifiers_finds_the_full_chord() {
        let grabs = [show_l(), capture_shift_l()];
        let mut state = state();
        consider_event(LCTRL, false, false, false, &grabs, &mut state);
        consider_event(LSHIFT, false, false, false, &grabs, &mut state);
        let result = consider_event(VK_L, false, false, false, &grabs, &mut state);
        assert_eq!(result.cmd, Some(DesktopCmd::Capture));
    }

    #[test]
    fn suspended_hook_tracks_modifiers_without_eating_the_chord() {
        let grabs = [show_l()];
        let mut state = state();
        consider_event(LCTRL, false, false, true, &grabs, &mut state);
        consider_event(LALT, false, false, true, &grabs, &mut state);
        let recording = consider_event(VK_L, false, false, true, &grabs, &mut state);
        assert!(recording.cmd.is_none());
        assert!(!recording.eat);
        consider_event(VK_L, true, false, true, &grabs, &mut state);
        let resumed = consider_event(VK_L, false, false, false, &grabs, &mut state);
        assert_eq!(resumed.cmd, Some(DesktopCmd::Show));
    }

    #[test]
    fn altgr_does_not_match_ctrl_alt_chord() {
        let mut state = state();
        send(&mut state, LCTRL, false, false);
        send(&mut state, RALT, false, false);
        let result = send(&mut state, VK_L, false, false);
        assert!(result.cmd.is_none());
        assert!(!result.eat);
    }

    #[test]
    fn releasing_one_control_keeps_the_other_control_down() {
        let mut state = state();
        send(&mut state, LCTRL, false, false);
        send(&mut state, RCTRL, false, false);
        send(&mut state, LCTRL, true, false);
        send(&mut state, LALT, false, false);
        let result = send(&mut state, VK_L, false, false);
        assert_eq!(result.cmd, Some(DesktopCmd::Show));
    }
}
