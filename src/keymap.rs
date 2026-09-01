use std::collections::BTreeMap;
use std::str::FromStr;

use global_hotkey::hotkey::{Code, HotKey, Modifiers as HotMods};
use gpui::{App, KeyBinding, Keystroke};
use serde::{Deserialize, Serialize};

use crate::actions::{
    Capture, CopyExport, DeleteSelected, OpenSettings, PasteSnip, StartDraw, ToggleFormat,
    UploadImage,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutId {
    Capture,
    Show,
    Upload,
    Paste,
    Draw,
    Copy,
    ToggleFormat,
    Delete,
    Settings,
}

impl ShortcutId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Show => "show",
            Self::Upload => "upload",
            Self::Paste => "paste",
            Self::Draw => "draw",
            Self::Copy => "copy",
            Self::ToggleFormat => "toggle_format",
            Self::Delete => "delete",
            Self::Settings => "settings",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Capture,
    Document,
    Window,
}

/// Catalog-facing OS commands. Tray Quit is not a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalCmd {
    Capture,
    Show,
}

pub struct Spec {
    pub id: ShortcutId,
    pub group: Group,
    pub label: &'static str,
    pub default: &'static str,
    pub context: Option<&'static str>,
    pub required: bool,
    pub os: Option<GlobalCmd>,
}

impl Spec {
    pub fn global(&self) -> bool {
        self.os.is_some()
    }
}

/// Missing key means catalog default. JSON `null` means unbound.
pub type Overrides = BTreeMap<ShortcutId, Option<String>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignError {
    Invalid,
    ModifierOnly,
    TakenByRequired,
}

pub const CATALOG: &[Spec] = &[
    Spec {
        id: ShortcutId::Capture,
        group: Group::Capture,
        label: "Create snip from screenshot",
        default: "ctrl-alt-m",
        context: None,
        required: false,
        os: Some(GlobalCmd::Capture),
    },
    Spec {
        id: ShortcutId::Show,
        group: Group::Window,
        label: "Toggle main window",
        default: "ctrl-alt-l",
        context: None,
        required: false,
        os: Some(GlobalCmd::Show),
    },
    Spec {
        id: ShortcutId::Upload,
        group: Group::Capture,
        label: "Upload snip",
        default: "ctrl-o",
        context: None,
        required: false,
        os: None,
    },
    Spec {
        id: ShortcutId::Paste,
        group: Group::Capture,
        label: "Paste image or path from clipboard",
        default: "ctrl-v",
        context: None,
        required: true,
        os: None,
    },
    Spec {
        id: ShortcutId::Draw,
        group: Group::Capture,
        label: "Create snip from drawing",
        default: "ctrl-d",
        context: None,
        required: false,
        os: None,
    },
    Spec {
        id: ShortcutId::Copy,
        group: Group::Document,
        label: "Copy last format",
        default: "ctrl-c",
        context: None,
        required: true,
        os: None,
    },
    Spec {
        id: ShortcutId::ToggleFormat,
        group: Group::Document,
        label: "Toggle Markdown / LaTeX",
        default: "ctrl-l",
        context: None,
        required: false,
        os: None,
    },
    Spec {
        id: ShortcutId::Delete,
        group: Group::Document,
        label: "Delete selected snip",
        default: "delete",
        context: Some("SnipList && !SearchField"),
        required: true,
        os: None,
    },
    Spec {
        id: ShortcutId::Settings,
        group: Group::Window,
        label: "Settings",
        default: "ctrl-,",
        context: None,
        required: false,
        os: None,
    },
];

pub fn spec(id: ShortcutId) -> &'static Spec {
    CATALOG
        .iter()
        .find(|s| s.id == id)
        .expect("catalog covers every ShortcutId")
}

pub fn effective(over: &Overrides, id: ShortcutId) -> Option<String> {
    match over.get(&id) {
        Some(None) => None,
        Some(Some(chord)) => Some(chord.clone()),
        None => Some(spec(id).default.to_string()),
    }
}

pub fn global_bindings(over: &Overrides) -> Vec<(ShortcutId, String, GlobalCmd)> {
    CATALOG
        .iter()
        .filter_map(|s| {
            let cmd = s.os?;
            let chord = effective(over, s.id)?;
            Some((s.id, chord, cmd))
        })
        .collect()
}

pub fn is_customized(over: &Overrides, id: ShortcutId) -> bool {
    effective(over, id).as_deref() != Some(spec(id).default)
}

pub fn normalize(raw: &str) -> Result<String, AssignError> {
    let ks = Keystroke::parse(raw).map_err(|_| AssignError::Invalid)?;
    if matches!(
        ks.key.as_str(),
        "control" | "shift" | "alt" | "platform" | "fn" | "function"
    ) {
        return Err(AssignError::ModifierOnly);
    }
    Ok(ks.unparse())
}

pub fn assign(
    over: &mut Overrides,
    id: ShortcutId,
    chord: String,
) -> Result<Option<ShortcutId>, AssignError> {
    let chord = normalize(&chord)?;
    if effective(over, id).as_deref() == Some(chord.as_str()) {
        return Ok(None);
    }
    let mut stolen = None;
    if let Some(other) = holder(over, &chord, id) {
        if spec(other).required {
            return Err(AssignError::TakenByRequired);
        }
        over.insert(other, None);
        stolen = Some(other);
    }
    if chord == spec(id).default {
        over.remove(&id);
    } else {
        over.insert(id, Some(chord));
    }
    Ok(stolen)
}

pub fn restore(over: &mut Overrides, id: ShortcutId) -> Result<Option<ShortcutId>, AssignError> {
    assign(over, id, spec(id).default.to_string())
}

pub fn unbind(over: &mut Overrides, id: ShortcutId) {
    over.insert(id, None);
}

pub fn reset(over: &mut Overrides) {
    over.clear();
}

pub fn chips(chord: &str) -> Vec<String> {
    let Ok(ks) = Keystroke::parse(chord) else {
        return vec![chord.to_string()];
    };
    let mut out = Vec::new();
    if ks.modifiers.control {
        out.push("Ctrl".into());
    }
    if ks.modifiers.alt {
        out.push("Alt".into());
    }
    if ks.modifiers.platform {
        out.push(if cfg!(target_os = "macos") {
            "Cmd".into()
        } else if cfg!(target_os = "windows") {
            "Win".into()
        } else {
            "Super".into()
        });
    }
    if ks.modifiers.shift {
        out.push("Shift".into());
    }
    let key = if ks.key.len() == 1 {
        ks.key.to_ascii_uppercase()
    } else if ks.key == "delete" {
        "Delete".into()
    } else {
        let mut s = ks.key;
        if let Some(c) = s.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        s
    };
    out.push(key);
    out
}

pub fn to_global_hotkey(chord: &str) -> Option<HotKey> {
    let ks = Keystroke::parse(chord).ok()?;
    let mut mods = HotMods::empty();
    if ks.modifiers.control {
        mods |= HotMods::CONTROL;
    }
    if ks.modifiers.alt {
        mods |= HotMods::ALT;
    }
    if ks.modifiers.shift {
        mods |= HotMods::SHIFT;
    }
    if ks.modifiers.platform {
        mods |= HotMods::SUPER;
    }
    if mods.is_empty() {
        return None;
    }
    let code = gpui_key_to_code(&ks.key)?;
    Some(HotKey::new(Some(mods), code))
}

pub fn apply(cx: &mut App, over: &Overrides) {
    use crate::actions::{
        CloseSheet, CloseWindow, DrawEraser, DrawPen, DrawRedo, DrawUndo, QuitApp, RetryOcr,
        SelectNext, SelectPrev, ToggleSource,
    };
    cx.clear_key_bindings();
    cx.bind_keys([
        KeyBinding::new("ctrl-n", Capture, None),
        KeyBinding::new("escape", CloseSheet, None),
        KeyBinding::new("ctrl-shift-c", CopyExport, None),
        KeyBinding::new("down", SelectNext, None),
        KeyBinding::new("j", SelectNext, Some("SnipList && !SearchField")),
        KeyBinding::new("up", SelectPrev, None),
        KeyBinding::new("k", SelectPrev, Some("SnipList && !SearchField")),
        KeyBinding::new("left", SelectPrev, Some("OrigView")),
        KeyBinding::new("right", SelectNext, Some("OrigView")),
        KeyBinding::new(
            "backspace",
            DeleteSelected,
            Some("SnipList && !SearchField"),
        ),
        KeyBinding::new("ctrl-r", RetryOcr, None),
        KeyBinding::new("ctrl-e", ToggleSource, None),
        KeyBinding::new("ctrl-q", QuitApp, None),
        KeyBinding::new("ctrl-w", CloseWindow, None),
    ]);
    cx.bind_keys([
        KeyBinding::new("1", DrawPen, Some("DrawBoard")),
        KeyBinding::new("2", DrawEraser, Some("DrawBoard")),
        KeyBinding::new("3", DrawUndo, Some("DrawBoard")),
        KeyBinding::new("4", DrawRedo, Some("DrawBoard")),
        KeyBinding::new("ctrl-z", DrawUndo, Some("DrawBoard")),
        KeyBinding::new("ctrl-shift-z", DrawRedo, Some("DrawBoard")),
    ]);
    for spec in CATALOG {
        let Some(chord) = effective(over, spec.id) else {
            continue;
        };
        if Keystroke::parse(&chord).is_err() {
            continue;
        }
        bind_catalog(cx, spec, &chord);
    }
    crate::ui::search_field::bind_keys(cx);
    crate::ui::source_editor::bind_keys(cx);
}

fn bind_catalog(cx: &mut App, spec: &Spec, chord: &str) {
    match spec.id {
        ShortcutId::Capture => {
            cx.bind_keys([KeyBinding::new(chord, Capture, spec.context)]);
        }
        ShortcutId::Show => {}
        ShortcutId::Upload => {
            cx.bind_keys([KeyBinding::new(chord, UploadImage, spec.context)]);
        }
        ShortcutId::Paste => {
            cx.bind_keys([KeyBinding::new(chord, PasteSnip, spec.context)]);
        }
        ShortcutId::Draw => {
            cx.bind_keys([KeyBinding::new(chord, StartDraw, spec.context)]);
        }
        ShortcutId::Copy => {
            cx.bind_keys([KeyBinding::new(chord, CopyExport, spec.context)]);
        }
        ShortcutId::ToggleFormat => {
            cx.bind_keys([KeyBinding::new(chord, ToggleFormat, spec.context)]);
        }
        ShortcutId::Delete => {
            cx.bind_keys([KeyBinding::new(chord, DeleteSelected, spec.context)]);
        }
        ShortcutId::Settings => {
            cx.bind_keys([KeyBinding::new(chord, OpenSettings, spec.context)]);
        }
    }
}

fn holder(over: &Overrides, chord: &str, except: ShortcutId) -> Option<ShortcutId> {
    CATALOG.iter().find_map(|s| {
        if s.id == except {
            return None;
        }
        (effective(over, s.id).as_deref() == Some(chord)).then_some(s.id)
    })
}

fn gpui_key_to_code(key: &str) -> Option<Code> {
    if key.len() == 1 {
        let c = key.chars().next()?;
        if c.is_ascii_lowercase() {
            return Code::from_str(&format!("Key{}", c.to_ascii_uppercase())).ok();
        }
        if c.is_ascii_digit() {
            return Code::from_str(&format!("Digit{c}")).ok();
        }
    }
    let name = match key {
        "," => "Comma",
        "." => "Period",
        "/" => "Slash",
        ";" => "Semicolon",
        "'" => "Quote",
        "[" => "BracketLeft",
        "]" => "BracketRight",
        "\\" => "Backslash",
        "-" => "Minus",
        "=" => "Equal",
        "`" => "Backquote",
        "delete" => "Delete",
        "backspace" => "Backspace",
        "enter" => "Enter",
        "tab" => "Tab",
        "space" => "Space",
        "escape" => "Escape",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        _ => return None,
    };
    Code::from_str(name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_catalog() {
        let over = Overrides::new();
        assert_eq!(
            effective(&over, ShortcutId::Capture).as_deref(),
            Some("ctrl-alt-m")
        );
        assert_eq!(
            effective(&over, ShortcutId::Show).as_deref(),
            Some("ctrl-alt-l")
        );
        assert!(!is_customized(&over, ShortcutId::Capture));
    }

    #[test]
    fn assign_stores_override_and_restore_clears_it() {
        let mut over = Overrides::new();
        assert_eq!(
            assign(&mut over, ShortcutId::Draw, "ctrl-shift-d".into()).unwrap(),
            None
        );
        assert_eq!(
            effective(&over, ShortcutId::Draw).as_deref(),
            Some("ctrl-shift-d")
        );
        assert!(is_customized(&over, ShortcutId::Draw));
        restore(&mut over, ShortcutId::Draw).unwrap();
        assert!(!is_customized(&over, ShortcutId::Draw));
        assert!(over.is_empty());
    }

    #[test]
    fn steal_unbinds_non_required() {
        let mut over = Overrides::new();
        let stolen = assign(&mut over, ShortcutId::Capture, "ctrl-o".into()).unwrap();
        assert_eq!(stolen, Some(ShortcutId::Upload));
        assert_eq!(effective(&over, ShortcutId::Upload), None);
        assert_eq!(
            effective(&over, ShortcutId::Capture).as_deref(),
            Some("ctrl-o")
        );
    }

    #[test]
    fn steal_from_required_is_refused() {
        let mut over = Overrides::new();
        assert_eq!(
            assign(&mut over, ShortcutId::Capture, "ctrl-c".into()),
            Err(AssignError::TakenByRequired)
        );
        assert_eq!(
            effective(&over, ShortcutId::Copy).as_deref(),
            Some("ctrl-c")
        );
        assert!(!is_customized(&over, ShortcutId::Capture));
    }

    #[test]
    fn restore_steals_default_back() {
        let mut over = Overrides::new();
        assign(&mut over, ShortcutId::Capture, "ctrl-o".into()).unwrap();
        restore(&mut over, ShortcutId::Upload).unwrap();
        assert_eq!(
            effective(&over, ShortcutId::Upload).as_deref(),
            Some("ctrl-o")
        );
        assert_eq!(effective(&over, ShortcutId::Capture), None);
    }

    #[test]
    fn chips_split_modifiers() {
        assert_eq!(chips("ctrl-shift-s"), ["Ctrl", "Shift", "S"]);
        assert_eq!(chips("delete"), ["Delete"]);
        assert_eq!(chips("ctrl-,"), ["Ctrl", ","]);
    }

    #[test]
    fn global_hotkey_needs_a_modifier() {
        assert!(to_global_hotkey("ctrl-shift-s").is_some());
        assert!(to_global_hotkey("s").is_none());
    }

    #[test]
    fn normalize_rejects_modifier_only() {
        assert_eq!(normalize("ctrl"), Err(AssignError::ModifierOnly));
        assert_eq!(normalize("Ctrl-Shift-S").unwrap(), "ctrl-shift-s");
    }

    #[test]
    fn unbind_required_stores_null() {
        let mut over = Overrides::new();
        unbind(&mut over, ShortcutId::Copy);
        assert_eq!(over.get(&ShortcutId::Copy), Some(&None));
        assert_eq!(effective(&over, ShortcutId::Copy), None);
        assert!(is_customized(&over, ShortcutId::Copy));
    }

    #[test]
    fn restore_after_unbind_brings_default_back() {
        let mut over = Overrides::new();
        unbind(&mut over, ShortcutId::Copy);
        restore(&mut over, ShortcutId::Copy).unwrap();
        assert_eq!(
            effective(&over, ShortcutId::Copy).as_deref(),
            Some("ctrl-c")
        );
        assert!(!is_customized(&over, ShortcutId::Copy));
        assert!(over.is_empty());
    }

    #[test]
    fn unbind_then_assign_new_chord() {
        let mut over = Overrides::new();
        unbind(&mut over, ShortcutId::Draw);
        assert_eq!(effective(&over, ShortcutId::Draw), None);
        assign(&mut over, ShortcutId::Draw, "ctrl-shift-d".into()).unwrap();
        assert_eq!(
            effective(&over, ShortcutId::Draw).as_deref(),
            Some("ctrl-shift-d")
        );
    }

    #[test]
    fn global_bindings_empty_overrides_capture_and_show() {
        let over = Overrides::new();
        let binds = global_bindings(&over);
        assert_eq!(
            binds
                .iter()
                .map(|(id, chord, cmd)| (*id, chord.as_str(), *cmd))
                .collect::<Vec<_>>(),
            vec![
                (ShortcutId::Capture, "ctrl-alt-m", GlobalCmd::Capture),
                (ShortcutId::Show, "ctrl-alt-l", GlobalCmd::Show),
            ]
        );
    }

    #[test]
    fn catalog_os_rows_are_global() {
        for spec in CATALOG {
            assert_eq!(spec.global(), spec.os.is_some());
            if let Some(cmd) = spec.os {
                assert!(matches!(cmd, GlobalCmd::Capture | GlobalCmd::Show));
            }
        }
        assert_eq!(spec(ShortcutId::Show).default, "ctrl-alt-l");
        assert_eq!(spec(ShortcutId::Show).group, Group::Window);
        let id: ShortcutId = serde_json::from_str("\"show\"").unwrap();
        assert_eq!(id, ShortcutId::Show);
        assert_eq!(id.as_str(), "show");
    }

    #[test]
    fn catalog_does_not_advertise_ctrl_w() {
        assert!(CATALOG.iter().all(|s| s.default != "ctrl-w"));
    }
}
