use anyhow::{anyhow, Result};
use ratex_layout::layout_options::LayoutOptions;
use ratex_layout::{layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg_with_color_syntax, SvgColorSyntax, SvgOptions};
use ratex_types::math_style::MathStyle;

use crate::doc::{snip_kind, split_math, Block, BlockKind, MathRun, SnipKind};
use crate::table::{self, Slot, Table};

/// Inline math em size. Display blocks use `FONT_SIZE_DISPLAY`.
const FONT_SIZE: f64 = 16.0;
const FONT_SIZE_DISPLAY: f64 = 18.0;
const FONT_PAD: f64 = 3.0;
/// Extra device pixels in the SVG file. GPUI rasters SVG at 2× the file's
/// width/height, then we display at FONT_SIZE — same trick as toolbar icons.
const RASTER_DPR: f64 = 3.0;

#[derive(Clone, Debug)]
pub struct SvgMath {
    pub svg: String,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug)]
pub enum InlineSeg {
    Text(String),
    Math(SvgMath),
}

#[derive(Clone, Debug)]
pub enum PreviewBlock {
    Paragraph(Vec<InlineSeg>),
    Display(SvgMath),
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

/// Render a LaTeX snippet to SVG via RaTeX (parser → layout → SVG).
///
/// GPUI's usvg backend does not paint `rgba(...)` fills, so we emit `rgb(...)`.
pub fn latex_to_math(latex: &str, style: MathStyle) -> Result<SvgMath> {
    let source = compact_tex(latex.trim());
    if source.is_empty() {
        return Err(anyhow!("empty latex"));
    }
    let display = matches!(style, MathStyle::Display);
    let svg = match render_math(&source, style) {
        Ok(svg) => svg,
        Err(err) => {
            if let Some(inner) = strip_env(&source, "aligned") {
                render_math(&inner, style)?
            } else {
                return Err(err);
            }
        }
    };
    let font = if display {
        FONT_SIZE_DISPLAY
    } else {
        FONT_SIZE
    };
    let (width, height) = svg_pt_size(&svg).unwrap_or((
        font as f32 * 2.0 * RASTER_DPR as f32,
        font as f32 * RASTER_DPR as f32,
    ));
    Ok(SvgMath {
        svg,
        width: (width / RASTER_DPR as f32).max(1.0),
        height: (height / RASTER_DPR as f32).max(1.0),
    })
}

fn render_math(source: &str, style: MathStyle) -> Result<String> {
    let ast = parse(source).map_err(|e| anyhow!("ratex parse: {e}"))?;
    let display = matches!(style, MathStyle::Display);
    let opts = LayoutOptions {
        style,
        ..Default::default()
    };
    let tree = layout(&ast, &opts);
    let list = to_display_list(&tree);
    let dpr = RASTER_DPR;
    let font = if display {
        FONT_SIZE_DISPLAY
    } else {
        FONT_SIZE
    };
    let opts = SvgOptions {
        font_size: font * dpr,
        padding: FONT_PAD * dpr,
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

fn try_math(latex: &str, style: MathStyle) -> Option<SvgMath> {
    latex_to_math(latex, style).ok()
}

fn math_seg(tex: &str, style: MathStyle) -> InlineSeg {
    match try_math(tex, style) {
        Some(math) => InlineSeg::Math(math),
        None => InlineSeg::Text(format!("${tex}$")),
    }
}

fn segs_from_text(text: &str, math: MathStyle) -> Vec<InlineSeg> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut segs = Vec::new();
    for run in split_math(&normalized) {
        match run {
            MathRun::Text(t) if !t.is_empty() => segs.push(InlineSeg::Text(t)),
            MathRun::Inline(tex) | MathRun::Display(tex) => segs.push(math_seg(&tex, math)),
            MathRun::Text(_) => {}
        }
    }
    segs
}

/// Collapse OCR newlines and split `$...$` into text + math for table cells
/// and other mixed strings that are not already `BlockKind::Formula`.
pub fn inline_preview(text: &str) -> Vec<InlineSeg> {
    segs_from_text(text, MathStyle::Text)
}

/// Pixel geometry for the preview pane: origin cells only, spans occupy
/// one rectangle so headers like "Image to Text" line up with R@1/R@5/R@10.
fn table_preview_layout(table: &Table) -> PreviewLayout {
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
        let inner = (w - PAD_X).max(24.0);
        let lines = ((estimate_text_width(text) / inner).ceil() as usize).max(1);
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
                        segs: inline_preview(text),
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
    s.chars()
        .map(|ch| {
            if ch == '$' {
                5.0
            } else if ch.is_ascii() {
                7.2
            } else {
                12.0
            }
        })
        .sum()
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

fn push_table_block(html: &str, out: &mut Vec<PreviewBlock>) {
    if let Some(table) = table::parse_html(html) {
        out.push(PreviewBlock::Table(table_preview_layout(&table)));
    } else {
        out.push(PreviewBlock::Fallback(html.to_string()));
    }
}

/// Build a document-style preview: prose, inline/display math, tables.
pub fn document_preview(blocks: &[Block]) -> Vec<PreviewBlock> {
    let formula_only = matches!(snip_kind(blocks), SnipKind::Formula);
    let mut out = Vec::new();
    let mut para: Vec<InlineSeg> = Vec::new();
    let flush_para = |para: &mut Vec<InlineSeg>, blocks: &mut Vec<PreviewBlock>| {
        if para.iter().any(|s| match s {
            InlineSeg::Text(t) => !t.trim().is_empty(),
            InlineSeg::Math(_) => true,
        }) {
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
                push_table_block(&block.text, &mut out);
            }
            BlockKind::Formula => {
                let (body, unwrapped_display) = crate::math::unwrap_formula(&block.text);
                if formula_only || block.display || unwrapped_display {
                    flush_para(&mut para, &mut out);
                    if let Some(math) = try_math(&body, MathStyle::Display) {
                        out.push(PreviewBlock::Display(math));
                    } else {
                        out.push(PreviewBlock::Fallback(body));
                    }
                } else {
                    para.push(math_seg(&body, MathStyle::Text));
                }
            }
            BlockKind::Text => {
                if table::looks_like_html_table(&block.text) {
                    flush_para(&mut para, &mut out);
                    push_table_block(&block.text, &mut out);
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
                        MathRun::Inline(tex) => para.push(math_seg(&tex, MathStyle::Text)),
                        MathRun::Display(tex) => {
                            flush_para(&mut para, &mut out);
                            if let Some(math) = try_math(&tex, MathStyle::Display) {
                                out.push(PreviewBlock::Display(math));
                            } else {
                                out.push(PreviewBlock::Fallback(format!("$${tex}$$")));
                            }
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
                    segs.iter().any(|s| matches!(s, InlineSeg::Math(_))),
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
            segs.iter().any(|s| matches!(s, InlineSeg::Math(_))),
            "expected $\\dagger$ to become math, got {segs:?}"
        );
        assert!(
            segs.iter().any(|s| match s {
                InlineSeg::Text(t) => t.contains("SARCLIP"),
                InlineSeg::Math(_) => false,
            }),
            "expected surrounding text, got {segs:?}"
        );
    }

    #[test]
    fn raster_svg_is_larger_than_display_box() {
        let math = latex_to_math("E=mc^2", MathStyle::Display).expect("ratex");
        let file_w = super::attr_pt(&math.svg, "width").expect("svg width");
        assert!(
            file_w > math.width * 2.0,
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
            matches!(preview.as_slice(), [PreviewBlock::Display(_)]),
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
            r"Hence $$E=mc^2$$ (1) holds.",
        )]);
        assert!(
            preview.iter().any(|b| matches!(b, PreviewBlock::Display(_))),
            "numbered display math should not collapse to inline, got {preview:?}"
        );
        assert!(
            !preview.iter().any(|b| match b {
                PreviewBlock::Paragraph(segs) => segs.iter().any(|s| match s {
                    InlineSeg::Text(t) => t.contains("(1)"),
                    InlineSeg::Math(_) => false,
                }),
                _ => false,
            }),
            "equation number after $$ should be folded into the math, got {preview:?}"
        );
    }

    #[test]
    fn display_tag_renders() {
        let math = latex_to_math(r"E=mc^2 \tag{11}", MathStyle::Display).expect("tag");
        assert!(math.width > 0.0 && math.height > 0.0);
        let (body, _) = crate::math::unwrap_formula(
            r"{\rm ACC}=\frac{1}{N}I\left[\hat{y}_{i}=y_{i}\right],\\tag{1}\\
\\",
        );
        latex_to_math(&body, MathStyle::Display).expect("repaired tag");
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
            },
            Block::new(BlockKind::Text, r, "as usual."),
        ];
        let preview = document_preview(&blocks);
        assert!(
            preview.iter().any(|b| matches!(b, PreviewBlock::Display(_))),
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
        let lay = table_preview_layout(&t);
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
                InlineSeg::Math(_) => "",
            })
            .collect()
    }
}
