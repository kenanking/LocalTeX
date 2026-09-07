use std::sync::Arc;
use std::time::SystemTime;

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::export::CopyKind;
use crate::table;

pub use crate::math::{MathRun, split_math, unwrap_formula};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockKind {
    Text,
    Formula,
    Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExportFmt {
    #[default]
    Markdown,
    Latex,
}

impl ExportFmt {
    pub fn toggle(self) -> Self {
        match self {
            ExportFmt::Markdown => ExportFmt::Latex,
            ExportFmt::Latex => ExportFmt::Markdown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportFmt::Markdown => "Markdown",
            ExportFmt::Latex => "LaTeX",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn from_points(x0: u32, y0: u32, x1: u32, y1: u32) -> Self {
        let x = x0.min(x1);
        let y = y0.min(y1);
        let w = x0.abs_diff(x1).max(1);
        let h = y0.abs_diff(y1).max(1);
        Self { x, y, w, h }
    }

    pub fn bottom(self) -> u32 {
        self.y.saturating_add(self.h)
    }
}

/// Layout role. Orthogonal to [`BlockKind`]: kind is payload grammar,
/// Structural weight used by export and preview layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlockRole {
    #[default]
    Body,
    DocTitle,
    SectionTitle,
    Caption,
}

impl BlockRole {
    pub fn interrupts_prose(self) -> bool {
        !matches!(self, Self::Body)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    pub bbox: Rect,
    pub text: String,
    /// Layout `display_formula`, or OCR wrapped the body in `$$`.
    pub display: bool,
    pub role: BlockRole,
}

impl Block {
    pub fn new(kind: BlockKind, bbox: Rect, text: impl Into<String>) -> Self {
        Self {
            kind,
            bbox,
            text: text.into(),
            display: false,
            role: BlockRole::Body,
        }
    }

    pub fn with_role(mut self, role: BlockRole) -> Self {
        self.role = role;
        self
    }

    pub fn promote_html_table(&mut self) {
        if self.kind != BlockKind::Table && table::looks_like_html_table(&self.text) {
            self.kind = BlockKind::Table;
        }
    }
}

pub fn decode_blocks_json(json: &str) -> Result<Vec<Block>, serde_json::Error> {
    serde_json::from_str(json)
}

pub fn encode_blocks_json(blocks: &[Block]) -> Result<String, serde_json::Error> {
    serde_json::to_string(blocks)
}

#[derive(Debug, Clone)]
pub enum DocStatus {
    Recognizing,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnipKind {
    Formula,
    Table,
    Mixed,
}

#[derive(Clone)]
pub enum ImageSlot {
    OnDisk,
    Loaded(Arc<RgbaImage>),
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistState {
    New,
    InsertPending { revision: u64 },
    Stored,
}

impl ImageSlot {
    pub fn pixels(&self) -> Option<&Arc<RgbaImage>> {
        match self {
            Self::Loaded(img) => Some(img),
            Self::OnDisk | Self::Missing => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcrMeta {
    pub elapsed_s: f32,
    pub confidence: Option<f32>,
}

#[derive(Clone)]
pub struct Document {
    pub id: Uuid,
    pub created_at: SystemTime,
    pub image: ImageSlot,
    pub blocks: Vec<Block>,
    pub status: DocStatus,
    pub first_line: String,
    pub thumb_jpeg: Vec<u8>,
    pub persist: PersistState,
    pub blocks_loaded: bool,
    pub ocr: Option<OcrMeta>,
    pub ink: Option<Arc<Vec<Vec<[f32; 3]>>>>,
    pub revision: u64,
    /// Snapshot written at recognition. Edits change `blocks` only.
    pub ocr_blocks: Vec<Block>,
    pub raw_text: Option<String>,
    pub source_error: Option<String>,
    pub source_pending: bool,
    pub source_updated_at: Option<std::time::Instant>,
}

impl Document {
    pub fn source_feedback_ready(&self) -> bool {
        self.source_updated_at
            .is_none_or(|at| at.elapsed() >= crate::source::PREVIEW_FEEDBACK_DELAY)
    }

    pub fn pending(image: Arc<RgbaImage>) -> Self {
        Self {
            id: Uuid::new_v4(),
            created_at: SystemTime::now(),
            image: ImageSlot::Loaded(image),
            blocks: Vec::new(),
            status: DocStatus::Recognizing,
            first_line: String::new(),
            thumb_jpeg: Vec::new(),
            persist: PersistState::New,
            blocks_loaded: true,
            ocr: None,
            ink: None,
            revision: 0,
            ocr_blocks: Vec::new(),
            raw_text: None,
            source_error: None,
            source_pending: false,
            source_updated_at: None,
        }
    }

    pub fn from_list_item(item: crate::store::SnipListItem) -> Self {
        Self {
            id: item.id,
            created_at: item.created_at,
            image: ImageSlot::OnDisk,
            blocks: Vec::new(),
            status: item.status,
            first_line: item.first_line,
            thumb_jpeg: Vec::new(),
            persist: PersistState::Stored,
            blocks_loaded: false,
            ocr: item.ocr,
            ink: None,
            revision: 0,
            ocr_blocks: Vec::new(),
            raw_text: None,
            source_error: None,
            source_pending: false,
            source_updated_at: None,
        }
    }

    pub fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn is_persisted(&self) -> bool {
        matches!(self.persist, PersistState::Stored)
    }

    pub fn insert_pending_revision(&self) -> Option<u64> {
        match self.persist {
            PersistState::InsertPending { revision } => Some(revision),
            PersistState::New | PersistState::Stored => None,
        }
    }

    pub fn first_line(&self) -> String {
        if let Some(text) = &self.raw_text {
            return collapse_preview_line(text);
        }
        match &self.status {
            DocStatus::Recognizing => "Recognizing…".into(),
            DocStatus::Failed(err) => format!("Failed: {err}"),
            DocStatus::Ready => {
                if !self.first_line.is_empty() {
                    collapse_preview_line(&self.first_line)
                } else {
                    ready_first_line(&self.blocks)
                }
            }
        }
    }

    pub fn refresh_first_line(&mut self) {
        if let Some(text) = &self.raw_text {
            self.first_line = collapse_preview_line(text);
            return;
        }
        if matches!(self.status, DocStatus::Ready) {
            self.first_line = ready_first_line(&self.blocks);
        }
    }

    pub fn is_edited(&self) -> bool {
        self.raw_text.is_some() || (self.blocks_loaded && self.blocks != self.ocr_blocks)
    }

    pub fn has_ready_blocks(&self) -> bool {
        matches!(self.status, DocStatus::Ready) && self.blocks_loaded
    }

    pub fn can_retry(&self) -> bool {
        !matches!(self.image, ImageSlot::Missing) && !matches!(self.status, DocStatus::Recognizing)
    }

    pub fn age_label(&self) -> String {
        let Ok(elapsed) = SystemTime::now().duration_since(self.created_at) else {
            return crate::i18n::t("history.now");
        };
        let secs = elapsed.as_secs();
        if secs < 5 {
            crate::i18n::t("history.just_now")
        } else if secs < 60 {
            rust_i18n::t!("history.seconds_ago", n = secs).into_owned()
        } else if secs < 3600 {
            rust_i18n::t!("history.minutes_ago", n = secs / 60).into_owned()
        } else {
            rust_i18n::t!("history.hours_ago", n = secs / 3600).into_owned()
        }
    }

    pub fn snip_kind(&self) -> SnipKind {
        snip_kind(&self.blocks)
    }

    pub fn search_text_for_blocks(blocks: &[Block]) -> String {
        let mut out = String::new();
        for b in blocks {
            out.push_str(&b.text);
            out.push('\n');
        }
        out
    }

    #[cfg(test)]
    pub fn primary_copy(&self, fmt: ExportFmt, prefs: &crate::prefs::Prefs) -> String {
        self.text_for(CopyKind::primary(self.snip_kind(), fmt), prefs)
    }

    pub fn text_for(&self, kind: CopyKind, prefs: &crate::prefs::Prefs) -> String {
        if self.source_pending || self.source_error.is_some() {
            return self.raw_text.clone().unwrap_or_default();
        }
        kind.render(&self.blocks, prefs)
    }
}

pub fn snip_kind(blocks: &[Block]) -> SnipKind {
    let mut n_formula = 0usize;
    let mut n_table = 0usize;
    let mut n_text = 0usize;
    for b in blocks {
        if b.text.trim().is_empty() {
            continue;
        }
        match b.kind {
            BlockKind::Formula => n_formula += 1,
            BlockKind::Table => n_table += 1,
            BlockKind::Text => {
                if !b.role.interrupts_prose() {
                    n_text += 1;
                }
            }
        }
    }
    if n_formula > 0 && n_table == 0 && n_text == 0 {
        SnipKind::Formula
    } else if n_table > 0 && n_formula == 0 && n_text == 0 {
        SnipKind::Table
    } else {
        SnipKind::Mixed
    }
}

#[cfg(test)]
pub use crate::export::{copy_rows, export_blocks, visible_copy_rows};

fn ready_first_line(blocks: &[Block]) -> String {
    blocks
        .iter()
        .find(|b| !b.text.trim().is_empty())
        .map(|b| match b.kind {
            BlockKind::Table => table_preview_line(&b.text),
            _ => collapse_preview_line(&b.text),
        })
        .unwrap_or_else(|| "(empty)".into())
}

/// Sidebar / library titles are one line. OCR file lists keep blank lines in
/// `Block.text` (correct for preview); collapse them here, not in OCR.
fn collapse_preview_line(text: &str) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "(empty)".into()
    } else {
        collapsed.chars().take(48).collect()
    }
}

fn table_preview_line(html: &str) -> String {
    if let Some(t) = table::parse_html(html) {
        let slots = t.slot_grid();
        let cells: Vec<&str> = slots
            .iter()
            .flatten()
            .filter_map(|s| match s {
                table::Slot::Origin { text, .. } if !text.is_empty() => Some(text.as_str()),
                _ => None,
            })
            .take(4)
            .collect();
        if !cells.is_empty() {
            let s = cells.join(" · ");
            return s.chars().take(48).collect();
        }
    }
    "Table".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::CopyHabit;

    fn rect(y: u32) -> Rect {
        Rect {
            x: 0,
            y,
            w: 10,
            h: 10,
        }
    }

    #[test]
    fn copy_habit_ignores_inapplicable_and_isolates_kinds() {
        let mut h = CopyHabit::default();
        assert!(!h.remember(SnipKind::Formula, CopyKind::Tsv));
        assert_eq!(
            h.resolve(SnipKind::Formula, ExportFmt::Markdown),
            CopyKind::MdDisplay
        );

        assert!(h.remember(SnipKind::Table, CopyKind::Tsv));
        assert!(!h.remember(SnipKind::Table, CopyKind::Tsv));
        assert_eq!(
            h.resolve(SnipKind::Formula, ExportFmt::Markdown),
            CopyKind::MdDisplay
        );
        assert_eq!(h.resolve(SnipKind::Table, ExportFmt::Latex), CopyKind::Tsv);
    }

    #[test]
    fn copy_habit_preferred_drops_inapplicable_slot() {
        let h: CopyHabit = serde_json::from_str(r#"{"formula":"md_table"}"#).unwrap();
        assert_eq!(h.preferred(SnipKind::Formula), None);
        assert_eq!(
            h.resolve(SnipKind::Formula, ExportFmt::Latex),
            CopyKind::Latex
        );
    }

    #[test]
    fn copy_habit_unknown_id_is_none_for_that_slot() {
        let h: CopyHabit =
            serde_json::from_str(r#"{"formula":"not_a_kind","table":"tsv"}"#).unwrap();
        assert_eq!(h.preferred(SnipKind::Formula), None);
        assert_eq!(h.preferred(SnipKind::Table), Some(CopyKind::Tsv));
    }

    #[test]
    fn source_feedback_waits_for_idle_and_resets_on_new_edit() {
        let mut doc = Document::pending(Arc::new(RgbaImage::new(1, 1)));
        assert!(doc.source_feedback_ready());
        doc.source_updated_at =
            Some(std::time::Instant::now() - crate::source::PREVIEW_FEEDBACK_DELAY);
        assert!(doc.source_feedback_ready());
        doc.source_updated_at = Some(std::time::Instant::now());
        assert!(!doc.source_feedback_ready());
    }

    #[test]
    fn pending_source_copies_latest_text_instead_of_previous_blocks() {
        let mut doc = Document::pending(Arc::new(RgbaImage::new(1, 1)));
        doc.blocks = vec![Block::new(BlockKind::Formula, rect(0), "x")];
        doc.raw_text = Some("$$ y $$".into());
        doc.source_pending = true;
        assert_eq!(
            doc.text_for(CopyKind::Markdown, &crate::prefs::Prefs::default()),
            "$$ y $$"
        );
    }

    #[test]
    fn primary_copy_shares_kind_table_with_habit_resolve() {
        let mut doc = Document::pending(Arc::new(RgbaImage::new(1, 1)));
        doc.status = DocStatus::Ready;
        doc.blocks = vec![Block::new(BlockKind::Formula, rect(0), r"x^{2}")];
        let prefs = crate::prefs::Prefs::default();
        assert_eq!(
            doc.primary_copy(ExportFmt::Markdown, &prefs),
            doc.text_for(
                CopyKind::primary(SnipKind::Formula, ExportFmt::Markdown),
                &prefs
            )
        );
        let habit = CopyHabit::default();
        assert_eq!(
            habit.resolve(SnipKind::Formula, ExportFmt::Markdown),
            CopyKind::primary(SnipKind::Formula, ExportFmt::Markdown)
        );
    }

    #[test]
    fn search_blob_is_raw_block_text() {
        let blocks = vec![Block::new(BlockKind::Formula, rect(0), r"\frac{1}{2}")];
        let blob = Document::search_text_for_blocks(&blocks);
        assert!(blob.contains(r"\frac{1}{2}"));
        assert!(
            !blob.contains('$'),
            "must not wrap raw TeX in Markdown delimiters"
        );
    }

    #[test]
    fn markdown_wraps_formulas() {
        let blocks = vec![
            Block::new(BlockKind::Text, rect(0), "Hello"),
            Block::new(BlockKind::Formula, rect(20), "E=mc^2"),
        ];
        let md = export_blocks(
            &blocks,
            ExportFmt::Markdown,
            &crate::prefs::Prefs::default(),
        );
        assert_eq!(md, "Hello\n$E=mc^2$");
    }

    #[test]
    fn markdown_joins_inline_formula_on_same_line() {
        let blocks = vec![
            Block::new(
                BlockKind::Text,
                Rect {
                    x: 0,
                    y: 0,
                    w: 40,
                    h: 16,
                },
                "Let",
            ),
            Block::new(
                BlockKind::Formula,
                Rect {
                    x: 44,
                    y: 0,
                    w: 40,
                    h: 16,
                },
                "E=mc^2",
            ),
            Block::new(
                BlockKind::Text,
                Rect {
                    x: 88,
                    y: 0,
                    w: 50,
                    h: 16,
                },
                "denote",
            ),
        ];
        let md = export_blocks(
            &blocks,
            ExportFmt::Markdown,
            &crate::prefs::Prefs::default(),
        );
        assert_eq!(md, "Let $E=mc^2$ denote");
    }

    #[test]
    fn formula_copy_rows_match_mathpix() {
        let blocks = vec![Block::new(BlockKind::Formula, rect(0), r"x^{2}")];
        let rows = copy_rows(&blocks, &crate::prefs::Prefs::default());
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].kind, CopyKind::MsWord);
        assert_eq!(rows[0].kind.label(), "MathML");
        assert!(
            rows[0]
                .text
                .contains(r#"xmlns="http://www.w3.org/1998/Math/MathML""#),
            "{}",
            rows[0].text
        );
        assert_eq!(rows[1].text, r"x^{2}");
        assert_eq!(rows[2].kind.label(), "Inline");
        assert_eq!(rows[2].text, r"$x^{2}$");
        assert_eq!(rows[3].kind.label(), "Display");
        assert_eq!(rows[3].text, "$$\nx^{2}\n$$");
        assert!(rows[4].text.contains(r"\begin{equation}"));
        assert_eq!(rows[0].kind.symbol(), "ml");
        assert_eq!(rows[1].kind.symbol(), "TeX");
        assert_eq!(rows[2].kind.symbol(), "$");
        assert_eq!(rows[3].kind.symbol(), "$$");
        assert_eq!(rows[4].kind.symbol(), "eq");
        assert!(rows[4].text.contains(r"x^{2}"));
    }

    #[test]
    fn table_block_exports_markdown_and_latex() {
        let html = "<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>";
        let blocks = vec![Block::new(BlockKind::Table, rect(0), html)];
        assert_eq!(snip_kind(&blocks), SnipKind::Table);
        let prefs = crate::prefs::Prefs::default();
        let md = export_blocks(&blocks, ExportFmt::Markdown, &prefs);
        assert!(md.contains("| A | B |"), "{md}");
        let tex = export_blocks(&blocks, ExportFmt::Latex, &prefs);
        assert!(tex.contains("\\begin{tabular}"), "{tex}");
        let rows = copy_rows(&blocks, &prefs);
        assert_eq!(rows[0].kind, CopyKind::LatexTable);
        assert_eq!(rows[1].kind, CopyKind::MdTable);
        assert_eq!(rows[0].kind.symbol(), "TeX");
        assert_eq!(rows[1].kind.symbol(), "MD");
    }

    #[test]
    fn mixed_copy_keeps_latex_row_in_api() {
        let prefs = crate::prefs::Prefs::default();
        let blocks = vec![Block::new(
            BlockKind::Text,
            rect(0),
            "A long paragraph without math.",
        )];
        let rows = copy_rows(&blocks, &prefs);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, CopyKind::Markdown);
        assert_eq!(rows[1].kind, CopyKind::LatexDoc);
        assert_eq!(visible_copy_rows(&rows).len(), 1);
        let blocks = vec![Block::new(
            BlockKind::Text,
            rect(0),
            r"Hence $$R@K=\frac{1}{N}$$ (11) holds.",
        )];
        let rows = copy_rows(&blocks, &prefs);
        assert_eq!(rows.len(), 2, "API still has both formats");
        assert_eq!(
            visible_copy_rows(&rows).len(),
            1,
            "UI hides LaTeX when the preview payload matches Markdown"
        );
        let blocks = vec![
            Block::new(BlockKind::Text, rect(0), "Let "),
            Block::new(BlockKind::Formula, rect(20), "x_1"),
        ];
        let rows = copy_rows(&blocks, &prefs);
        assert!(
            visible_copy_rows(&rows)
                .iter()
                .any(|r| r.kind == CopyKind::LatexDoc),
            "LaTeX stays visible when it differs from Markdown"
        );
    }

    #[test]
    fn table_plus_caption_is_table_snip() {
        let html = "<table><tr><td>a</td></tr></table>";
        let blocks = vec![
            Block::new(BlockKind::Text, rect(0), "Table 1: Scores").with_role(BlockRole::Caption),
            Block::new(BlockKind::Table, rect(20), html),
        ];
        assert_eq!(snip_kind(&blocks), SnipKind::Table);
        let rows = copy_rows(&blocks, &crate::prefs::Prefs::default());
        assert!(
            rows.iter().any(|r| r.kind == CopyKind::MdTable),
            "caption must not hide table copy rows"
        );
    }

    #[test]
    fn title_plus_body_is_mixed() {
        let blocks = vec![
            Block::new(BlockKind::Text, rect(0), "Intro").with_role(BlockRole::SectionTitle),
            Block::new(BlockKind::Text, rect(20), "Body paragraph."),
        ];
        assert_eq!(snip_kind(&blocks), SnipKind::Mixed);
    }

    #[test]
    fn markdown_emits_heading_and_caption_blank_line() {
        let html = "<table><tr><th>A</th></tr><tr><td>1</td></tr></table>";
        let prefs = crate::prefs::Prefs::default();
        let titled = vec![
            Block::new(BlockKind::Text, rect(0), "Intro").with_role(BlockRole::DocTitle),
            Block::new(BlockKind::Text, rect(20), "Body."),
        ];
        let md = export_blocks(&titled, ExportFmt::Markdown, &prefs);
        assert!(md.starts_with("# Intro"), "{md}");
        assert!(md.contains("\n\nBody."), "{md}");

        let table = vec![
            Block::new(BlockKind::Text, rect(0), "Table 1: Scores").with_role(BlockRole::Caption),
            Block::new(BlockKind::Table, rect(40), html),
        ];
        let md = export_blocks(&table, ExportFmt::Markdown, &prefs);
        assert!(md.contains("Table 1: Scores\n\n"), "{md}");
        assert!(md.contains("| A |"), "{md}");
        let tex = copy_rows(&table, &prefs)
            .into_iter()
            .find(|r| r.kind == CopyKind::LatexTable)
            .expect("latex table row")
            .text;
        assert!(tex.contains("\\begin{table}"), "{tex}");
        assert!(tex.contains("\\caption{Table 1: Scores}"), "{tex}");
    }

    #[test]
    fn html_payload_in_text_block_promotes_to_table() {
        let mut b = Block::new(
            BlockKind::Text,
            rect(0),
            "<table><tr><td>a</td></tr></table>",
        );
        b.promote_html_table();
        assert_eq!(b.kind, BlockKind::Table);
    }

    #[test]
    fn first_line_collapses_blank_lines_in_text_block() {
        let blocks = vec![Block::new(
            BlockKind::Text,
            rect(0),
            "build.rs\n\nCargo.lock\n\nCargo.toml\n\nLICENSE\n\nREADME.md\n\nrustfmt.toml",
        )];
        let line = ready_first_line(&blocks);
        assert!(
            !line.contains('\n') && !line.contains('\r'),
            "sidebar title must be one line, got {line:?}"
        );
        assert!(
            line.starts_with("build.rs Cargo.lock"),
            "expected collapsed file names, got {line:?}"
        );
        assert!(line.chars().count() <= 48, "{line:?}");
    }

    #[test]
    fn first_line_getter_collapses_stored_newlines() {
        let mut doc = Document::pending(Arc::new(RgbaImage::new(1, 1)));
        doc.status = DocStatus::Ready;
        doc.first_line = "build.rs\n\nCargo.lock\n\nCargo.toml".into();
        assert_eq!(doc.first_line(), "build.rs Cargo.lock Cargo.toml");
    }
}
