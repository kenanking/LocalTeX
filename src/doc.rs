use std::sync::Arc;
use std::time::SystemTime;

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::math;
use crate::table;

pub use crate::math::{split_math, unwrap_formula, MathRun};

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
    pub fn label(self) -> &'static str {
        match self {
            ExportFmt::Markdown => "Markdown",
            ExportFmt::Latex => "LaTeX",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            ExportFmt::Markdown => ExportFmt::Latex,
            ExportFmt::Latex => ExportFmt::Markdown,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    pub bbox: Rect,
    pub text: String,
    /// Layout `display_formula`, or OCR wrapped the body in `$$`.
    #[serde(default)]
    pub display: bool,
}

impl Block {
    pub fn new(kind: BlockKind, bbox: Rect, text: impl Into<String>) -> Self {
        Self {
            kind,
            bbox,
            text: text.into(),
            display: false,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyKind {
    Latex,
    MdInline,
    MdDisplay,
    Equation,
    Markdown,
    LatexDoc,
    MdTable,
    LatexTable,
    Tsv,
}

impl CopyKind {
    pub fn label(self) -> &'static str {
        match self {
            CopyKind::Latex => "LaTeX",
            CopyKind::MdInline => "Markdown inline",
            CopyKind::MdDisplay => "Markdown display",
            CopyKind::Equation => "Equation",
            CopyKind::Markdown => "Markdown",
            CopyKind::LatexDoc => "LaTeX",
            CopyKind::MdTable => "Markdown table",
            CopyKind::LatexTable => "LaTeX table",
            CopyKind::Tsv => "TSV",
        }
    }
}

#[derive(Clone)]
pub struct CopyRow {
    pub kind: CopyKind,
    pub text: String,
}

#[derive(Clone)]
pub enum ImageSlot {
    /// File on disk; pixels not necessarily in RAM.
    OnDisk,
    /// Decoded screenshot (capture session or after lazy load).
    Loaded(Arc<RgbaImage>),
    /// DB row exists; PNG missing or unreadable. OCR blocks still valid.
    Missing,
}

impl ImageSlot {
    pub fn pixels(&self) -> Option<&Arc<RgbaImage>> {
        match self {
            Self::Loaded(img) => Some(img),
            Self::OnDisk | Self::Missing => None,
        }
    }
}

/// Wall time and hybrid confidence from one finished recognize. Absent
/// when OCR produced no token/layout evidence (or the row predates this).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcrMeta {
    pub elapsed_s: f32,
    pub confidence: f32,
}

#[derive(Clone)]
pub struct Document {
    pub id: Uuid,
    pub created_at: SystemTime,
    pub image: ImageSlot,
    pub blocks: Vec<Block>,
    pub status: DocStatus,
    /// Sidebar title; set from OCR / DB so the list does not need `blocks`.
    pub first_line: String,
    pub thumb_jpeg: Vec<u8>,
    pub persisted: bool,
    pub blocks_loaded: bool,
    pub ocr: Option<OcrMeta>,
}

impl Document {
    pub fn pending(image: Arc<RgbaImage>) -> Self {
        Self {
            id: Uuid::new_v4(),
            created_at: SystemTime::now(),
            image: ImageSlot::Loaded(image),
            blocks: Vec::new(),
            status: DocStatus::Recognizing,
            first_line: String::new(),
            thumb_jpeg: Vec::new(),
            persisted: false,
            blocks_loaded: true,
            ocr: None,
        }
    }

    pub fn from_list_item(item: crate::store::SnipListItem) -> Self {
        Self {
            id: item.id,
            created_at: item.created_at,
            image: if item.png_missing {
                ImageSlot::Missing
            } else {
                ImageSlot::OnDisk
            },
            blocks: Vec::new(),
            status: DocStatus::Ready,
            first_line: item.first_line,
            thumb_jpeg: item.thumb_jpeg,
            persisted: true,
            blocks_loaded: false,
            ocr: item.ocr,
        }
    }

    pub fn first_line(&self) -> String {
        match &self.status {
            DocStatus::Recognizing => "Recognizing…".into(),
            DocStatus::Failed(err) => format!("Failed: {err}"),
            DocStatus::Ready => {
                if !self.first_line.is_empty() {
                    self.first_line.clone()
                } else {
                    ready_first_line(&self.blocks)
                }
            }
        }
    }

    pub fn refresh_first_line(&mut self) {
        if matches!(self.status, DocStatus::Ready) {
            self.first_line = ready_first_line(&self.blocks);
        }
    }

    pub fn can_retry(&self) -> bool {
        !matches!(self.image, ImageSlot::Missing) && !matches!(self.status, DocStatus::Recognizing)
    }

    pub fn age_label(&self) -> String {
        let Ok(elapsed) = SystemTime::now().duration_since(self.created_at) else {
            return "now".into();
        };
        let secs = elapsed.as_secs();
        if secs < 5 {
            "just now".into()
        } else if secs < 60 {
            format!("{secs}s ago")
        } else if secs < 3600 {
            format!("{}m ago", secs / 60)
        } else {
            format!("{}h ago", secs / 3600)
        }
    }

    pub fn snip_kind(&self) -> SnipKind {
        snip_kind(&self.blocks)
    }

    /// FTS blob: raw block text + Markdown with default delimiters.
    pub fn search_text_for_blocks(blocks: &[Block]) -> String {
        let mut out = String::new();
        for b in blocks {
            out.push_str(&b.text);
            out.push('\n');
        }
        out.push_str(&export_blocks(
            blocks,
            ExportFmt::Markdown,
            &crate::prefs::Prefs::default(),
        ));
        out
    }

    pub fn copy_rows(&self, prefs: &crate::prefs::Prefs) -> Vec<CopyRow> {
        if !matches!(self.status, DocStatus::Ready) {
            return Vec::new();
        }
        match snip_kind(&self.blocks) {
            SnipKind::Mixed => mixed_copy_rows(&self.blocks, prefs),
            _ => copy_rows(&self.blocks, prefs),
        }
    }

    pub fn primary_copy(&self, fmt: ExportFmt, prefs: &crate::prefs::Prefs) -> String {
        let rows = self.copy_rows(prefs);
        let want = match (self.snip_kind(), fmt) {
            (SnipKind::Formula, ExportFmt::Markdown) => CopyKind::MdDisplay,
            (SnipKind::Formula, ExportFmt::Latex) => CopyKind::Latex,
            (SnipKind::Table, ExportFmt::Markdown) => CopyKind::MdTable,
            (SnipKind::Table, ExportFmt::Latex) => CopyKind::LatexTable,
            (SnipKind::Mixed, ExportFmt::Markdown) => CopyKind::Markdown,
            (SnipKind::Mixed, ExportFmt::Latex) => CopyKind::LatexDoc,
        };
        rows.iter()
            .find(|r| r.kind == want)
            .or(rows.first())
            .map(|r| r.text.clone())
            .unwrap_or_default()
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
                if table::looks_like_html_table(&b.text) {
                    n_table += 1;
                } else {
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

pub fn export_blocks(blocks: &[Block], fmt: ExportFmt, prefs: &crate::prefs::Prefs) -> String {
    let mut out = String::new();
    for (row_i, row) in group_rows(blocks).into_iter().enumerate() {
        if row_i > 0 {
            out.push('\n');
        }
        for (i, block) in row.into_iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(&export_block(block, fmt, prefs));
        }
    }
    out
}

fn export_block(block: &Block, fmt: ExportFmt, prefs: &crate::prefs::Prefs) -> String {
    let kind = effective_kind(block);
    match (fmt, kind) {
        (ExportFmt::Markdown, BlockKind::Formula) => {
            let body = unwrap_formula(&block.text).0;
            if block.display || math::is_display_body(&body) {
                prefs.wrap_block(&body)
            } else {
                prefs.wrap_inline(&body)
            }
        }
        (ExportFmt::Markdown, BlockKind::Table) => table::html_to_markdown(&block.text),
        (ExportFmt::Markdown, BlockKind::Text) => emit_text_block(&block.text, fmt, prefs),
        (ExportFmt::Latex, BlockKind::Formula) => {
            prefs.wrap_block(unwrap_formula(&block.text).0.trim())
        }
        (ExportFmt::Latex, BlockKind::Table) => table::html_to_latex(&block.text),
        (ExportFmt::Latex, BlockKind::Text) => emit_text_block(&block.text, fmt, prefs),
    }
}

fn effective_kind(block: &Block) -> BlockKind {
    if block.kind == BlockKind::Table || table::looks_like_html_table(&block.text) {
        BlockKind::Table
    } else {
        block.kind
    }
}

fn formula_body(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Formula && !b.text.trim().is_empty())
        .map(|b| unwrap_formula(&b.text).0)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" \\\\ ")
}

fn table_html(blocks: &[Block]) -> Option<String> {
    let mut parts = Vec::new();
    for b in blocks {
        if b.kind == BlockKind::Table || table::looks_like_html_table(&b.text) {
            parts.push(b.text.as_str());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

pub fn copy_rows(blocks: &[Block], prefs: &crate::prefs::Prefs) -> Vec<CopyRow> {
    match snip_kind(blocks) {
        SnipKind::Formula => {
            let body = formula_body(blocks);
            if body.is_empty() {
                return Vec::new();
            }
            vec![
                CopyRow {
                    kind: CopyKind::Latex,
                    text: body.clone(),
                },
                CopyRow {
                    kind: CopyKind::MdInline,
                    text: prefs.wrap_inline(&body),
                },
                CopyRow {
                    kind: CopyKind::MdDisplay,
                    text: format!("$$ {body} $$"),
                },
                CopyRow {
                    kind: CopyKind::Equation,
                    text: format!("\\begin{{equation}}\n{body}\n\\end{{equation}}"),
                },
            ]
        }
        SnipKind::Table => {
            let Some(html) = table_html(blocks) else {
                return Vec::new();
            };
            let parsed = table::parse_html(&html);
            let md = parsed
                .as_ref()
                .map(|t| t.to_markdown())
                .unwrap_or_else(|| table::html_to_markdown(&html));
            let tex = parsed
                .as_ref()
                .map(|t| t.to_latex())
                .unwrap_or_else(|| table::html_to_latex(&html));
            let tsv = parsed.as_ref().map(|t| t.to_tsv()).unwrap_or_default();
            let mut rows = vec![
                CopyRow {
                    kind: CopyKind::LatexTable,
                    text: tex,
                },
                CopyRow {
                    kind: CopyKind::MdTable,
                    text: md,
                },
            ];
            if !tsv.is_empty() {
                rows.push(CopyRow {
                    kind: CopyKind::Tsv,
                    text: tsv,
                });
            }
            rows
        }
        SnipKind::Mixed => mixed_copy_rows(blocks, prefs),
    }
}

fn mixed_copy_rows(blocks: &[Block], prefs: &crate::prefs::Prefs) -> Vec<CopyRow> {
    let md = export_blocks(blocks, ExportFmt::Markdown, prefs);
    let tex = export_blocks(blocks, ExportFmt::Latex, prefs);
    vec![
        CopyRow {
            kind: CopyKind::Markdown,
            text: md,
        },
        CopyRow {
            kind: CopyKind::LatexDoc,
            text: tex,
        },
    ]
}

/// Whitespace-collapsed equality matches the copy-row preview string.
pub fn copy_payload_eq(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

pub fn visible_copy_rows(rows: &[CopyRow]) -> Vec<CopyRow> {
    let md = rows.iter().find(|r| r.kind == CopyKind::Markdown);
    rows.iter()
        .filter(|r| {
            if r.kind == CopyKind::LatexDoc {
                !md.is_some_and(|m| copy_payload_eq(&m.text, &r.text))
            } else {
                true
            }
        })
        .cloned()
        .collect()
}

fn ready_first_line(blocks: &[Block]) -> String {
    blocks
        .iter()
        .find(|b| !b.text.trim().is_empty())
        .map(|b| match b.kind {
            BlockKind::Table => table_preview_line(&b.text),
            _ => b.text.chars().take(48).collect(),
        })
        .unwrap_or_else(|| "(empty)".into())
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

fn emit_text_block(text: &str, fmt: ExportFmt, prefs: &crate::prefs::Prefs) -> String {
    if table::looks_like_html_table(text) {
        return match fmt {
            ExportFmt::Latex => table::html_to_latex(text),
            ExportFmt::Markdown => table::html_to_markdown(text),
        };
    }
    let mut out = String::new();
    for run in split_math(text) {
        match run {
            MathRun::Text(s) => match fmt {
                ExportFmt::Latex => out.push_str(&escape_latex(&s)),
                ExportFmt::Markdown => out.push_str(&s),
            },
            MathRun::Inline(s) => out.push_str(&prefs.wrap_inline(&s)),
            MathRun::Display(s) => out.push_str(&prefs.wrap_block(&s)),
        }
    }
    out
}

fn group_rows(blocks: &[Block]) -> Vec<Vec<&Block>> {
    let mut rows: Vec<Vec<&Block>> = Vec::new();
    for block in blocks {
        if matches!(effective_kind(block), BlockKind::Table | BlockKind::Formula)
            && (block.kind == BlockKind::Table
                || table::looks_like_html_table(&block.text)
                || block.text.contains('\n')
                || block.text.len() > 48)
        {
            rows.push(vec![block]);
            continue;
        }
        if let Some(row) = rows.last_mut() {
            if let Some(prev) = row.last() {
                if vertically_overlap(prev.bbox, block.bbox) {
                    row.push(block);
                    continue;
                }
            }
        }
        rows.push(vec![block]);
    }
    rows
}

fn vertically_overlap(a: Rect, b: Rect) -> bool {
    a.y < b.bottom() && b.y < a.bottom()
}

pub fn escape_latex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '%' => out.push_str("\\%"),
            '&' => out.push_str("\\&"),
            '$' => out.push_str("\\$"),
            '#' => out.push_str("\\#"),
            '_' => out.push_str("\\_"),
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(y: u32) -> Rect {
        Rect {
            x: 0,
            y,
            w: 10,
            h: 10,
        }
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
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].text, r"x^{2}");
        assert_eq!(rows[1].text, r"$x^{2}$");
        assert_eq!(rows[2].text, r"$$ x^{2} $$");
        assert!(rows[3].text.contains(r"\begin{equation}"));
        assert!(rows[3].text.contains(r"x^{2}"));
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
}
