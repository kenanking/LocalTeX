//! Copy / document export keyed by CopyKind.

use crate::doc::{
    unwrap_formula, Block, BlockKind, BlockRole, CopyKind, CopyRow, ExportFmt, MathRun, Rect,
    SnipKind,
};
use crate::math;
use crate::prefs::Prefs;
use crate::table;

impl CopyKind {
    pub fn render(self, blocks: &[Block], prefs: &Prefs) -> String {
        match self {
            CopyKind::MsWord => crate::office::formula_mathml(&formula_body(blocks)),
            CopyKind::Latex => formula_body(blocks),
            CopyKind::MdInline => prefs.wrap_inline(&formula_body(blocks)),
            CopyKind::MdDisplay => {
                let body = formula_body(blocks);
                format!("$$ {body} $$")
            }
            CopyKind::Equation => {
                let body = formula_body(blocks);
                format!("\\begin{{equation}}\n{body}\n\\end{{equation}}")
            }
            CopyKind::LatexTable => render_latex_table_snip(blocks),
            CopyKind::MdTable => export_blocks(blocks, ExportFmt::Markdown, prefs),
            CopyKind::Tsv => table_tsv(blocks).unwrap_or_default(),
            CopyKind::Markdown => export_blocks(blocks, ExportFmt::Markdown, prefs),
            CopyKind::LatexDoc => export_blocks(blocks, ExportFmt::Latex, prefs),
        }
    }
}

fn table_tsv(blocks: &[Block]) -> Option<String> {
    let html = table_html(blocks)?;
    Some(
        table::parse_html(&html)
            .map(|t| t.to_tsv())
            .unwrap_or_default(),
    )
}

pub fn export_blocks(blocks: &[Block], fmt: ExportFmt, prefs: &Prefs) -> String {
    let grouped = group_rows(blocks);
    let mut out = String::new();
    for (row_i, row) in grouped.iter().enumerate() {
        if row_i > 0 {
            if row_is_spaced(&grouped[row_i - 1]) || row_is_spaced(row) {
                out.push_str("\n\n");
            } else {
                out.push('\n');
            }
        }
        for (i, block) in row.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(&export_block(block, fmt, prefs));
        }
    }
    out
}

fn row_is_spaced(row: &[&Block]) -> bool {
    row.iter().any(|b| {
        matches!(b.kind, BlockKind::Table)
            || (b.kind == BlockKind::Text && b.role.interrupts_prose())
    })
}

fn export_block(block: &Block, fmt: ExportFmt, prefs: &Prefs) -> String {
    match (fmt, block.kind) {
        (ExportFmt::Markdown, BlockKind::Formula) => {
            let body = unwrap_formula(&block.text).0;
            if block.display || math::is_display_body(&body) {
                prefs.wrap_block(&body)
            } else {
                prefs.wrap_inline(&body)
            }
        }
        (ExportFmt::Markdown, BlockKind::Table) => table::html_to_markdown(&block.text),
        (ExportFmt::Markdown, BlockKind::Text) => match block.role {
            BlockRole::DocTitle => format!("# {}", emit_text_block(&block.text, fmt, prefs)),
            BlockRole::SectionTitle => format!("## {}", emit_text_block(&block.text, fmt, prefs)),
            _ => escape_md_leading_hashes(&emit_text_block(&block.text, fmt, prefs)),
        },
        (ExportFmt::Latex, BlockKind::Formula) => {
            prefs.wrap_block(unwrap_formula(&block.text).0.trim())
        }
        (ExportFmt::Latex, BlockKind::Table) => table::html_to_latex(&block.text),
        (ExportFmt::Latex, BlockKind::Text) => match block.role {
            BlockRole::DocTitle => {
                format!("\\section*{{{}}}", emit_text_block(&block.text, fmt, prefs))
            }
            BlockRole::SectionTitle => format!(
                "\\subsection*{{{}}}",
                emit_text_block(&block.text, fmt, prefs)
            ),
            _ => emit_text_block(&block.text, fmt, prefs),
        },
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
        if b.kind == BlockKind::Table {
            parts.push(b.text.as_str());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

pub fn copy_rows(blocks: &[Block], prefs: &Prefs) -> Vec<CopyRow> {
    let kind = crate::doc::snip_kind(blocks);
    match kind {
        SnipKind::Formula if formula_body(blocks).is_empty() => return Vec::new(),
        SnipKind::Table if table_html(blocks).is_none() => return Vec::new(),
        _ => {}
    }
    CopyKind::ALL
        .into_iter()
        .filter(|k| k.applies_to(kind))
        .filter_map(|k| {
            let text = k.render(blocks, prefs);
            if k == CopyKind::Tsv && text.is_empty() {
                None
            } else {
                Some(CopyRow { kind: k, text })
            }
        })
        .collect()
}

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

fn emit_text_block(text: &str, fmt: ExportFmt, prefs: &Prefs) -> String {
    let mut out = String::new();
    for run in crate::doc::split_math(text) {
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
        if block.kind == BlockKind::Text && block.role.interrupts_prose() {
            rows.push(vec![block]);
            continue;
        }
        if matches!(block.kind, BlockKind::Table | BlockKind::Formula)
            && (block.kind == BlockKind::Table
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

fn render_latex_table_snip(blocks: &[Block]) -> String {
    let mut caps_before = Vec::new();
    let mut caps_after = Vec::new();
    let mut seen_table = false;
    let mut bodies = Vec::new();
    for b in blocks {
        if b.kind == BlockKind::Text && b.role == BlockRole::Caption {
            let cap = escape_latex(b.text.trim());
            if seen_table {
                caps_after.push(cap);
            } else {
                caps_before.push(cap);
            }
        } else if b.kind == BlockKind::Table {
            seen_table = true;
            bodies.push(table::html_to_latex(&b.text));
        }
    }
    let body = bodies.join("\n");
    if caps_before.is_empty() && caps_after.is_empty() {
        return body;
    }
    let mut s = String::from("\\begin{table}[htbp]\n\\centering\n");
    for c in &caps_before {
        s.push_str(&format!("\\caption{{{c}}}\n"));
    }
    s.push_str(&body);
    for c in &caps_after {
        s.push_str(&format!("\n\\caption{{{c}}}"));
    }
    s.push_str("\n\\end{table}");
    s
}

/// Body/caption lines that start with `#` must not become Markdown headings.
/// Layout titles already use `BlockRole`; this is only the GFM export view.
fn escape_md_leading_hashes(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.trim_start().starts_with('#') {
                line.replacen('#', "\\#", 1)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    #[test]
    fn copy_kinds_have_unique_ids_and_cover_all() {
        let mut ids = std::collections::HashSet::new();
        for kind in CopyKind::ALL {
            assert!(ids.insert(kind.id()), "duplicate {}", kind.id());
            assert!(!kind.label().is_empty());
            assert!(!kind.symbol().is_empty());
        }
        assert_eq!(CopyKind::ALL.len(), 10);
    }

    #[test]
    fn markdown_body_escapes_leading_hash() {
        let prefs = crate::prefs::Prefs::default();
        let blocks = vec![Block::new(
            BlockKind::Text,
            crate::doc::Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            "# not a heading",
        )];
        let md = export_blocks(&blocks, ExportFmt::Markdown, &prefs);
        assert_eq!(md, "\\# not a heading");
        let titled = vec![Block::new(
            BlockKind::Text,
            crate::doc::Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            "Intro",
        )
        .with_role(BlockRole::DocTitle)];
        let md = export_blocks(&titled, ExportFmt::Markdown, &prefs);
        assert_eq!(md, "# Intro");
    }
}
