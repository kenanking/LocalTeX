//! Windows global chords. `RegisterHotKey` does not see later L/M presses
//! while Ctrl+Alt stay down and the GPUI window is focused (those become
//! SYSKEY).

use std::sync::mpsc::Sender;
use std::sync::{Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_LOWER_IL_INJECTED, LLKHF_UP, MSG, WH_KEYBOARD_LL,
};

use super::DesktopCmd;
use crate::identity::APP_SLUG;

const REPEAT_GAP: Duration = Duration::from_millis(40);

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
}

struct HookState {
    last_fire: Option<(u16, Instant)>,
    eating: Option<u16>,
}

struct Consider {
    eat: bool,
    cmd: Option<DesktopCmd>,
}

static WIN_CHORDS: RwLock<Vec<WinChord>> = RwLock::new(Vec::new());
static HOOK: Mutex<Option<isize>> = Mutex::new(None);
static HOOK_TX: Mutex<Option<Sender<DesktopCmd>>> = Mutex::new(None);
static HOOK_STATE: Mutex<HookState> = Mutex::new(HookState {
    last_fire: None,
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

fn async_down(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(i32::from(vk.0)) as u16 & 0x8000 != 0 }
}

fn query_live_mods() -> LiveMods {
    LiveMods {
        ctrl: async_down(VK_CONTROL),
        alt: async_down(VK_MENU),
        shift: async_down(VK_SHIFT),
        win: async_down(VK_LWIN) || async_down(VK_RWIN),
    }
}

fn mods_match(grab: &WinChord, live: LiveMods) -> bool {
    live.ctrl == grab.control
        && live.alt == grab.alt
        && live.shift == grab.shift
        && live.win == grab.platform
}

fn should_emit_keydown(last: Option<(u16, Instant)>, vk: u16, now: Instant) -> bool {
    match last {
        Some((prev, at)) if prev == vk && now.saturating_duration_since(at) < REPEAT_GAP => false,
        _ => true,
    }
}

fn consider_event(
    vk: u16,
    is_up: bool,
    injected: bool,
    live: LiveMods,
    grabs: &[WinChord],
    state: &mut HookState,
    now: Instant,
) -> Consider {
    if is_up {
        if state.eating == Some(vk) {
            state.eating = None;
        }
        return Consider {
            eat: false,
            cmd: None,
        };
    }
    if injected {
        return Consider {
            eat: false,
            cmd: None,
        };
    }
    let Some(grab) = grabs.iter().copied().find(|g| g.vk == vk) else {
        return Consider {
            eat: false,
            cmd: None,
        };
    };
    if !mods_match(&grab, live) {
        state.eating = None;
        return Consider {
            eat: false,
            cmd: None,
        };
    }
    let cmd = if should_emit_keydown(state.last_fire, vk, now) {
        state.last_fire = Some((vk, now));
        Some(grab.cmd)
    } else {
        None
    };
    state.eating = Some(vk);
    Consider { eat: true, cmd }
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
    {
        let hook = match HOOK.lock() {
            Ok(h) => h,
            Err(poisoned) => poisoned.into_inner(),
        };
        if hook.is_some() {
            return;
        }
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    if let Err(err) = thread::Builder::new()
        .name("localtex-hotkey".into())
        .spawn(move || hook_thread_main(ready_tx))
    {
        eprintln!("{APP_SLUG}: windows chord hook thread: {err}");
        return;
    }
    match ready_rx.recv() {
        Ok(Ok(())) => eprintln!("{APP_SLUG}: windows chord hook installed"),
        Ok(Err(err)) => eprintln!("{APP_SLUG}: windows chord hook: {err}"),
        Err(err) => eprintln!("{APP_SLUG}: windows chord hook thread: {err}"),
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
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
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
    let live = query_live_mods();
    let grabs = match WIN_CHORDS.read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut state = match HOOK_STATE.lock() {
        Ok(s) => s,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    let result = consider_event(vk, is_up, injected, live, &grabs, &mut state, now);
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
        consider_event, gpui_key_to_vk, mods_match, set_chords, should_emit_keydown, Consider,
        HookState, LiveMods, WinChord, REPEAT_GAP, WIN_CHORDS,
    };
    use crate::desktop::DesktopCmd;
    use std::time::{Duration, Instant};

    const VK_L: u16 = 0x4C;
    const VK_M: u16 = 0x4D;
    const VK_LMENU: u16 = 0xA4;

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

    fn held() -> LiveMods {
        LiveMods {
            ctrl: true,
            alt: true,
            shift: false,
            win: false,
        }
    }

    fn none_held() -> LiveMods {
        LiveMods::default()
    }

    fn run(
        state: &mut HookState,
        vk: u16,
        is_up: bool,
        injected: bool,
        live: LiveMods,
        now: Instant,
    ) -> Consider {
        let grabs = [show_l(), capture_m()];
        consider_event(vk, is_up, injected, live, &grabs, state, now)
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
        assert!(grabs
            .iter()
            .any(|g| g.vk == VK_L && g.cmd == DesktopCmd::Show));
        assert!(grabs
            .iter()
            .any(|g| g.vk == VK_M && g.cmd == DesktopCmd::Capture));
    }

    #[test]
    fn ctrl_alt_l_matches_only_those_modifiers() {
        let grab = show_l();
        assert!(mods_match(&grab, held()));
        assert!(!mods_match(&grab, none_held()));
        assert!(!mods_match(
            &grab,
            LiveMods {
                ctrl: true,
                alt: false,
                shift: false,
                win: false,
            }
        ));
        assert!(!mods_match(
            &grab,
            LiveMods {
                ctrl: true,
                alt: true,
                shift: true,
                win: false,
            }
        ));
    }

    #[test]
    fn bare_l_does_not_fire_or_eat() {
        let mut state = HookState {
            last_fire: None,
            eating: None,
        };
        let r = run(&mut state, VK_L, false, false, none_held(), Instant::now());
        assert!(r.cmd.is_none());
        assert!(!r.eat);
    }

    #[test]
    fn held_ctrl_alt_l_fires_and_repeat_after_gap() {
        let mut state = HookState {
            last_fire: None,
            eating: None,
        };
        let t0 = Instant::now();
        let down = run(&mut state, VK_L, false, false, held(), t0);
        assert_eq!(down.cmd, Some(DesktopCmd::Show));
        assert!(down.eat);
        let up = run(
            &mut state,
            VK_L,
            true,
            false,
            held(),
            t0 + Duration::from_millis(10),
        );
        assert!(up.cmd.is_none());
        assert!(!up.eat);
        let again = run(
            &mut state,
            VK_L,
            false,
            false,
            held(),
            t0 + REPEAT_GAP + Duration::from_millis(1),
        );
        assert_eq!(again.cmd, Some(DesktopCmd::Show));
        assert!(again.eat);
    }

    #[test]
    fn injected_alt_pair_then_bare_l_does_not_fire() {
        let mut state = HookState {
            last_fire: None,
            eating: None,
        };
        let t0 = Instant::now();
        let first = run(&mut state, VK_L, false, false, held(), t0);
        assert_eq!(first.cmd, Some(DesktopCmd::Show));
        let _ = run(
            &mut state,
            VK_L,
            true,
            false,
            held(),
            t0 + Duration::from_millis(10),
        );
        let inj_down = run(
            &mut state,
            VK_LMENU,
            false,
            true,
            none_held(),
            t0 + Duration::from_millis(20),
        );
        assert!(inj_down.cmd.is_none());
        let inj_up = run(
            &mut state,
            VK_LMENU,
            true,
            true,
            none_held(),
            t0 + Duration::from_millis(21),
        );
        assert!(inj_up.cmd.is_none());
        let bare = run(
            &mut state,
            VK_L,
            false,
            false,
            none_held(),
            t0 + Duration::from_millis(80),
        );
        assert!(bare.cmd.is_none());
        assert!(!bare.eat);
    }

    #[test]
    fn injected_l_does_not_fire_even_with_mods_down() {
        let mut state = HookState {
            last_fire: None,
            eating: None,
        };
        let r = run(&mut state, VK_L, false, true, held(), Instant::now());
        assert!(r.cmd.is_none());
        assert!(!r.eat);
    }

    #[test]
    fn keydown_emits_again_after_repeat_gap() {
        let t0 = Instant::now();
        assert!(should_emit_keydown(None, VK_L, t0));
        assert!(!should_emit_keydown(
            Some((VK_L, t0)),
            VK_L,
            t0 + Duration::from_millis(20)
        ));
        assert!(should_emit_keydown(
            Some((VK_L, t0)),
            VK_L,
            t0 + REPEAT_GAP + Duration::from_millis(1)
        ));
    }
}
