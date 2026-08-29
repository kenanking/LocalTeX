use anyhow::{anyhow, Result};
use ratex_layout::layout_options::LayoutOptions;
use ratex_layout::{layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg_with_color_syntax, SvgColorSyntax, SvgOptions};
use ratex_types::math_style::MathStyle;

use crate::doc::{snip_kind, split_math, Block, BlockKind, BlockRole, CopyRow, MathRun, SnipKind};
use crate::math::ScriptKind;
use crate::prefs::{BlockDelim, InlineDelim, Prefs};
use crate::table::{self, Slot, Table};
use uuid::Uuid;

/// Inline math em size. Display blocks use `FONT_SIZE_DISPLAY`.
const FONT_SIZE: f64 = 16.0;
const FONT_SIZE_DISPLAY: f64 = 18.0;
const FONT_SIZE_SCRIPT: f64 = 11.5;
const FONT_PAD: f64 = 3.0;
const FONT_PAD_SCRIPT: f64 = 0.0;
/// Extra device pixels in the SVG file relative to the CSS display box.
pub fn raster_dpr(scale: f32) -> f64 {
    (f64::from(scale) * 1.5).clamp(1.0, 2.0)
}

#[derive(Clone, Debug)]
pub struct SvgMath {
    pub svg: String,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug)]
pub struct ScriptGlyph {
    pub svg: SvgMath,
    pub tex: String,
    pub kind: ScriptKind,
}

#[derive(Clone, Debug)]
pub enum InlineSeg {
    Text(String),
    /// Nucleus plus a lone `_{…}` / `^{…}` (or OCR orphan) kept as one wrap atom.
    TextScript {
        nucleus: String,
        glyph: ScriptGlyph,
    },
    Math {
        svg: SvgMath,
        tex: String,
    },
}

fn seg_is_visible(seg: &InlineSeg) -> bool {
    match seg {
        InlineSeg::Text(t) => !t.trim().is_empty(),
        InlineSeg::TextScript { .. } | InlineSeg::Math { .. } => true,
    }
}

#[derive(Clone, Debug)]
pub struct Eqno {
    /// `\tag` payload, e.g. `1`.
    pub raw: String,
    /// Typeset `(n)`. None → GPUI paints `format_eqno(raw)` as text.
    pub math: Option<SvgMath>,
}

impl Eqno {
    fn new(raw: String, dpr: f64) -> Self {
        let math = try_math(&crate::math::format_eqno(&raw), MathStyle::Display, dpr);
        Self { raw, math }
    }

    pub fn width(&self) -> f32 {
        self.math.as_ref().map(|m| m.width).unwrap_or_else(|| {
            crate::math::format_eqno(&self.raw).chars().count() as f32 * 10.0 + 4.0
        })
    }

    pub fn height(&self) -> f32 {
        self.math.as_ref().map(|m| m.height).unwrap_or(18.0)
    }
}

#[derive(Clone, Debug)]
pub enum PreviewBlock {
    Paragraph(Vec<InlineSeg>),
    Heading {
        role: crate::doc::BlockRole,
        segs: Vec<InlineSeg>,
    },
    Caption(Vec<InlineSeg>),
    Display {
        math: SvgMath,
        eqno: Option<Eqno>,
    },
    Table(PreviewLayout),
    Fallback(String),
}

#[derive(Debug, Clone, Default)]
pub struct PreviewLayout {
    pub width: f32,
    pub height: f32,
    pub cells: Vec<PlacedCell>,
}

#[derive(Debug, Clone)]
pub struct PlacedCell {
    pub row: usize,
    pub col: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub segs: Vec<InlineSeg>,
    pub header: bool,
    pub numeric: bool,
    pub colspan: usize,
}

/// Preview + copy rows for one selected document (built off the UI thread).
pub struct DocDerived {
    pub id: Uuid,
    pub revision: u64,
    pub dpr: f64,
    pub inline_delim: InlineDelim,
    pub block_delim: BlockDelim,
    pub preview: Vec<PreviewBlock>,
    pub copy_rows: Vec<CopyRow>,
}

impl DocDerived {
    pub fn matches(&self, id: Uuid, revision: u64, dpr: f64, prefs: &Prefs) -> bool {
        self.id == id
            && self.revision == revision
            && (self.dpr - dpr).abs() < 1e-6
            && self.inline_delim == prefs.inline_delim
            && self.block_delim == prefs.block_delim
    }
}

/// Render a LaTeX snippet to SVG via RaTeX (parser → layout → SVG).
///
/// GPUI's usvg backend does not paint `rgba(...)` fills, so we emit `rgb(...)`.
#[cfg(test)]
pub fn latex_to_math(latex: &str, style: MathStyle) -> Result<SvgMath> {
    latex_to_math_with_dpr(latex, style, raster_dpr(1.0))
}

pub fn latex_to_math_with_dpr(latex: &str, style: MathStyle, dpr: f64) -> Result<SvgMath> {
    let display = matches!(style, MathStyle::Display);
    let font = if display {
        FONT_SIZE_DISPLAY
    } else {
        FONT_SIZE
    };
    latex_to_math_sized(latex, style, dpr, font, FONT_PAD)
}

fn latex_to_math_sized(
    latex: &str,
    style: MathStyle,
    dpr: f64,
    font: f64,
    pad: f64,
) -> Result<SvgMath> {
    let source = compact_tex(latex.trim());
    if source.is_empty() {
        return Err(anyhow!("empty latex"));
    }
    let svg = match render_math(&source, style, dpr, font, pad) {
        Ok(svg) => svg,
        Err(err) => {
            if let Some(inner) = strip_env(&source, "aligned") {
                render_math(&inner, style, dpr, font, pad)?
            } else {
                return Err(err);
            }
        }
    };
    let (width, height) =
        svg_pt_size(&svg).unwrap_or((font as f32 * 2.0 * dpr as f32, font as f32 * dpr as f32));
    Ok(SvgMath {
        svg,
        width: (width / dpr as f32).max(1.0),
        height: (height / dpr as f32).max(1.0),
    })
}

fn render_math(source: &str, style: MathStyle, dpr: f64, font: f64, pad: f64) -> Result<String> {
    let ast = parse(source).map_err(|e| anyhow!("ratex parse: {e}"))?;
    let opts = LayoutOptions {
        style,
        ..Default::default()
    };
    let tree = layout(&ast, &opts);
    let list = to_display_list(&tree);
    let opts = SvgOptions {
        font_size: font * dpr,
        padding: pad * dpr,
        stroke_width: 0.7 * dpr,
        embed_glyphs: true,
        ..Default::default()
    };
    Ok(render_to_svg_with_color_syntax(
        &list,
        &opts,
        SvgColorSyntax::Rgb,
    ))
}

fn svg_pt_size(svg: &str) -> Option<(f32, f32)> {
    let w = attr_pt(svg, "width")?;
    let h = attr_pt(svg, "height")?;
    if w > 0.0 && h > 0.0 {
        Some((w, h))
    } else {
        None
    }
}

fn attr_pt(svg: &str, name: &str) -> Option<f32> {
    let key = format!("{name}=\"");
    let i = svg.find(&key)? + key.len();
    let rest = &svg[i..];
    let end = rest.find('"')?;
    let raw = rest[..end].trim().trim_end_matches("pt");
    raw.parse().ok()
}

/// UniRec (and some LaTeX dumps) insert spaces around tokens (`\mathbf { z }`).
fn compact_tex(s: &str) -> String {
    let mut out = s.split_whitespace().collect::<Vec<_>>().join(" ");
    for (a, b) in [
        (" {", "{"),
        ("{ ", "{"),
        (" }", "}"),
        ("} ", "}"),
        (" _", "_"),
        ("_ ", "_"),
        (" ^", "^"),
        ("^ ", "^"),
    ] {
        while out.contains(a) {
            out = out.replace(a, b);
        }
    }
    out
}

fn strip_env(s: &str, name: &str) -> Option<String> {
    let open = format!("\\begin{{{name}}}");
    let close = format!("\\end{{{name}}}");
    let rest = s.trim().strip_prefix(&open)?.trim();
    let rest = rest.strip_suffix(&close)?.trim();
    let rest = rest.strip_suffix(r"\\").unwrap_or(rest).trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

fn try_math(latex: &str, style: MathStyle, dpr: f64) -> Option<SvgMath> {
    latex_to_math_with_dpr(latex, style, dpr).ok()
}

fn last_attach_char(segs: &[InlineSeg]) -> Option<char> {
    match segs.last()? {
        InlineSeg::Text(t) => t.chars().last(),
        InlineSeg::TextScript { nucleus, .. } => nucleus.chars().last(),
        InlineSeg::Math { .. } => None,
    }
}

fn is_orphan_script_body(tex: &str) -> bool {
    let t = tex.trim();
    if t.is_empty() || t.chars().count() > 24 {
        return false;
    }
    if t.contains('+') || t.contains('=') || t.contains('^') || t.contains('_') || t.contains('-') {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    !lower.contains(r"\frac")
        && !lower.contains(r"\sum")
        && !lower.contains(r"\int")
        && !lower.contains(r"\prod")
        && !lower.contains(r"\sqrt")
}

fn script_glyph_for(prev: Option<char>, tex: &str) -> Option<(ScriptKind, String)> {
    if let Some(kind) = crate::math::script_kind(tex) {
        return Some((kind, crate::math::unwrap_script_body(tex).to_string()));
    }
    if prev.is_some_and(|c| c.is_alphanumeric() || c == ')') && is_orphan_script_body(tex) {
        Some((ScriptKind::Sub, tex.trim().to_string()))
    } else {
        None
    }
}

fn attach_script(segs: &mut Vec<InlineSeg>, glyph: ScriptGlyph) -> bool {
    let Some(InlineSeg::Text(_)) = segs.last() else {
        return false;
    };
    let InlineSeg::Text(nucleus) = segs.pop().expect("last is Text") else {
        return false;
    };
    segs.push(InlineSeg::TextScript { nucleus, glyph });
    true
}

fn math_only(tex: &str, style: MathStyle, dpr: f64) -> InlineSeg {
    match latex_to_math_sized(tex, style, dpr, FONT_SIZE, FONT_PAD) {
        Ok(svg) => InlineSeg::Math {
            svg,
            tex: tex.to_string(),
        },
        Err(_) => InlineSeg::Text(format!("${tex}$")),
    }
}

fn push_inline(segs: &mut Vec<InlineSeg>, tex: &str, style: MathStyle, dpr: f64) {
    if matches!(style, MathStyle::Text) {
        if let Some((kind, body)) = script_glyph_for(last_attach_char(segs), tex) {
            if let Ok(svg) =
                latex_to_math_sized(&body, style, dpr, FONT_SIZE_SCRIPT, FONT_PAD_SCRIPT)
            {
                let glyph = ScriptGlyph {
                    svg,
                    tex: tex.to_string(),
                    kind,
                };
                if attach_script(segs, glyph) {
                    return;
                }
            }
        }
    }
    segs.push(math_only(tex, style, dpr));
}

fn segs_from_text(text: &str, math: MathStyle, dpr: f64) -> Vec<InlineSeg> {
    segs_from_normalized(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        math,
        dpr,
    )
}

fn segs_from_cell(text: &str, dpr: f64) -> Vec<InlineSeg> {
    segs_from_normalized(&table::cell_display_text(text), MathStyle::Text, dpr)
}

fn segs_from_normalized(normalized: &str, math: MathStyle, dpr: f64) -> Vec<InlineSeg> {
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut segs = Vec::new();
    for run in split_math(normalized) {
        match run {
            MathRun::Text(t) if !t.is_empty() => segs.push(InlineSeg::Text(t)),
            MathRun::Inline(tex) | MathRun::Display(tex) => {
                push_inline(&mut segs, &tex, math, dpr);
            }
            MathRun::Text(_) => {}
        }
    }
    segs
}

/// Split inline segs on `\n` so table list cells can stack as lines.
pub(crate) fn segs_lines(segs: &[InlineSeg]) -> Vec<Vec<InlineSeg>> {
    let mut lines = vec![Vec::new()];
    for seg in segs {
        match seg {
            InlineSeg::Text(t) => {
                let mut parts = t.split('\n');
                if let Some(first) = parts.next() {
                    if !first.is_empty() {
                        lines
                            .last_mut()
                            .unwrap()
                            .push(InlineSeg::Text(first.to_string()));
                    }
                }
                for part in parts {
                    lines.push(Vec::new());
                    if !part.is_empty() {
                        lines
                            .last_mut()
                            .unwrap()
                            .push(InlineSeg::Text(part.to_string()));
                    }
                }
            }
            other => lines.last_mut().unwrap().push(other.clone()),
        }
    }
    if lines.len() == 1 && lines[0].is_empty() {
        lines.clear();
    }
    lines
}

/// Collapse OCR newlines and split `$...$` into text + math for table cells
/// and other mixed strings that are not already `BlockKind::Formula`.
#[cfg(test)]
pub fn inline_preview(text: &str) -> Vec<InlineSeg> {
    segs_from_text(text, MathStyle::Text, raster_dpr(1.0))
}

/// Pixel geometry for the preview pane: origin cells only, spans occupy
/// one rectangle so headers like "Image to Text" line up with R@1/R@5/R@10.
fn table_preview_layout(table: &Table, dpr: f64) -> PreviewLayout {
    const PAD_X: f32 = 16.0;
    const PAD_Y: f32 = 8.0;
    const MIN_COL: f32 = 56.0;
    const MAX_COL: f32 = 220.0;
    const LINE: f32 = 18.0;
    const MIN_ROW: f32 = 28.0;

    let slots = table.slot_grid();
    if slots.is_empty() {
        return PreviewLayout::default();
    }
    let cols = slots.iter().map(|r| r.len()).max().unwrap_or(0);
    let rows = slots.len();
    if cols == 0 {
        return PreviewLayout::default();
    }

    let mut col_w = vec![MIN_COL; cols];
    for row in &slots {
        for (c, slot) in row.iter().enumerate() {
            if let Slot::Origin { text, colspan, .. } = slot {
                if *colspan <= 1 {
                    col_w[c] = col_w[c].max(estimate_text_width(text) + PAD_X).min(MAX_COL);
                }
            }
        }
    }
    for row in &slots {
        let mut c = 0usize;
        while c < row.len() {
            match &row[c] {
                Slot::Origin { text, colspan, .. } if *colspan > 1 => {
                    let span = (*colspan).min(cols.saturating_sub(c)).max(1);
                    let need = (estimate_text_width(text) + PAD_X).min(MAX_COL * span as f32);
                    let have: f32 = col_w[c..c + span].iter().sum();
                    if need > have {
                        let extra = (need - have) / span as f32;
                        for slot_w in &mut col_w[c..c + span] {
                            *slot_w += extra;
                        }
                    }
                    c += span;
                }
                Slot::Origin { colspan, .. } => c += (*colspan).max(1),
                _ => c += 1,
            }
        }
    }

    let content_h = |text: &str, w: f32| {
        let display = table::cell_display_text(text);
        if table::looks_like_list(&display) {
            let n = display
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count()
                .max(1);
            return (n as f32) * LINE + PAD_Y;
        }
        let inner = (w - PAD_X).max(24.0);
        let lines = ((estimate_text_width(&display) / inner).ceil() as usize).max(1);
        (lines as f32) * LINE + PAD_Y
    };

    let mut row_h = vec![MIN_ROW; rows];
    for (r, row) in slots.iter().enumerate() {
        let mut c = 0usize;
        while c < row.len() {
            match &row[c] {
                Slot::Origin {
                    text,
                    rowspan,
                    colspan,
                } if *rowspan <= 1 => {
                    let span = (*colspan).min(cols.saturating_sub(c)).max(1);
                    let w: f32 = col_w[c..c + span].iter().sum();
                    row_h[r] = row_h[r].max(content_h(text, w));
                    c += span;
                }
                Slot::Origin { colspan, .. } => c += (*colspan).max(1),
                _ => c += 1,
            }
        }
    }
    for (r, row) in slots.iter().enumerate() {
        let mut c = 0usize;
        while c < row.len() {
            match &row[c] {
                Slot::Origin {
                    text,
                    rowspan,
                    colspan,
                } if *rowspan > 1 => {
                    let cs = (*colspan).min(cols.saturating_sub(c)).max(1);
                    let rs = (*rowspan).min(rows.saturating_sub(r)).max(1);
                    let w: f32 = col_w[c..c + cs].iter().sum();
                    let need = content_h(text, w);
                    let have: f32 = row_h[r..r + rs].iter().sum();
                    if need > have {
                        row_h[r + rs - 1] += need - have;
                    }
                    c += cs;
                }
                Slot::Origin { colspan, .. } => c += (*colspan).max(1),
                _ => c += 1,
            }
        }
    }

    // Spanned first row (rowspan/colspan) ⇒ paint the first two rows as header.
    let grouped = slots.first().is_some_and(|row| {
        row.iter().any(|s| {
            matches!(
                s,
                Slot::Origin {
                    colspan,
                    rowspan,
                    ..
                } if *colspan > 1 || *rowspan > 1
            )
        })
    });
    let header_rows = if grouped { 2.min(rows) } else { 1.min(rows) };

    let mut xs = vec![0.0; cols];
    for i in 1..cols {
        xs[i] = xs[i - 1] + col_w[i - 1];
    }
    let mut ys = vec![0.0; rows];
    for i in 1..rows {
        ys[i] = ys[i - 1] + row_h[i - 1];
    }

    let mut cells = Vec::new();
    for (r, row) in slots.iter().enumerate() {
        let mut c = 0usize;
        while c < row.len() {
            match &row[c] {
                Slot::Origin {
                    text,
                    rowspan,
                    colspan,
                } => {
                    let cs = (*colspan).min(cols.saturating_sub(c)).max(1);
                    let rs = (*rowspan).min(rows.saturating_sub(r)).max(1);
                    let w: f32 = col_w[c..c + cs].iter().sum();
                    let h: f32 = row_h[r..r + rs].iter().sum();
                    cells.push(PlacedCell {
                        row: r,
                        col: c,
                        x: xs[c],
                        y: ys[r],
                        w,
                        h,
                        segs: segs_from_cell(text, dpr),
                        header: r < header_rows,
                        numeric: looks_numeric(text),
                        colspan: cs,
                    });
                    c += cs;
                }
                _ => c += 1,
            }
        }
    }

    PreviewLayout {
        width: col_w.iter().sum(),
        height: row_h.iter().sum(),
        cells,
    }
}

fn estimate_text_width(s: &str) -> f32 {
    s.lines()
        .map(|line| {
            line.chars()
                .map(|ch| {
                    if ch == '$' {
                        5.0
                    } else if ch.is_ascii() {
                        7.2
                    } else {
                        12.0
                    }
                })
                .sum::<f32>()
        })
        .fold(0.0_f32, f32::max)
}

fn looks_numeric(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    let mut digit = false;
    for ch in t.chars() {
        if ch.is_ascii_digit() {
            digit = true;
        } else if !matches!(ch, '.' | '+' | '-' | '%' | ' ') {
            return false;
        }
    }
    digit
}

fn push_table_block(html: &str, out: &mut Vec<PreviewBlock>, dpr: f64) {
    if let Some(table) = table::parse_html(html) {
        out.push(PreviewBlock::Table(table_preview_layout(&table, dpr)));
    } else {
        out.push(PreviewBlock::Fallback(html.to_string()));
    }
}

/// Typeset display math with `\tag` stripped. The number is a GPUI sidecar;
/// RaTeX only sees the body. Fallback keeps the original tagged source.
fn push_display(tex: &str, fallback: &str, dpr: f64, out: &mut Vec<PreviewBlock>) {
    let canon = crate::math::canonicalize_tex(tex);
    let (body, tags) = crate::math::split_display_tag(&canon);
    let typeset = if body.is_empty() {
        canon.as_str()
    } else {
        body.as_str()
    };
    match try_math(typeset, MathStyle::Display, dpr) {
        Some(math) => {
            let eqno = tags
                .last()
                .filter(|s| !s.is_empty())
                .cloned()
                .map(|raw| Eqno::new(raw, dpr));
            out.push(PreviewBlock::Display { math, eqno });
        }
        None => out.push(PreviewBlock::Fallback(fallback.to_string())),
    }
}

/// Build a document-style preview: prose, inline/display math, tables.
#[cfg(test)]
pub fn document_preview(blocks: &[Block]) -> Vec<PreviewBlock> {
    document_preview_with_dpr(blocks, raster_dpr(1.0))
}

pub fn document_preview_with_dpr(blocks: &[Block], dpr: f64) -> Vec<PreviewBlock> {
    let formula_only = matches!(snip_kind(blocks), SnipKind::Formula);
    let mut out = Vec::new();
    let mut para: Vec<InlineSeg> = Vec::new();
    let flush_para = |para: &mut Vec<InlineSeg>, blocks: &mut Vec<PreviewBlock>| {
        if para.iter().any(seg_is_visible) {
            blocks.push(PreviewBlock::Paragraph(std::mem::take(para)));
        } else {
            para.clear();
        }
    };

    for block in blocks {
        if block.text.trim().is_empty() {
            continue;
        }
        match block.kind {
            BlockKind::Table => {
                flush_para(&mut para, &mut out);
                push_table_block(&block.text, &mut out, dpr);
            }
            BlockKind::Formula => {
                let (body, unwrapped_display) = crate::math::unwrap_formula(&block.text);
                if formula_only || block.display || unwrapped_display {
                    flush_para(&mut para, &mut out);
                    push_display(&body, &body, dpr, &mut out);
                } else {
                    push_inline(&mut para, &body, MathStyle::Text, dpr);
                }
            }
            BlockKind::Text => {
                if table::looks_like_html_table(&block.text) {
                    flush_para(&mut para, &mut out);
                    push_table_block(&block.text, &mut out, dpr);
                    continue;
                }
                if block.role.interrupts_prose() {
                    flush_para(&mut para, &mut out);
                    let segs = segs_from_text(&block.text, MathStyle::Text, dpr);
                    if segs.iter().any(seg_is_visible) {
                        match block.role {
                            BlockRole::DocTitle | BlockRole::SectionTitle => {
                                out.push(PreviewBlock::Heading {
                                    role: block.role,
                                    segs,
                                })
                            }
                            BlockRole::Caption => out.push(PreviewBlock::Caption(segs)),
                            BlockRole::Body => {}
                        }
                    }
                    continue;
                }
                for run in split_math(&block.text) {
                    match run {
                        MathRun::Text(t) => {
                            if t.contains('\n') {
                                let parts: Vec<&str> = t.split('\n').collect();
                                for (i, part) in parts.iter().enumerate() {
                                    if !part.is_empty() {
                                        para.push(InlineSeg::Text((*part).to_string()));
                                    }
                                    if i + 1 < parts.len() {
                                        flush_para(&mut para, &mut out);
                                    }
                                }
                            } else if !t.is_empty() {
                                para.push(InlineSeg::Text(t));
                            }
                        }
                        MathRun::Inline(tex) => {
                            push_inline(&mut para, &tex, MathStyle::Text, dpr);
                        }
                        MathRun::Display(tex) => {
                            flush_para(&mut para, &mut out);
                            push_display(&tex, &format!("$${tex}$$"), dpr, &mut out);
                        }
                    }
                }
            }
        }
    }
    flush_para(&mut para, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_simple_formula_svg() {
        let math = latex_to_math("E=mc^2", MathStyle::Display).expect("ratex");
        let svg = &math.svg;
        assert!(
            svg.contains("<svg"),
            "expected svg, got: {}",
            &svg[..svg.len().min(200)]
        );
        assert!(
            !svg.contains("rgba("),
            "usvg cannot paint rgba() fills: {}",
            &svg[..svg.len().min(200)]
        );
        assert!(svg.contains("rgb("));
        let math = latex_to_math("E=mc^2", MathStyle::Display).expect("size");
        assert!(math.width > 0.0 && math.height > 0.0);
        assert!(
            math.height < 48.0,
            "body-size math should not be huge, got h={}",
            math.height
        );
    }

    #[test]
    fn renders_spaced_tokens_aligned() {
        let compact = r"\begin{aligned} {\mathbf{z}}_{v}^{i}=f_{v}^{(0)}({\mathbf{I}}_{i}^{\mathsf{opt}}), \\ \end{aligned}";
        let svg = latex_to_math(compact, MathStyle::Display)
            .expect("compact aligned")
            .svg;
        assert!(svg.contains("<svg"));
        assert!(!svg.contains("rgba("));
        let spaced = r"\begin{aligned} {\mathbf { z } _ { v } ^ { i } } = f _ { v } ^ { ( 0 ) } ( \mathbf { I } _ { i } ^ { \mathsf { o p t } } ) , \\ \end{aligned}";
        latex_to_math(spaced, MathStyle::Display).expect("spaced aligned");
        assert_eq!(
            compact_tex(r"\mathbf { z } _ { v } ^ { i }"),
            r"\mathbf{z}_{v}^{i}"
        );
    }

    #[test]
    fn mixed_text_preview_has_paragraph() {
        let blocks = vec![Block::new(
            BlockKind::Text,
            crate::doc::Rect {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
            },
            "Because $c$ and $1-c$.",
        )];
        let preview = document_preview(&blocks);
        assert!(
            preview
                .iter()
                .any(|b| matches!(b, PreviewBlock::Paragraph(_))),
            "expected a paragraph"
        );
    }

    #[test]
    fn title_and_body_are_separate_preview_blocks() {
        let r = crate::doc::Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        let blocks = vec![
            Block::new(BlockKind::Text, r, "Introduction")
                .with_role(crate::doc::BlockRole::DocTitle),
            Block::new(BlockKind::Text, r, "The method works."),
        ];
        let preview = document_preview(&blocks);
        assert!(
            matches!(
                preview.as_slice(),
                [
                    PreviewBlock::Heading {
                        role: crate::doc::BlockRole::DocTitle,
                        ..
                    },
                    PreviewBlock::Paragraph(_)
                ]
            ),
            "title must not weld to body, got {preview:?}"
        );
    }

    #[test]
    fn short_formula_block_stays_inline_with_neighbors() {
        let r = crate::doc::Rect {
            x: 0,
            y: 0,
            w: 40,
            h: 16,
        };
        let blocks = vec![
            Block::new(BlockKind::Text, r, "Let "),
            Block::new(BlockKind::Formula, r, "E=mc^2"),
            Block::new(BlockKind::Text, r, " denote energy."),
        ];
        let preview = document_preview(&blocks);
        assert_eq!(
            preview.len(),
            1,
            "expected a single paragraph, got {preview:?}"
        );
        match &preview[0] {
            PreviewBlock::Paragraph(segs) => {
                assert!(
                    segs.iter().any(|s| matches!(s, InlineSeg::Math { .. })),
                    "expected inline math"
                );
            }
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn table_cell_newlines_still_render_inline_math() {
        let segs = inline_preview("SARCLIP\n$\n\\dagger\n$");
        assert!(
            segs.iter().any(|s| matches!(s, InlineSeg::Math { .. })),
            "expected $\\dagger$ to become math, got {segs:?}"
        );
        assert!(
            segs.iter().any(|s| match s {
                InlineSeg::Text(t) => t.contains("SARCLIP"),
                _ => false,
            }),
            "expected SARCLIP text, got {segs:?}"
        );
    }

    #[test]
    fn subscript_formula_is_marked_and_smaller() {
        let segs = inline_preview(r"H$_2$O");
        let sub = segs.iter().find_map(|s| match s {
            InlineSeg::TextScript { glyph, .. } if glyph.kind == ScriptKind::Sub => {
                Some((glyph.svg.height, glyph.tex.as_str()))
            }
            _ => None,
        });
        let Some((sub_h, tex)) = sub else {
            panic!("expected a TextScript subscript, got {segs:?}");
        };
        assert!(tex.contains('2'), "subscript tex {tex}");
        let full = inline_preview("$x$");
        let full_h = full
            .iter()
            .find_map(|s| match s {
                InlineSeg::Math { svg, .. } => Some(svg.height),
                _ => None,
            })
            .expect("inline x");
        assert!(
            sub_h < full_h,
            "subscript {sub_h} should be shorter than inline {full_h}"
        );
    }

    #[test]
    fn orphan_digit_after_letter_is_subscript() {
        let segs = inline_preview("H$2$O");
        assert!(
            segs.iter().any(|s| matches!(
                s,
                InlineSeg::TextScript {
                    glyph: ScriptGlyph {
                        kind: ScriptKind::Sub,
                        ..
                    },
                    ..
                }
            )),
            "expected orphan $2$ after H to be a TextScript subscript, got {segs:?}"
        );
    }

    #[test]
    fn empty_nucleus_script_from_ocr_is_subscript() {
        let segs = inline_preview(r"AP${}_{50}$, and");
        assert!(
            segs.iter().any(|s| matches!(
                s,
                InlineSeg::TextScript {
                    glyph: ScriptGlyph {
                        kind: ScriptKind::Sub,
                        ..
                    },
                    ..
                }
            )),
            "expected {{}}_{{50}} after AP to be a TextScript subscript, got {segs:?}"
        );
    }

    #[test]
    fn document_preview_marks_empty_nucleus_subscripts() {
        let r = crate::doc::Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        let blocks = vec![Block::new(
            BlockKind::Text,
            r,
            r"reaching 36.02 mAP, 69.60 AP${}_{50}$, and 33.80 AP${}_{75}$.",
        )];
        let preview = document_preview(&blocks);
        let segs = match &preview[0] {
            PreviewBlock::Paragraph(segs) => segs,
            other => panic!("expected paragraph, got {other:?}"),
        };
        let subs: Vec<&str> = segs
            .iter()
            .filter_map(|s| match s {
                InlineSeg::TextScript {
                    glyph:
                        ScriptGlyph {
                            kind: ScriptKind::Sub,
                            tex,
                            ..
                        },
                    ..
                } => Some(tex.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(subs, ["{}_{50}", "{}_{75}"]);
        assert!(
            segs.iter().any(|s| matches!(
                s,
                InlineSeg::TextScript { nucleus, .. } if nucleus.ends_with("AP")
            )),
            "script must attach to the AP text run, got {segs:?}"
        );
    }

    #[test]
    fn next_script_keeps_leading_space_on_its_nucleus() {
        let segs = inline_preview(r"AP${}_{50}$ by 2.65%, and AP${}_{75}$");
        let nuclei: Vec<&str> = segs
            .iter()
            .filter_map(|s| match s {
                InlineSeg::TextScript { nucleus, .. } => Some(nucleus.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            nuclei.len(),
            2,
            "expected two TextScript atoms, got {segs:?}"
        );
        assert!(
            nuclei[1].starts_with(' '),
            "space after the first script stays on the next nucleus, got {nuclei:?}"
        );
    }

    #[test]
    fn script_glyph_for_orphans_after_letters() {
        assert_eq!(
            script_glyph_for(Some('H'), "2"),
            Some((ScriptKind::Sub, "2".into()))
        );
        assert_eq!(
            script_glyph_for(Some('H'), "_2"),
            Some((ScriptKind::Sub, "2".into()))
        );
        assert_eq!(script_glyph_for(Some(' '), "x"), None);
        assert_eq!(script_glyph_for(None, "2"), None);
        assert_eq!(
            script_glyph_for(Some('n'), "^2"),
            Some((ScriptKind::Super, "2".into()))
        );
    }

    #[test]
    fn orphan_script_body_is_a_short_nucleus() {
        assert!(is_orphan_script_body("2"));
        assert!(is_orphan_script_body("ij"));
        assert!(is_orphan_script_body(r"\mathrm{max}"));
        assert!(!is_orphan_script_body("x+y"));
        assert!(!is_orphan_script_body(r"\frac{1}{2}"));
        assert!(!is_orphan_script_body("E=mc^2"));
    }

    #[test]
    fn section_title_is_heading_not_paragraph() {
        let r = crate::doc::Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        let blocks = vec![
            Block::new(BlockKind::Text, r, "Method").with_role(crate::doc::BlockRole::SectionTitle),
            Block::new(BlockKind::Text, r, "Details."),
        ];
        let preview = document_preview(&blocks);
        assert!(
            matches!(
                preview.as_slice(),
                [
                    PreviewBlock::Heading {
                        role: crate::doc::BlockRole::SectionTitle,
                        ..
                    },
                    PreviewBlock::Paragraph(_)
                ]
            ),
            "section title must not weld to body, got {preview:?}"
        );
    }

    #[test]
    fn table_list_cell_keeps_item_breaks() {
        let html = "<table><tr><td>- one\n- two</td></tr></table>";
        let table = table::parse_html(html).unwrap();
        let layout = table_preview_layout(&table, raster_dpr(1.0));
        assert_eq!(layout.cells.len(), 1);
        let lines = segs_lines(&layout.cells[0].segs);
        assert_eq!(lines.len(), 2, "got {lines:?}");
    }

    #[test]
    fn raster_svg_is_larger_than_display_box() {
        let math = latex_to_math("E=mc^2", MathStyle::Display).expect("ratex");
        let file_w = super::attr_pt(&math.svg, "width").expect("svg width");
        assert!(
            file_w > math.width * raster_dpr(1.0) as f32 * 0.9,
            "SVG file should be DPR-scaled, file={file_w} display={}",
            math.width
        );
    }

    #[test]
    fn lone_formula_snip_is_display() {
        let r = crate::doc::Rect {
            x: 0,
            y: 0,
            w: 40,
            h: 16,
        };
        let preview = document_preview(&[Block::new(BlockKind::Formula, r, "E=mc^2")]);
        assert!(
            matches!(preview.as_slice(), [PreviewBlock::Display { .. }]),
            "lone formula snip should be a centered display block, got {preview:?}"
        );
    }

    #[test]
    fn numbered_display_in_prose_stays_display() {
        let r = crate::doc::Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 24,
        };
        let preview = document_preview(&[Block::new(
            BlockKind::Text,
            r,
            r"Hence $$E=mc^2 \tag{1}$$ holds.",
        )]);
        let tagged = preview.iter().find_map(|b| match b {
            PreviewBlock::Display { eqno, .. } => eqno.as_ref().map(|e| e.raw.as_str()),
            _ => None,
        });
        assert_eq!(
            tagged,
            Some("1"),
            "tagged display math should not collapse to inline, got {preview:?}"
        );
    }

    #[test]
    fn display_tag_is_sidecar_not_typeset() {
        let r = crate::doc::Rect {
            x: 0,
            y: 0,
            w: 40,
            h: 16,
        };
        let preview =
            document_preview(&[Block::new(BlockKind::Formula, r, r"$$E=mc^2 \tag{11}$$")]);
        match preview.as_slice() {
            [PreviewBlock::Display {
                eqno: Some(eqno), ..
            }] => {
                assert_eq!(eqno.raw, "11");
                assert!(
                    eqno.math.is_some(),
                    "eqno should typeset as display math, not a muted UI label"
                );
            }
            other => panic!("expected tagged display, got {other:?}"),
        }
        let (body, tags) = crate::math::split_display_tag(r"E=mc^2 \tag{11}");
        assert_eq!(tags, vec!["11".to_string()]);
        latex_to_math(&body, MathStyle::Display).expect("body without tag");
        let (body, _) = crate::math::unwrap_formula(
            r"{\rm ACC}=\frac{1}{N}I\left[\hat{y}_{i}=y_{i}\right],\\tag{1}\\
\\",
        );
        let (body, tags) = crate::math::split_display_tag(&body);
        assert_eq!(tags, vec!["1".to_string()]);
        latex_to_math(&body, MathStyle::Display).expect("repaired tag body");
    }

    #[test]
    fn display_formula_block_stays_display_with_neighbors() {
        let r = crate::doc::Rect {
            x: 0,
            y: 0,
            w: 40,
            h: 16,
        };
        let blocks = vec![
            Block::new(BlockKind::Text, r, "We have"),
            Block {
                kind: BlockKind::Formula,
                bbox: r,
                text: "E=mc^2".into(),
                display: true,
                role: crate::doc::BlockRole::Body,
            },
            Block::new(BlockKind::Text, r, "as usual."),
        ];
        let preview = document_preview(&blocks);
        assert!(
            preview
                .iter()
                .any(|b| matches!(b, PreviewBlock::Display { .. })),
            "layout display_formula should stay display even when short, got {preview:?}"
        );
    }

    #[test]
    fn display_style_sum_is_taller_than_text_style() {
        let tex = r"\sum_{i=1}^{n} i";
        let display = latex_to_math(tex, MathStyle::Display).expect("display");
        let inline = latex_to_math(tex, MathStyle::Text).expect("text");
        assert!(
            display.height > inline.height,
            "display limits should sit above/below, h_d={} h_t={}",
            display.height,
            inline.height
        );
    }

    #[test]
    fn preview_layout_merges_header_spans() {
        let html = r#"<table>
<tr>
<th rowspan="2">Method</th>
<th rowspan="2">Image Backbone</th>
<th colspan="3">Image to Text</th>
</tr>
<tr><th>R@1</th><th>R@5</th><th>R@10</th></tr>
<tr><td>OpenCLIP</td><td>ViT-B/32</td><td>11.1</td><td>22.2</td><td>33.3</td></tr>
</table>"#;
        let t = table::parse_html(html).unwrap();
        let lay = table_preview_layout(&t, raster_dpr(1.0));
        let method = lay
            .cells
            .iter()
            .find(|c| cell_plain(c) == "Method")
            .expect("Method");
        let backbone = lay
            .cells
            .iter()
            .find(|c| cell_plain(c) == "Image Backbone")
            .expect("Backbone");
        let group = lay
            .cells
            .iter()
            .find(|c| cell_plain(c) == "Image to Text")
            .expect("group");
        let r1 = lay
            .cells
            .iter()
            .find(|c| cell_plain(c) == "R@1")
            .expect("R@1");
        let r5 = lay
            .cells
            .iter()
            .find(|c| cell_plain(c) == "R@5")
            .expect("R@5");
        let r10 = lay
            .cells
            .iter()
            .find(|c| cell_plain(c) == "R@10")
            .expect("R@10");
        assert!(
            !lay.cells.iter().any(|c| c.col == 0 && c.row == 1),
            "rowspan must not leave an empty cell under Method"
        );
        assert!(
            (r1.x - group.x).abs() < 0.51,
            "R@1 left {} vs group {}",
            r1.x,
            group.x
        );
        assert!(
            (group.w - (r1.w + r5.w + r10.w)).abs() < 0.51,
            "group width {} vs R@ sum {}",
            group.w,
            r1.w + r5.w + r10.w
        );
        assert!(
            method.h > r1.h + 1.0,
            "Method should span both header rows ({} vs {})",
            method.h,
            r1.h
        );
        assert!(
            backbone.w > 88.0,
            "content-sized columns, not a uniform 88px: {}",
            backbone.w
        );
        assert!(
            method.header
                && r1.header
                && !lay
                    .cells
                    .iter()
                    .any(|c| cell_plain(c) == "OpenCLIP" && c.header)
        );
        assert!(lay
            .cells
            .iter()
            .find(|c| cell_plain(c) == "11.1")
            .is_some_and(|c| c.numeric));
    }

    fn cell_plain(c: &PlacedCell) -> String {
        c.segs
            .iter()
            .map(|s| match s {
                InlineSeg::Text(t) => t.as_str(),
                InlineSeg::TextScript { nucleus, .. } => nucleus.as_str(),
                InlineSeg::Math { .. } => "",
            })
            .collect()
    }
}
