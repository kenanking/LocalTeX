//! Copy / document export. New formats implement [`Exporter`].

use crate::doc::{
    unwrap_formula, Block, BlockKind, BlockRole, CopyKind, CopyRow, ExportFmt, MathRun, Rect,
    SnipKind,
};
use crate::math;
use crate::prefs::Prefs;
use crate::table;

pub trait Exporter: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn applies_to(&self, kind: SnipKind) -> bool;
    fn copy_kind(&self) -> CopyKind;
    fn render(&self, blocks: &[Block], prefs: &Prefs) -> String;
}

struct MsWord;
struct LatexPlain;
struct MdInline;
struct MdDisplay;
struct EquationEnv;
struct LatexTable;
struct MdTable;
struct TsvTable;
struct MarkdownDoc;
struct LatexDoc;

impl Exporter for MsWord {
    fn id(&self) -> &'static str {
        "ms_word"
    }
    fn label(&self) -> &'static str {
        CopyKind::MsWord.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Formula
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::MsWord
    }
    fn render(&self, blocks: &[Block], _: &Prefs) -> String {
        crate::office::formula_mathml(&formula_body(blocks))
    }
}

impl Exporter for LatexPlain {
    fn id(&self) -> &'static str {
        "latex"
    }
    fn label(&self) -> &'static str {
        CopyKind::Latex.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Formula
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::Latex
    }
    fn render(&self, blocks: &[Block], _: &Prefs) -> String {
        formula_body(blocks)
    }
}

impl Exporter for MdInline {
    fn id(&self) -> &'static str {
        "md_inline"
    }
    fn label(&self) -> &'static str {
        CopyKind::MdInline.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Formula
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::MdInline
    }
    fn render(&self, blocks: &[Block], prefs: &Prefs) -> String {
        prefs.wrap_inline(&formula_body(blocks))
    }
}

impl Exporter for MdDisplay {
    fn id(&self) -> &'static str {
        "md_display"
    }
    fn label(&self) -> &'static str {
        CopyKind::MdDisplay.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Formula
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::MdDisplay
    }
    fn render(&self, blocks: &[Block], _: &Prefs) -> String {
        let body = formula_body(blocks);
        format!("$$ {body} $$")
    }
}

impl Exporter for EquationEnv {
    fn id(&self) -> &'static str {
        "equation"
    }
    fn label(&self) -> &'static str {
        CopyKind::Equation.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Formula
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::Equation
    }
    fn render(&self, blocks: &[Block], _: &Prefs) -> String {
        let body = formula_body(blocks);
        format!("\\begin{{equation}}\n{body}\n\\end{{equation}}")
    }
}

impl Exporter for LatexTable {
    fn id(&self) -> &'static str {
        "latex_table"
    }
    fn label(&self) -> &'static str {
        CopyKind::LatexTable.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Table
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::LatexTable
    }
    fn render(&self, blocks: &[Block], _: &Prefs) -> String {
        render_latex_table_snip(blocks)
    }
}

impl Exporter for MdTable {
    fn id(&self) -> &'static str {
        "md_table"
    }
    fn label(&self) -> &'static str {
        CopyKind::MdTable.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Table
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::MdTable
    }
    fn render(&self, blocks: &[Block], prefs: &Prefs) -> String {
        export_blocks(blocks, ExportFmt::Markdown, prefs)
    }
}

impl Exporter for TsvTable {
    fn id(&self) -> &'static str {
        "tsv"
    }
    fn label(&self) -> &'static str {
        CopyKind::Tsv.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Table
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::Tsv
    }
    fn render(&self, blocks: &[Block], _: &Prefs) -> String {
        table_tsv(blocks).unwrap_or_default()
    }
}

impl Exporter for MarkdownDoc {
    fn id(&self) -> &'static str {
        "markdown"
    }
    fn label(&self) -> &'static str {
        CopyKind::Markdown.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Mixed
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::Markdown
    }
    fn render(&self, blocks: &[Block], prefs: &Prefs) -> String {
        export_blocks(blocks, ExportFmt::Markdown, prefs)
    }
}

impl Exporter for LatexDoc {
    fn id(&self) -> &'static str {
        "latex_doc"
    }
    fn label(&self) -> &'static str {
        CopyKind::LatexDoc.label()
    }
    fn applies_to(&self, kind: SnipKind) -> bool {
        kind == SnipKind::Mixed
    }
    fn copy_kind(&self) -> CopyKind {
        CopyKind::LatexDoc
    }
    fn render(&self, blocks: &[Block], prefs: &Prefs) -> String {
        export_blocks(blocks, ExportFmt::Latex, prefs)
    }
}

const EXPORTERS: [&dyn Exporter; 10] = [
    &MsWord,
    &LatexPlain,
    &MdInline,
    &MdDisplay,
    &EquationEnv,
    &LatexTable,
    &MdTable,
    &TsvTable,
    &MarkdownDoc,
    &LatexDoc,
];

pub fn exporters() -> &'static [&'static dyn Exporter] {
    &EXPORTERS
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
    exporters()
        .iter()
        .copied()
        .filter(|exp| exp.applies_to(kind))
        .filter_map(|exp| {
            let text = exp.render(blocks, prefs);
            if exp.copy_kind() == CopyKind::Tsv && text.is_empty() {
                None
            } else {
                Some(CopyRow {
                    kind: exp.copy_kind(),
                    text,
                    exporter_id: exp.id(),
                    label: exp.label(),
                })
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
    fn exporters_have_unique_ids() {
        let mut ids = std::collections::HashSet::new();
        for exp in exporters() {
            assert!(ids.insert(exp.id()), "duplicate {}", exp.id());
            assert!(!exp.label().is_empty());
        }
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
