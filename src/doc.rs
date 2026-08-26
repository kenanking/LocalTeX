use std::sync::Arc;
use std::time::SystemTime;

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Text,
    Formula,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Clone)]
pub struct Block {
    pub kind: BlockKind,
    pub bbox: Rect,
    pub text: String,
}

impl Block {
    pub fn new(kind: BlockKind, bbox: Rect, text: impl Into<String>) -> Self {
        Self {
            kind,
            bbox,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DocStatus {
    Recognizing,
    Ready,
    Failed(String),
}

#[derive(Clone)]
pub struct Document {
    pub id: Uuid,
    pub created_at: SystemTime,
    pub image: Arc<RgbaImage>,
    pub blocks: Vec<Block>,
    pub status: DocStatus,
}

impl Document {
    pub fn pending(image: Arc<RgbaImage>) -> Self {
        Self {
            id: Uuid::new_v4(),
            created_at: SystemTime::now(),
            image,
            blocks: Vec::new(),
            status: DocStatus::Recognizing,
        }
    }

    pub fn first_line(&self) -> String {
        match &self.status {
            DocStatus::Recognizing => "Recognizing…".into(),
            DocStatus::Failed(err) => format!("Failed: {err}"),
            DocStatus::Ready => self
                .blocks
                .iter()
                .find(|b| !b.text.trim().is_empty())
                .map(|b| b.text.chars().take(48).collect())
                .unwrap_or_else(|| "(empty)".into()),
        }
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

    pub fn export(&self, fmt: ExportFmt, prefs: &crate::prefs::Prefs) -> String {
        match &self.status {
            DocStatus::Recognizing => String::new(),
            DocStatus::Failed(err) => {
                format!("% {} error: {err}", crate::identity::APP_NAME)
            }
            DocStatus::Ready => export_blocks(&self.blocks, fmt, prefs),
        }
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
            match (fmt, block.kind) {
                (ExportFmt::Markdown, BlockKind::Formula) => {
                    let body = block.text.trim();
                    if body.contains('\n') || body.len() > 48 {
                        out.push_str(&prefs.wrap_block(body));
                    } else {
                        out.push_str(&prefs.wrap_inline(body));
                    }
                }
                (ExportFmt::Markdown, _) => out.push_str(&block.text),
                (ExportFmt::Latex, BlockKind::Formula) => out.push_str(&block.text),
                (ExportFmt::Latex, _) => out.push_str(&escape_latex(&block.text)),
            }
        }
    }
    out
}

fn group_rows(blocks: &[Block]) -> Vec<Vec<&Block>> {
    let mut rows: Vec<Vec<&Block>> = Vec::new();
    for block in blocks {
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

fn escape_latex(text: &str) -> String {
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

    #[test]
    fn markdown_wraps_formulas() {
        let blocks = vec![
            Block::new(
                BlockKind::Text,
                Rect {
                    x: 0,
                    y: 0,
                    w: 10,
                    h: 10,
                },
                "Hello",
            ),
            Block::new(
                BlockKind::Formula,
                Rect {
                    x: 0,
                    y: 20,
                    w: 10,
                    h: 10,
                },
                "E=mc^2",
            ),
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
}
