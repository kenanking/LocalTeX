//! Copy / document export. New formats implement [`Exporter`].

use crate::doc::{
    unwrap_formula, Block, BlockKind, CopyKind, CopyRow, ExportFmt, MathRun, Rect, SnipKind,
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

struct LatexPlain;
struct MdInline;
struct MdDisplay;
struct EquationEnv;
struct LatexTable;
struct MdTable;
struct TsvTable;
struct MarkdownDoc;
struct LatexDoc;

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
        table_exports(blocks).map(|e| e.tex).unwrap_or_default()
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
    fn render(&self, blocks: &[Block], _: &Prefs) -> String {
        table_exports(blocks).map(|e| e.md).unwrap_or_default()
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
        table_exports(blocks).map(|e| e.tsv).unwrap_or_default()
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

const EXPORTERS: [&dyn Exporter; 9] = [
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

struct TableOut {
    md: String,
    tex: String,
    tsv: String,
}

fn table_exports(blocks: &[Block]) -> Option<TableOut> {
    let html = table_html(blocks)?;
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
    Some(TableOut { md, tex, tsv })
}

pub fn export_blocks(blocks: &[Block], fmt: ExportFmt, prefs: &Prefs) -> String {
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

fn export_block(block: &Block, fmt: ExportFmt, prefs: &Prefs) -> String {
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
    if table::looks_like_html_table(text) {
        return match fmt {
            ExportFmt::Latex => table::html_to_latex(text),
            ExportFmt::Markdown => table::html_to_markdown(text),
        };
    }
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

    #[test]
    fn exporters_have_unique_ids() {
        let mut ids = std::collections::HashSet::new();
        for exp in exporters() {
            assert!(ids.insert(exp.id()), "duplicate {}", exp.id());
            assert!(!exp.label().is_empty());
        }
    }
}
