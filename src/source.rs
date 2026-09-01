//! Whole-document editor source: `Vec<Block>` ↔ editable text.
//! Formula snips are a TeX body (display wrapped with newlines). Mixed is
//! Markdown with `$` / `$$` / `\[` islands. Tables are `\begin{tabular}`.

use crate::doc::{unwrap_formula, Block, BlockKind, BlockRole, Rect, SnipKind};
use crate::math::{self, canonicalize_mixed_text, is_display_body, MathRun};
use crate::prefs::Prefs;
use crate::table;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn unit_rect() -> Rect {
    Rect {
        x: 0,
        y: 0,
        w: 1,
        h: 1,
    }
}

pub fn blocks_to_source(blocks: &[Block], prefs: &Prefs) -> String {
    let mut out = String::new();
    for b in blocks {
        if b.text.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        match b.kind {
            BlockKind::Formula => {
                let (body, display) = unwrap_formula(&b.text);
                if b.display || display || is_display_body(&body) {
                    out.push_str(&prefs.wrap_block(&body));
                } else {
                    out.push_str(&prefs.wrap_inline(&body));
                }
            }
            BlockKind::Table => out.push_str(&table::html_to_latex(&b.text)),
            BlockKind::Text => match b.role {
                BlockRole::DocTitle => {
                    out.push_str("# ");
                    out.push_str(&emit_text(&b.text, prefs));
                }
                BlockRole::SectionTitle => {
                    out.push_str("## ");
                    out.push_str(&emit_text(&b.text, prefs));
                }
                _ => out.push_str(&emit_text(&b.text, prefs)),
            },
        }
    }
    out
}

pub fn editor_lang_label(kind: SnipKind, src: &str) -> &'static str {
    match kind {
        SnipKind::Formula | SnipKind::Table => "LaTeX",
        SnipKind::Mixed if src.contains("\\begin{tabular}") => "Markdown + tabular",
        SnipKind::Mixed => "Markdown",
    }
}

fn emit_text(text: &str, prefs: &Prefs) -> String {
    let mut out = String::new();
    for run in math::split_math(text) {
        match run {
            MathRun::Text(t) => out.push_str(&t),
            MathRun::Inline(s) => out.push_str(&prefs.wrap_inline(&s)),
            MathRun::Display(s) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&prefs.wrap_block(&s));
                out.push('\n');
            }
        }
    }
    out
}

pub fn parse_source(src: &str, _prefs: &Prefs) -> Result<Vec<Block>, ParseError> {
    let src = src.trim();
    if src.is_empty() {
        return Ok(Vec::new());
    }
    let islands = split_islands(src)?;
    if islands.len() == 1 {
        match &islands[0] {
            Island::Display(body) => {
                return Ok(vec![formula_block(body, true)]);
            }
            Island::InlineOnly(body) => {
                return Ok(vec![formula_block(body, false)]);
            }
            Island::Tabular(tex) => {
                let html = table::latex_to_html(tex).ok_or_else(|| ParseError {
                    message: "Can't parse this tabular yet.".into(),
                })?;
                return Ok(vec![table_block(html)]);
            }
            Island::Markdown(_) => {}
        }
    }
    let mut blocks = Vec::new();
    for island in islands {
        match island {
            Island::Display(body) => blocks.push(formula_block(&body, true)),
            Island::InlineOnly(body) => blocks.push(formula_block(&body, false)),
            Island::Tabular(tex) => {
                let html = table::latex_to_html(&tex).ok_or_else(|| ParseError {
                    message: "Can't parse this tabular yet.".into(),
                })?;
                blocks.push(table_block(html));
            }
            Island::Markdown(md) => blocks.extend(parse_markdown_chunk(&md)),
        }
    }
    Ok(blocks)
}

fn formula_block(body: &str, display: bool) -> Block {
    let mut b = Block::new(BlockKind::Formula, unit_rect(), body);
    b.display = display;
    b
}

fn table_block(html: String) -> Block {
    Block::new(BlockKind::Table, unit_rect(), html)
}

enum Island {
    Markdown(String),
    Display(String),
    InlineOnly(String),
    Tabular(String),
}

fn split_islands(src: &str) -> Result<Vec<Island>, ParseError> {
    let tab = find_tabulars(src);
    let mut with_disp = Vec::new();
    for piece in tab {
        match piece {
            Piece::Tabular(t) => with_disp.push(Island::Tabular(t)),
            Piece::Text(t) => with_disp.extend(split_display_islands(&t)),
        }
    }
    if with_disp.is_empty() {
        with_disp.push(Island::Markdown(src.to_string()));
    }
    Ok(collapse_inline_only(with_disp))
}

enum Piece {
    Text(String),
    Tabular(String),
}

fn find_tabulars(src: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = src;
    loop {
        let Some(start) = rest.find("\\begin{tabular}") else {
            if !rest.is_empty() {
                out.push(Piece::Text(rest.to_string()));
            }
            break;
        };
        if start > 0 {
            out.push(Piece::Text(rest[..start].to_string()));
        }
        let from = &rest[start..];
        let Some(end_rel) = from.find("\\end{tabular}") else {
            out.push(Piece::Text(from.to_string()));
            break;
        };
        let end = end_rel + "\\end{tabular}".len();
        out.push(Piece::Tabular(from[..end].to_string()));
        rest = &from[end..];
    }
    out
}

fn split_display_islands(src: &str) -> Vec<Island> {
    let mut out = Vec::new();
    let mut last = 0usize;
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let raw: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    while i < raw.len() {
        if raw[i] == '\\' && i + 1 < raw.len() && raw[i + 1] == '[' {
            if let Some(end) = find_raw_bracket_end(&raw, i + 2) {
                let byte_start = chars[i].0;
                let byte_end = if end + 2 < chars.len() {
                    chars[end + 2].0
                } else {
                    src.len()
                };
                if byte_start > last {
                    push_md(&mut out, &src[last..byte_start]);
                }
                let body: String = raw[i + 2..end].iter().collect();
                out.push(Island::Display(body.trim().to_string()));
                last = byte_end;
                i = end + 2;
                continue;
            }
        }
        if raw[i] == '$' && i + 1 < raw.len() && raw[i + 1] == '$' {
            if let Some(end) = find_raw_dollar_end(&raw, i + 2) {
                let byte_start = chars[i].0;
                let byte_end = if end + 2 < chars.len() {
                    chars[end + 2].0
                } else {
                    src.len()
                };
                if byte_start > last {
                    push_md(&mut out, &src[last..byte_start]);
                }
                let body: String = raw[i + 2..end].iter().collect();
                out.push(Island::Display(body.trim().to_string()));
                last = byte_end;
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
    if last < src.len() {
        push_md(&mut out, &src[last..]);
    }
    out
}

fn find_raw_dollar_end(raw: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < raw.len() {
        if raw[i] == '$' && raw[i + 1] == '$' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_raw_bracket_end(raw: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < raw.len() {
        if raw[i] == '\\' && raw[i + 1] == ']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn push_md(out: &mut Vec<Island>, s: &str) {
    if !s.trim().is_empty() {
        out.push(Island::Markdown(s.to_string()));
    }
}

fn collapse_inline_only(islands: Vec<Island>) -> Vec<Island> {
    if islands.len() != 1 {
        return islands;
    }
    if let Island::Markdown(md) = &islands[0] {
        let t = md.trim();
        let runs = math::split_math(t);
        if runs.len() == 1 {
            if let MathRun::Inline(s) = &runs[0] {
                return vec![Island::InlineOnly(s.clone())];
            }
        }
        let (body, display) = unwrap_formula(t);
        if display {
            return vec![Island::Display(body)];
        }
        if !t.contains('\n') && !t.starts_with('#') && !t.contains('|') {
            let only_math = runs
                .iter()
                .all(|r| !matches!(r, MathRun::Text(s) if !s.trim().is_empty()));
            if only_math && runs.iter().any(|r| matches!(r, MathRun::Inline(_))) {
                return vec![Island::InlineOnly(unwrap_formula(t).0)];
            }
        }
    }
    islands
}

fn parse_markdown_chunk(md: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix("# ") {
            let mut b = Block::new(BlockKind::Text, unit_rect(), rest.to_string());
            b.role = BlockRole::DocTitle;
            blocks.push(b);
            continue;
        }
        if let Some(rest) = t.strip_prefix("## ") {
            let mut b = Block::new(BlockKind::Text, unit_rect(), rest.to_string());
            b.role = BlockRole::SectionTitle;
            blocks.push(b);
            continue;
        }
        let text = canonicalize_mixed_text(t);
        blocks.push(Block::new(BlockKind::Text, unit_rect(), text));
    }
    if blocks.is_empty() && !md.trim().is_empty() {
        blocks.push(Block::new(
            BlockKind::Text,
            unit_rect(),
            canonicalize_mixed_text(md.trim()),
        ));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::snip_kind;

    fn rect() -> Rect {
        unit_rect()
    }

    #[test]
    fn editor_lang_label_matches_snip_kind() {
        assert_eq!(editor_lang_label(SnipKind::Formula, "$$x$$"), "LaTeX");
        assert_eq!(editor_lang_label(SnipKind::Mixed, "hello $x$"), "Markdown");
        assert_eq!(
            editor_lang_label(SnipKind::Mixed, "\\begin{tabular}{c}a\\end{tabular}"),
            "Markdown + tabular"
        );
    }

    #[test]
    fn formula_source_puts_display_on_own_lines() {
        let prefs = Prefs::default();
        let mut b = Block::new(BlockKind::Formula, rect(), "E=mc^2");
        b.display = true;
        let src = blocks_to_source(&[b], &prefs);
        assert_eq!(src, "$$\nE=mc^2\n$$");
        let parsed = parse_source(&src, &prefs).expect("parse");
        assert_eq!(snip_kind(&parsed), SnipKind::Formula);
        assert_eq!(unwrap_formula(&parsed[0].text).0, "E=mc^2");
        assert!(parsed[0].display);
    }

    #[test]
    fn formula_source_accepts_brackets() {
        let prefs = Prefs::default();
        let parsed = parse_source("\\[\nE=mc^2\n\\]", &prefs).expect("parse");
        assert_eq!(snip_kind(&parsed), SnipKind::Formula);
        assert_eq!(parsed[0].text, "E=mc^2");
        assert!(parsed[0].display);
    }

    #[test]
    fn mixed_stripped_to_formula_becomes_formula() {
        let prefs = Prefs::default();
        let parsed = parse_source("$$\nE=mc^2\n$$", &prefs).expect("parse");
        assert_eq!(snip_kind(&parsed), SnipKind::Formula);
    }

    #[test]
    fn mixed_heading_and_formula() {
        let prefs = Prefs::default();
        let src = "# CAPM\n\nThe line.\n\n$$\nE=mc^2\n$$\n\nHere $x$.";
        let parsed = parse_source(src, &prefs).expect("parse");
        assert_eq!(snip_kind(&parsed), SnipKind::Mixed);
        assert_eq!(parsed[0].role, BlockRole::DocTitle);
        assert!(parsed.iter().any(|b| b.kind == BlockKind::Formula));
        let out = blocks_to_source(&parsed, &prefs);
        assert!(out.contains("$$\nE=mc^2\n$$"), "{out}");
        assert!(!out.contains("$$E=mc^2$$"), "{out}");
    }

    #[test]
    fn table_roundtrip_tabular() {
        let prefs = Prefs::default();
        let html = "<table><tr><td>a</td><td>b</td></tr><tr><td>1</td><td>2</td></tr></table>";
        let blocks = vec![Block::new(BlockKind::Table, rect(), html)];
        let src = blocks_to_source(&blocks, &prefs);
        assert!(src.contains("\\begin{tabular}"), "{src}");
        let parsed = parse_source(&src, &prefs).expect("parse");
        assert_eq!(snip_kind(&parsed), SnipKind::Table);
        assert_eq!(parsed[0].kind, BlockKind::Table);
        assert!(parsed[0].text.contains("<table>"));
    }
}
