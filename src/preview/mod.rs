use anyhow::{anyhow, Result};
use ratex_layout::layout_options::LayoutOptions;
use ratex_layout::{layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg_with_color_syntax, SvgColorSyntax, SvgOptions};
use ratex_types::math_style::MathStyle;

use crate::doc::{snip_kind, split_math, Block, BlockKind, BlockRole, MathRun, SnipKind};
use crate::export::CopyRow;
use crate::math::ScriptKind;
use crate::prefs::{BlockDelim, InlineDelim, Prefs};
use crate::table;
use uuid::Uuid;

mod table_layout;

const FONT_SIZE: f64 = 16.0;
const FONT_SIZE_DISPLAY: f64 = 18.0;
const FONT_SIZE_SCRIPT: f64 = 11.5;
const FONT_PAD: f64 = 3.0;
const FONT_PAD_SCRIPT: f64 = 0.0;

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
    TextScript { nucleus: String, glyph: ScriptGlyph },
    Math { svg: SvgMath, tex: String },
}

fn seg_is_visible(seg: &InlineSeg) -> bool {
    match seg {
        InlineSeg::Text(t) => !t.trim().is_empty(),
        InlineSeg::TextScript { .. } | InlineSeg::Math { .. } => true,
    }
}

#[derive(Clone, Debug)]
pub struct Eqno {
    pub raw: String,
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

    pub fn keep_if_selected(self, selected: Option<Uuid>) -> Option<Self> {
        (selected == Some(self.id)).then_some(self)
    }
}

pub(crate) fn should_spawn_derived(ready: bool, cache_hit: bool, busy: bool) -> bool {
    ready && !cache_hit && !busy
}

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

/// GPUI's usvg backend does not paint `rgba(...)` fills, so we emit `rgb(...)`.
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

#[cfg(test)]
pub fn inline_preview(text: &str) -> Vec<InlineSeg> {
    segs_from_text(text, MathStyle::Text, raster_dpr(1.0))
}

fn push_table_block(html: &str, out: &mut Vec<PreviewBlock>, dpr: f64) {
    if let Some(table) = table::parse_html(html) {
        out.push(PreviewBlock::Table(table_layout::table_preview_layout(
            &table, dpr,
        )));
    } else {
        out.push(PreviewBlock::Fallback(html.to_string()));
    }
}

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
mod tests;
