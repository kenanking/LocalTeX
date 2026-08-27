use anyhow::{anyhow, Result};
use ratex_layout::layout_options::LayoutOptions;
use ratex_layout::{layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg_with_color_syntax, SvgColorSyntax, SvgOptions};

use crate::doc::{split_math, Block, BlockKind};
use crate::table;

/// Body-text em size. RaTeX defaults to 40, which fills the preview pane
/// when the SVG is ObjectFit::Contain'd. Keep math at reading size.
const FONT_SIZE: f64 = 16.0;
const FONT_PAD: f64 = 3.0;

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
    Table(table::Table),
    Fallback(String),
}

/// Render a LaTeX snippet to SVG via RaTeX (parser → layout → SVG).
///
/// GPUI's usvg backend does not paint `rgba(...)` fills, so we emit `rgb(...)`.
pub fn latex_to_math(latex: &str) -> Result<SvgMath> {
    let source = compact_tex(latex.trim());
    if source.is_empty() {
        return Err(anyhow!("empty latex"));
    }
    let svg = match render_math(&source) {
        Ok(svg) => svg,
        Err(err) => {
            if let Some(inner) = strip_env(&source, "aligned") {
                render_math(&inner)?
            } else {
                return Err(err);
            }
        }
    };
    let (width, height) = svg_pt_size(&svg).unwrap_or((FONT_SIZE as f32 * 2.0, FONT_SIZE as f32));
    Ok(SvgMath { svg, width, height })
}

fn render_math(source: &str) -> Result<String> {
    let ast = parse(source).map_err(|e| anyhow!("ratex parse: {e}"))?;
    let tree = layout(&ast, &LayoutOptions::default());
    let list = to_display_list(&tree);
    let opts = SvgOptions {
        font_size: FONT_SIZE,
        padding: FONT_PAD,
        stroke_width: 0.7,
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

fn try_math(latex: &str) -> Option<SvgMath> {
    latex_to_math(latex).ok()
}

/// Build a document-style preview: prose, inline/display math, tables.
pub fn document_preview(blocks: &[Block]) -> Vec<PreviewBlock> {
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
                if let Some(table) = table::parse_html(&block.text) {
                    out.push(PreviewBlock::Table(table));
                } else {
                    out.push(PreviewBlock::Fallback(block.text.clone()));
                }
            }
            BlockKind::Formula => {
                if is_display_formula(&block.text) {
                    flush_para(&mut para, &mut out);
                    if let Some(math) = try_math(&block.text) {
                        out.push(PreviewBlock::Display(math));
                    } else {
                        out.push(PreviewBlock::Fallback(block.text.clone()));
                    }
                } else if let Some(math) = try_math(&block.text) {
                    para.push(InlineSeg::Math(math));
                } else {
                    para.push(InlineSeg::Text(format!("${}$", block.text.trim())));
                }
            }
            BlockKind::Text => {
                if table::looks_like_html_table(&block.text) {
                    flush_para(&mut para, &mut out);
                    if let Some(table) = table::parse_html(&block.text) {
                        out.push(PreviewBlock::Table(table));
                    } else {
                        out.push(PreviewBlock::Fallback(block.text.clone()));
                    }
                    continue;
                }
                for run in split_math(&block.text) {
                    match run {
                        crate::doc::MathRun::Text(t) => {
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
                        crate::doc::MathRun::Inline(tex) => {
                            if let Some(math) = try_math(&tex) {
                                para.push(InlineSeg::Math(math));
                            } else {
                                para.push(InlineSeg::Text(format!("${tex}$")));
                            }
                        }
                        crate::doc::MathRun::Display(tex) => {
                            flush_para(&mut para, &mut out);
                            if let Some(math) = try_math(&tex) {
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

fn is_display_formula(tex: &str) -> bool {
    let t = tex.trim();
    t.contains('\n') || t.len() > 48
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_simple_formula_svg() {
        let math = latex_to_math("E=mc^2").expect("ratex");
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
        let math = latex_to_math("E=mc^2").expect("size");
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
        let svg = latex_to_math(compact).expect("compact aligned").svg;
        assert!(svg.contains("<svg"));
        assert!(!svg.contains("rgba("));
        let spaced = r"\begin{aligned} {\mathbf { z } _ { v } ^ { i } } = f _ { v } ^ { ( 0 ) } ( \mathbf { I } _ { i } ^ { \mathsf { o p t } } ) , \\ \end{aligned}";
        latex_to_math(spaced).expect("spaced aligned");
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
}
