use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReadingWidth {
    Narrow,
    #[default]
    Medium,
    Wide,
}

impl ReadingWidth {
    pub fn cap_px(self) -> f32 {
        match self {
            Self::Narrow => 576.0,
            Self::Medium => 720.0,
            Self::Wide => 896.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContentFontSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Debug, Clone, Copy)]
pub struct ContentFontMetrics {
    pub body: f32,
    pub heading: f32,
    pub title: f32,
    pub caption: f32,
    pub inline_math: f64,
    pub display_math: f64,
    pub script: f64,
    pub body_line: f32,
    pub inline_line: f32,
    pub heading_line: f32,
    pub title_line: f32,
    pub table_line: f32,
}

impl ContentFontSize {
    pub fn metrics(self) -> ContentFontMetrics {
        match self {
            Self::Small => ContentFontMetrics {
                body: 12.0,
                heading: 16.0,
                title: 18.0,
                caption: 10.0,
                inline_math: 14.0,
                display_math: 16.0,
                script: 10.0,
                body_line: 18.0,
                inline_line: 14.0,
                heading_line: 20.0,
                title_line: 24.0,
                table_line: 16.0,
            },
            Self::Medium => ContentFontMetrics {
                body: 14.0,
                heading: 18.0,
                title: 20.0,
                caption: 12.0,
                inline_math: 16.0,
                display_math: 18.0,
                script: 11.5,
                body_line: 22.0,
                inline_line: 16.0,
                heading_line: 24.0,
                title_line: 28.0,
                table_line: 18.0,
            },
            Self::Large => ContentFontMetrics {
                body: 16.0,
                heading: 20.0,
                title: 22.0,
                caption: 14.0,
                inline_math: 18.0,
                display_math: 20.0,
                script: 13.0,
                body_line: 26.0,
                inline_line: 18.0,
                heading_line: 28.0,
                title_line: 32.0,
                table_line: 20.0,
            },
        }
    }
}

/// What the main window close button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowCloseAction {
    /// Keep the process (tray + hotkey). Clicking X hides to the tray.
    #[default]
    Minimize,
    /// Exit the app.
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub default_fmt: ExportFmt,
    #[serde(default = "default_true")]
    pub autocopy: bool,
    #[serde(default)]
    pub copy_habit: crate::export::CopyHabit,
    #[serde(default = "default_true")]
    pub show_original: bool,
    #[serde(default = "default_true")]
    pub hide_on_capture: bool,
    #[serde(default)]
    pub close_action: WindowCloseAction,
    #[serde(default)]
    pub launch_at_startup: bool,
    #[serde(default)]
    pub inline_delim: InlineDelim,
    #[serde(default)]
    pub block_delim: BlockDelim,
    /// History sidebar width in px, user-draggable between SIDEBAR limits.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    /// Original-image strip height in px. Window-level, not per-snip.
    #[serde(default = "default_orig_strip_h")]
    pub orig_strip_h: f32,
    /// User clicked collapse. Stays collapsed until they click expand.
    #[serde(default)]
    pub sidebar_pinned_collapsed: bool,
    /// Catalog overrides only. Missing key = default. `null` = unbound.
    #[serde(default)]
    pub shortcuts: keymap::Overrides,
    #[serde(default)]
    pub reading_width: ReadingWidth,
    #[serde(default)]
    pub content_font: ContentFontSize,
}

fn default_true() -> bool {
    true
}

fn default_sidebar_width() -> f32 {
    232.0
}

fn default_orig_strip_h() -> f32 {
    160.0
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            default_fmt: ExportFmt::Markdown,
            autocopy: true,
            copy_habit: crate::export::CopyHabit::default(),
            show_original: true,
            hide_on_capture: true,
            close_action: WindowCloseAction::Minimize,
            launch_at_startup: false,
            inline_delim: InlineDelim::Dollar,
            block_delim: BlockDelim::Dollars,
            sidebar_width: default_sidebar_width(),
            orig_strip_h: default_orig_strip_h(),
            sidebar_pinned_collapsed: false,
            shortcuts: keymap::Overrides::new(),
            reading_width: ReadingWidth::Medium,
            content_font: ContentFontSize::Medium,
        }
    }
}

impl Prefs {
    pub fn load() -> Self {
        let path = prefs_path();
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                eprintln!("{APP_SLUG}: read prefs: {err}");
                return Self::default();
            }
        };
        serde_json::from_str(&raw).unwrap_or_else(|err| {
            eprintln!("{APP_SLUG}: parse prefs: {err}");
            Self::default()
        })
    }

    pub(crate) fn save(&self) -> anyhow::Result<()> {
        let path = prefs_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw)?;
        replace_file(&tmp, &path)?;
        Ok(())
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

enum PrefsCommand {
    Save(Prefs),
    Shutdown(Sender<anyhow::Result<()>>),
}

#[derive(Clone)]
pub struct PrefsWriter {
    tx: Sender<PrefsCommand>,
    thread: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl PrefsWriter {
    pub fn start() -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("localtex-prefs-writer".into())
            .spawn(move || {
                while let Ok(command) = rx.recv() {
                    match command {
                        PrefsCommand::Save(prefs) => {
                            if let Err(err) = prefs.save() {
                                eprintln!("{APP_SLUG}: save prefs: {err:#}");
                            }
                        }
                        PrefsCommand::Shutdown(reply) => {
                            let _ = reply.send(Ok(()));
                            break;
                        }
                    }
                }
            })
            .map_err(|err| anyhow::anyhow!("spawn prefs writer: {err}"))?;
        Ok(Self {
            tx,
            thread: Arc::new(Mutex::new(Some(thread))),
        })
    }

    pub fn save(&self, prefs: Prefs) -> anyhow::Result<()> {
        self.tx
            .send(PrefsCommand::Save(prefs))
            .map_err(|_| anyhow::anyhow!("prefs writer stopped"))
    }

    pub fn shutdown(&self) -> anyhow::Result<()> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PrefsCommand::Shutdown(reply))
            .map_err(|_| anyhow::anyhow!("prefs writer stopped"))?;
        rx.recv()
            .map_err(|_| anyhow::anyhow!("prefs writer stopped"))??;
        let thread = self
            .thread
            .lock()
            .map_err(|_| anyhow::anyhow!("prefs writer join lock"))?
            .take();
        if let Some(thread) = thread {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("prefs writer panicked"))?;
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_file(tmp: &std::path::Path, path: &std::path::Path) -> std::io::Result<()> {
    fs::rename(tmp, path)
}

#[cfg(target_os = "windows")]
fn replace_file(tmp: &std::path::Path, path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let from: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|err| std::io::Error::other(err.to_string()))
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
        assert!(!p.launch_at_startup);
        assert!(p.hide_on_capture);
        assert!(p.autocopy);
    }

    #[test]
    fn launch_at_startup_defaults_off_when_missing() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert!(!p.launch_at_startup);
    }

    #[test]
    fn launch_at_startup_round_trips() {
        let p = Prefs {
            launch_at_startup: true,
            ..Prefs::default()
        };
        let raw = serde_json::to_string(&p).unwrap();
        let q: Prefs = serde_json::from_str(&raw).unwrap();
        assert!(q.launch_at_startup);
    }

    #[test]
    fn autocopy_false_in_json_stays_off_omitted_is_on() {
        let p: Prefs = serde_json::from_str(r#"{"autocopy":false}"#).unwrap();
        assert!(!p.autocopy);
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert!(p.autocopy);
    }

    #[test]
    fn copy_habit_unknown_id_does_not_fail_prefs() {
        let p: Prefs = serde_json::from_str(r#"{"copy_habit":{"formula":"not_a_kind"}}"#).unwrap();
        assert_eq!(p.copy_habit.preferred(crate::doc::SnipKind::Formula), None);
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
    fn orig_strip_h_defaults_when_missing() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p.orig_strip_h, default_orig_strip_h());
    }

    #[test]
    fn orig_strip_h_round_trips() {
        let p = Prefs {
            orig_strip_h: 180.0,
            ..Prefs::default()
        };
        let raw = serde_json::to_string(&p).unwrap();
        let q: Prefs = serde_json::from_str(&raw).unwrap();
        assert_eq!(q.orig_strip_h, 180.0);
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

    #[test]
    fn reading_width_defaults_medium_when_missing() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p.reading_width, ReadingWidth::Medium);
        assert_eq!(p.reading_width.cap_px(), 720.0);
    }

    #[test]
    fn reading_width_round_trips() {
        let p = Prefs {
            reading_width: ReadingWidth::Narrow,
            ..Prefs::default()
        };
        let raw = serde_json::to_string(&p).unwrap();
        let q: Prefs = serde_json::from_str(&raw).unwrap();
        assert_eq!(q.reading_width, ReadingWidth::Narrow);
        assert_eq!(q.reading_width.cap_px(), 576.0);
        assert_eq!(ReadingWidth::Wide.cap_px(), 896.0);
    }

    #[test]
    fn content_font_defaults_medium_when_missing() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p.content_font, ContentFontSize::Medium);
        assert_eq!(p.content_font.metrics().body, 14.0);
    }

    #[test]
    fn content_font_round_trips_small() {
        let p = Prefs {
            content_font: ContentFontSize::Small,
            ..Prefs::default()
        };
        let raw = serde_json::to_string(&p).unwrap();
        let q: Prefs = serde_json::from_str(&raw).unwrap();
        assert_eq!(q.content_font, ContentFontSize::Small);
    }

    #[test]
    fn content_font_medium_metrics_match_preview() {
        let m = ContentFontSize::Medium.metrics();
        assert_eq!(m.body, 14.0);
        assert_eq!(m.inline_math, 16.0);
        assert_eq!(m.display_math, 18.0);
    }
}
