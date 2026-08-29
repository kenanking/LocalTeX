use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::doc::ExportFmt;
use crate::identity::{self, APP_SLUG};
use crate::keymap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InlineDelim {
    #[default]
    Dollar,
    Paren,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlockDelim {
    #[default]
    Dollars,
    Brackets,
    Equation,
}

/// What the main window close button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowCloseAction {
    /// Keep the process (tray + hotkey). Clicking X minimizes.
    #[default]
    Minimize,
    /// Exit the app.
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub default_fmt: ExportFmt,
    #[serde(default)]
    pub autocopy: bool,
    #[serde(default = "default_true")]
    pub show_original: bool,
    #[serde(default = "default_true")]
    pub hide_on_capture: bool,
    #[serde(default)]
    pub close_action: WindowCloseAction,
    #[serde(default)]
    pub inline_delim: InlineDelim,
    #[serde(default)]
    pub block_delim: BlockDelim,
    /// History sidebar width in px, user-draggable between SIDEBAR limits.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    /// User clicked collapse. Stays collapsed until they click expand.
    #[serde(default)]
    pub sidebar_pinned_collapsed: bool,
    /// Catalog overrides only. Missing key = default. `null` = unbound.
    #[serde(default)]
    pub shortcuts: keymap::Overrides,
}

fn default_true() -> bool {
    true
}

fn default_sidebar_width() -> f32 {
    232.0
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            default_fmt: ExportFmt::Markdown,
            autocopy: false,
            show_original: true,
            hide_on_capture: true,
            close_action: WindowCloseAction::Minimize,
            inline_delim: InlineDelim::Dollar,
            block_delim: BlockDelim::Dollars,
            sidebar_width: default_sidebar_width(),
            sidebar_pinned_collapsed: false,
            shortcuts: keymap::Overrides::new(),
        }
    }
}

impl Prefs {
    pub fn load() -> Self {
        let path = prefs_path();
        let Ok(raw) = fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = prefs_path();
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!("{APP_SLUG}: prefs dir: {err}");
                return;
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(raw) => {
                if let Err(err) = fs::write(&path, raw) {
                    eprintln!("{APP_SLUG}: write prefs: {err}");
                }
            }
            Err(err) => eprintln!("{APP_SLUG}: encode prefs: {err}"),
        }
    }

    pub fn wrap_inline(&self, body: &str) -> String {
        match self.inline_delim {
            InlineDelim::Dollar => format!("${body}$"),
            InlineDelim::Paren => format!("\\({body}\\)"),
        }
    }

    pub fn wrap_block(&self, body: &str) -> String {
        match self.block_delim {
            BlockDelim::Dollars => format!("$$\n{body}\n$$"),
            BlockDelim::Brackets => format!("\\[\n{body}\n\\]"),
            BlockDelim::Equation => format!("\\begin{{equation*}}\n{body}\n\\end{{equation*}}"),
        }
    }
}

fn prefs_path() -> PathBuf {
    identity::config_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_wraps_match_mathpix_common() {
        let p = Prefs::default();
        assert_eq!(p.wrap_inline("x^2"), "$x^2$");
        assert_eq!(p.wrap_block("x^2"), "$$\nx^2\n$$");
        assert_eq!(p.close_action, WindowCloseAction::Minimize);
        assert!(p.hide_on_capture);
    }

    #[test]
    fn view_format_toggle_does_not_mutate_persisted_default() {
        let prefs = Prefs::default();
        let session = prefs.default_fmt.toggle();
        assert_eq!(session, ExportFmt::Latex);
        assert_eq!(prefs.default_fmt, ExportFmt::Markdown);
    }

    #[test]
    fn sidebar_pinned_collapsed_defaults_false_when_missing() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert!(!p.sidebar_pinned_collapsed);
    }

    #[test]
    fn sidebar_pinned_collapsed_round_trips() {
        let p = Prefs {
            sidebar_pinned_collapsed: true,
            ..Prefs::default()
        };
        let raw = serde_json::to_string(&p).unwrap();
        let q: Prefs = serde_json::from_str(&raw).unwrap();
        assert!(q.sidebar_pinned_collapsed);
    }
}
