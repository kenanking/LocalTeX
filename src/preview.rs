use anyhow::{anyhow, Result};
use ratex_layout::layout_options::LayoutOptions;
use ratex_layout::{layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg_with_color_syntax, SvgColorSyntax, SvgOptions};

/// Render a LaTeX snippet to SVG via RaTeX (parser → layout → SVG).
///
/// GPUI's usvg backend does not paint `rgba(...)` fills, so we emit `rgb(...)`.
pub fn latex_to_svg(latex: &str) -> Result<String> {
    let source = compact_tex(latex.trim());
    if source.is_empty() {
        return Err(anyhow!("empty latex"));
    }
    match render_math(&source) {
        Ok(svg) => Ok(svg),
        Err(err) => {
            if let Some(inner) = strip_env(&source, "aligned") {
                render_math(&inner)
            } else {
                Err(err)
            }
        }
    }
}

fn render_math(source: &str) -> Result<String> {
    let ast = parse(source).map_err(|e| anyhow!("ratex parse: {e}"))?;
    let tree = layout(&ast, &LayoutOptions::default());
    let list = to_display_list(&tree);
    let mut opts = SvgOptions::default();
    opts.embed_glyphs = true;
    Ok(render_to_svg_with_color_syntax(
        &list,
        &opts,
        SvgColorSyntax::Rgb,
    ))
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

/// Combine formula blocks into one preview document.
pub fn document_preview_svg(blocks: &[crate::doc::Block]) -> Result<Option<String>> {
    use crate::doc::BlockKind;
    let formulas: Vec<&str> = blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Formula && !b.text.trim().is_empty())
        .map(|b| b.text.as_str())
        .collect();
    if formulas.is_empty() {
        return Ok(None);
    }
    let joined = formulas.join(" \\\\ ");
    Ok(Some(latex_to_svg(&joined)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_simple_formula_svg() {
        let svg = latex_to_svg("E=mc^2").expect("ratex");
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
    }

    #[test]
    fn renders_spaced_tokens_aligned() {
        let compact = r"\begin{aligned} {\mathbf{z}}_{v}^{i}=f_{v}^{(0)}({\mathbf{I}}_{i}^{\mathsf{opt}}), \\ \end{aligned}";
        let svg = latex_to_svg(compact).expect("compact aligned");
        assert!(svg.contains("<svg"));
        assert!(!svg.contains("rgba("));
        let spaced = r"\begin{aligned} {\mathbf { z } _ { v } ^ { i } } = f _ { v } ^ { ( 0 ) } ( \mathbf { I } _ { i } ^ { \mathsf { o p t } } ) , \\ \end{aligned}";
        latex_to_svg(spaced).expect("spaced aligned");
        assert_eq!(
            compact_tex(r"\mathbf { z } _ { v } ^ { i }"),
            r"\mathbf{z}_{v}^{i}"
        );
    }
}
