//! Text postprocessing: clean_special_tokens, MarkdownConverter handlers,
//! truncate_repetitive_content, and OTSL→HTML.
//! Faithful ports of openocr/tools/infer_unirec_onnx.py,
//! openocr/tools/to_markdown.py and
//! openocr/tools/utils/opendoc_onnx_utils/utils.py.

use std::sync::OnceLock;

use regex::Regex;

fn re_static(pattern: &str) -> &'static Regex {
    static CACHE: std::sync::Mutex<Option<Vec<(String, &'static Regex)>>> =
        std::sync::Mutex::new(None);
    let mut guard = CACHE.lock().unwrap();
    let vec = guard.get_or_insert_with(Vec::new);
    if let Some((_, r)) = vec.iter().find(|(p, _)| p == pattern) {
        return r;
    }
    let r: &'static Regex = Box::leak(Box::new(Regex::new(pattern).unwrap()));
    vec.push((pattern.to_string(), r));
    r
}

fn sub(pattern: &str, replacement: &str, text: &str) -> String {
    re_static(pattern)
        .replace_all(text, regex::NoExpand(replacement))
        .into_owned()
}

/// infer_unirec_onnx.py clean_special_tokens.
pub(super) fn clean_special_tokens(text: &str) -> String {
    let mut text = text.replace('\u{0120}', " ").replace('\u{010a}', "\n");
    text = text
        .replace("<|bos|>", "")
        .replace("<|eos|>", "")
        .replace("<|pad|>", "");
    text = text.replace("-<|sn|>", "");
    text = text.replace(" <|sn|>", " ");
    text = text.replace("<|sn|>", " ");
    text = text.replace("<|unk|>", "");
    text = text.replace("<s>", "");
    text = text.replace("</s>", "");
    text = text.replace('\u{ffff}', "");
    text = sub(r"_{4,}", "___", &text);
    text = sub(r"\.{4,}", "...", &text);
    text
}

/// MarkdownConverter.replace_dict, in Python insertion order (duplicate
/// '\pm' key keeps its first position). Keys/values are the Python-evaluated
/// literals (single backslashes).
const REPLACE_DICT: [(&str, &str); 10] = [
    ("\\bm", "\\mathbf "),
    ("\\eqno", "\\quad "),
    ("\\quad", "\\quad "),
    ("\\leq", "\\leq "),
    ("\\pm", "\\pm "),
    ("\\varmathbb", "\\mathbb "),
    ("\\in fty", "\\infty"),
    ("\\mu", "\\mu "),
    ("\\cdot", "\\cdot "),
    ("\\langle", "\\langle "),
];

fn apply_replace_dict(mut text: String) -> String {
    for (k, v) in REPLACE_DICT {
        text = text.replace(k, v);
    }
    text
}

/// to_markdown.py fix_latex_brackets.
fn fix_latex_brackets(text: &str) -> String {
    static P: OnceLock<Regex> = OnceLock::new();
    let re = P.get_or_init(|| {
        Regex::new(
            r"\\(big|Big|bigg|Bigg|bigl|bigr|Bigl|Bigr|biggr|biggl|Biggl|Biggr)\{(\\?[{}\[\]\(\)\|])\}",
        )
        .unwrap()
    });
    re.replace_all(text, "\\$1$2").into_owned()
}

/// MarkdownConverter._process_formulas_in_text.
fn process_formulas_in_text(text: &str) -> String {
    let mut text = text
        .replace("\\upmu", "\\mu")
        .replace("\\(", "$")
        .replace("\\)", "$");
    text = apply_replace_dict(text);
    text
}

/// MarkdownConverter._handle_text.
pub(super) fn handle_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    if ["图中没有可识别的文本。", "图中无文本。", "图中没有文本。"].contains(&text)
    {
        return String::new();
    }
    let mut text = text.to_string();
    // module-level `rules`
    text = text.replace("-<|sn|>", "");
    text = text.replace("<|sn|>", "");
    text = text.replace("<|unk|>", "");
    text = text.replace('\u{ffff}', "");
    text = sub(r"_{4,}", "___", &text);
    text = sub(r"\.{4,}", "...", &text);
    text = process_formulas_in_text(&text);
    text = text.replace("$\\bullet$", "•");
    if text.contains("<table>") {
        text = sub(
            r"(?i)</?(table|tr|th|td|thead|tbody|tfoot)[^>]*>",
            "",
            &text,
        );
        text = sub(r"\n\s*\n+", "\n", &text);
    }
    normalize_text(&text)
}

/// Ligatures, NBSP, full-width ASCII alphanumerics, collapsed blank lines.
/// Do not Markdown-escape here: `Block.text` is shared with preview and LaTeX.
pub(super) fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            '\u{00A0}' | '\u{3000}' => out.push(' '),
            '\u{FF10}'..='\u{FF19}' | '\u{FF21}'..='\u{FF3A}' | '\u{FF41}'..='\u{FF5A}' => {
                out.push(char::from_u32(ch as u32 - 0xFEE0).unwrap())
            }
            _ => out.push(ch),
        }
    }
    sub(r"\n{3,}", "\n\n", &out)
}

/// When the text still carries \(\)/\[\] pairs, strip $ and rewrite them as $/$$.
pub(super) fn normalize_math_delimiters(text: &str) -> String {
    let has_paren = text.contains("\\(") && text.contains("\\)");
    let has_bracket = text.contains("\\[") && text.contains("\\]");
    if !has_paren && !has_bracket {
        return text.to_string();
    }
    let mut text = text.replace('$', "");
    text = text
        .replace("\\(", " $ ")
        .replace("\\)", " $ ")
        .replace("\\[", " $$ ")
        .replace("\\]", " $$ ");
    text
}

/// MarkdownConverter._handle_formula.
pub(super) fn handle_formula(text: &str) -> String {
    let text = text.replace("\\upmu", "\\mu");
    let mut result = sub(r"\\] \(\d+\)\n\n", "\\]", &text);
    result = result.replace("<|sn|>", "");
    result = result.replace("<|unk|>", "");
    result = result.replace('\u{ffff}', "");
    result = sub(r"_{4,}", "___", &result);
    // Literal str.replace calls (note: "\]\n*\[" is literal, not regex).
    result = result.replace("\\]\n*\\[", "\\\\");
    result = result.replace("\n\n\\[", "");
    result = result.replace("\\]\n\n", "");
    result = result.replace("\\[\n", "");
    result = result.replace("\n\\]", "");
    result = result.replace("\\]", "");
    result = result.replace("\\[", "");
    result = result.replace("\\( ", "");
    result = result.replace(" \\)", "");
    result = result.replace("\\(", "");
    result = result.replace("\\)", "");
    // strip('$') then rstrip('\\ '), then \upmu -> \mu
    let mut text = result.trim_matches('$').to_string();
    while text.ends_with('\\') || text.ends_with(' ') {
        text.pop();
    }
    let mut text = text.replace("\\upmu", "\\mu");
    text = apply_replace_dict(text);
    let mut processed = format!("$${}$$", text);
    processed = processed.replace('\n', "\\\\\n");
    processed = fix_latex_brackets(&processed);
    format!("{}\n\n", processed)
}

/// to_markdown.py extract_table_from_html.
fn extract_table_from_html(html: &str) -> String {
    static P: OnceLock<Regex> = OnceLock::new();
    static T: OnceLock<Regex> = OnceLock::new();
    let table_re = P.get_or_init(|| Regex::new(r"(?s)<table.*?>.*?</table>").unwrap());
    let tag_re = T.get_or_init(|| Regex::new(r"<table[^>]*>").unwrap());
    let tables: Vec<String> = table_re
        .find_iter(html)
        .map(|m| tag_re.replace_all(m.as_str(), "<table>").into_owned())
        .collect();
    tables.join("\n")
}

/// MarkdownConverter._handle_table.
pub(super) fn handle_table(text: &str) -> String {
    let markdown_table = extract_table_from_html(text);
    let mut table_content = markdown_table.replace("<tdcolspan=", "<td colspan=");
    table_content = table_content.replace("<tdrowspan=", "<td rowspan=");
    table_content = table_content.replace("\"colspan=", "\" colspan=");
    table_content = table_content.replace("<|sn|>", "");
    table_content = table_content.replace("<|unk|>", "");
    table_content = table_content.replace('\u{ffff}', "");
    table_content = sub(r"_{4,}", "___", &table_content);
    table_content = sub(r"\.{4,}", "...", &table_content);
    table_content = sub(
        "(?i)</td\\s+colspan=\"[^\"]*\"\\s*>",
        "</td>",
        &table_content,
    );
    table_content = sub(
        "(?i)</td\\s+rowspan=\"[^\"]*\"\\s*>",
        "</td>",
        &table_content,
    );
    table_content = sub(
        "(?i)</th\\s+rowspan=\"[^\"]*\"\\s*>",
        "</th>",
        &table_content,
    );
    table_content = sub(
        "(?i)</th\\s+colspan=\"[^\"]*\"\\s*>",
        "</th>",
        &table_content,
    );
    table_content = table_content.replace("\\(", "$").replace("\\)", "$");
    table_content = table_content.replace("\\[", "$$").replace("\\]", "$$");
    format!("{}\n\n\n", table_content)
}

// ---------------------------------------------------------------------------
// truncate_repetitive_content (utils.py)

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn find_shortest_repeating_substring(s: &[char]) -> Option<Vec<char>> {
    let n = s.len();
    for i in 1..=(n / 2) {
        if n.is_multiple_of(i) {
            let unit = &s[..i];
            let reps = n / i;
            let mut built = Vec::with_capacity(n);
            for _ in 0..reps {
                built.extend_from_slice(unit);
            }
            if built == s {
                return Some(unit.to_vec());
            }
        }
    }
    None
}

/// Returns (prefix, unit, count).
fn find_repeating_suffix(
    s: &[char],
    min_len: usize,
    min_repeats: usize,
) -> Option<(Vec<char>, Vec<char>, usize)> {
    let n = s.len();
    let mut i = n / min_repeats;
    while i >= min_len {
        if i == 0 {
            break;
        }
        let unit = &s[n - i..];
        // s ends with unit * min_repeats?
        let mut ends = true;
        for r in 0..min_repeats {
            let end = n - r * i;
            if end < i || &s[end - i..end] != unit {
                ends = false;
                break;
            }
        }
        if ends {
            let mut count = 0usize;
            let mut pos = n;
            while pos >= i && &s[pos - i..pos] == unit {
                pos -= i;
                count += 1;
            }
            let start_index = n - count * i;
            return Some((s[..start_index].to_vec(), unit.to_vec(), count));
        }
        i -= 1;
    }
    None
}

pub(super) fn truncate_repetitive_content(content: &str) -> String {
    let stripped = content.trim();
    if stripped.is_empty() {
        return content.to_string();
    }
    let sc = chars(stripped);

    // Priority 1: phrase-level suffix repetition in long single lines.
    if !stripped.contains('\n') && sc.len() > 100 {
        if let Some((prefix, unit, count)) = find_repeating_suffix(&sc, 8, 5) {
            if unit.len() * count > (sc.len() as f64 * 0.5) as usize {
                return prefix.into_iter().collect();
            }
        }
    }

    // Priority 2: full-string character-level repetition.
    if !stripped.contains('\n') && sc.len() > 10 {
        if let Some(unit) = find_shortest_repeating_substring(&sc) {
            let count = sc.len() / unit.len();
            if count >= 10 {
                return unit.into_iter().collect();
            }
        }
    }

    // Priority 3: line-level repetition.
    let lines: Vec<&str> = content
        .split('\n')
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() || lines.len() < 10 {
        return content.to_string();
    }
    let total = lines.len();
    // Counter.most_common: ties broken by first occurrence.
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for line in &lines {
        if let Some(entry) = counts.iter_mut().find(|(l, _)| l == line) {
            entry.1 += 1;
        } else {
            counts.push((line, 1));
        }
    }
    counts.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let (line, count) = counts[0];
    if count >= 10 && (count as f64 / total as f64) >= 0.8 {
        return line.to_string();
    }
    content.to_string()
}

// ---------------------------------------------------------------------------
// OTSL -> HTML (utils.py)

const OTSL_NL: &str = "<nl>";
const OTSL_FCEL: &str = "<fcel>";
const OTSL_ECEL: &str = "<ecel>";
const OTSL_LCEL: &str = "<lcel>";
const OTSL_UCEL: &str = "<ucel>";
const OTSL_XCEL: &str = "<xcel>";
const OTSL_TAGS: [&str; 6] = [
    OTSL_NL, OTSL_FCEL, OTSL_ECEL, OTSL_LCEL, OTSL_UCEL, OTSL_XCEL,
];

fn is_otsl_tag(s: &str) -> bool {
    OTSL_TAGS.contains(&s)
}

/// OTSL_FIND_PATTERN findall: tag + text up to next tag (or end).
fn otsl_find_cells(line: &str) -> Vec<String> {
    let mut positions: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < line.len() {
        if let Some(tag) = OTSL_TAGS.iter().find(|t| line[i..].starts_with(**t)) {
            positions.push(i);
            i += tag.len();
        } else {
            i += line[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
    }
    positions
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            let end = positions.get(k + 1).copied().unwrap_or(line.len());
            line[p..end].to_string()
        })
        .collect()
}

fn otsl_pad_to_sqr_v2(otsl: &str) -> String {
    let otsl = otsl.trim();
    if !otsl.contains(OTSL_NL) {
        return format!("{}{}", otsl, OTSL_NL);
    }
    struct Row {
        raw_cells: Vec<String>,
        total_len: usize,
        min_len: usize,
    }
    let mut row_data: Vec<Row> = Vec::new();
    for line in otsl.split(OTSL_NL) {
        if line.is_empty() {
            continue;
        }
        let raw_cells = otsl_find_cells(line);
        if raw_cells.is_empty() {
            continue;
        }
        let total_len = raw_cells.len();
        let mut min_len = 0usize;
        for (i, cell) in raw_cells.iter().enumerate() {
            if cell.starts_with(OTSL_FCEL) {
                min_len = i + 1;
            }
        }
        row_data.push(Row {
            raw_cells,
            total_len,
            min_len,
        });
    }
    if row_data.is_empty() {
        return OTSL_NL.to_string();
    }
    let global_min_width = row_data.iter().map(|r| r.min_len).max().unwrap_or(0);
    let max_total_len = row_data.iter().map(|r| r.total_len).max().unwrap_or(0);
    let search_start = global_min_width;
    let search_end = global_min_width.max(max_total_len);
    let mut min_total_cost = f64::INFINITY;
    let mut optimal_width = search_end;
    for width in search_start..=search_end {
        let cost: f64 = row_data
            .iter()
            .map(|r| (r.total_len as f64 - width as f64).abs())
            .sum();
        if cost < min_total_cost {
            min_total_cost = cost;
            optimal_width = width;
        }
    }
    let mut repaired: Vec<String> = Vec::new();
    for row in &row_data {
        let cells = &row.raw_cells;
        let new_cells: Vec<String> = if cells.len() > optimal_width {
            cells[..optimal_width].to_vec()
        } else {
            let mut c = cells.clone();
            c.resize(optimal_width, OTSL_ECEL.to_string());
            c
        };
        repaired.push(new_cells.concat());
    }
    repaired.join(OTSL_NL) + OTSL_NL
}

/// otsl_extract_tokens_and_text: (tokens, text_parts incl. tags, non-blank).
fn otsl_extract_tokens_and_text(s: &str) -> (Vec<String>, Vec<String>) {
    let mut tokens: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut seg_start = 0usize;
    while i < s.len() {
        if let Some(tag) = OTSL_TAGS.iter().find(|t| s[i..].starts_with(**t)) {
            let seg = &s[seg_start..i];
            if !seg.trim().is_empty() {
                parts.push(seg.to_string());
            }
            tokens.push(tag.to_string());
            parts.push(tag.to_string());
            i += tag.len();
            seg_start = i;
        } else {
            i += s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
    }
    let seg = &s[seg_start..];
    if !seg.trim().is_empty() {
        parts.push(seg.to_string());
    }
    (tokens, parts)
}

#[derive(Clone, Debug)]
struct TableCell {
    row_span: usize,
    col_span: usize,
    start_row: usize,
    end_row: usize,
    start_col: usize,
    end_col: usize,
    text: String,
}

fn otsl_parse_texts(texts: &[String], tokens: &[String]) -> (Vec<TableCell>, Vec<Vec<String>>) {
    // itertools.groupby(tokens, z == NL), keep non-NL groups.
    let mut split_row_tokens: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for t in tokens {
        if t == OTSL_NL {
            if !current.is_empty() {
                split_row_tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(t.clone());
        }
    }
    if !current.is_empty() {
        split_row_tokens.push(current);
    }

    let mut texts: Vec<String> = texts.to_vec();
    if !split_row_tokens.is_empty() {
        let max_cols = split_row_tokens.iter().map(|r| r.len()).max().unwrap_or(0);
        for row in split_row_tokens.iter_mut() {
            while row.len() < max_cols {
                row.push(OTSL_ECEL.to_string());
            }
        }
        let mut new_texts: Vec<String> = Vec::new();
        let mut text_idx = 0usize;
        for row in &split_row_tokens {
            for token in row {
                new_texts.push(token.clone());
                if text_idx < texts.len() && texts[text_idx] == *token {
                    text_idx += 1;
                    if text_idx < texts.len() && !is_otsl_tag(&texts[text_idx]) {
                        new_texts.push(texts[text_idx].clone());
                        text_idx += 1;
                    }
                }
            }
            new_texts.push(OTSL_NL.to_string());
            if text_idx < texts.len() && texts[text_idx] == OTSL_NL {
                text_idx += 1;
            }
        }
        texts = new_texts;
    }

    let count_right =
        |tokens: &[Vec<String>], c_idx: usize, r_idx: usize, which: &[&str]| -> usize {
            let mut span = 0usize;
            let mut c = c_idx;
            loop {
                match tokens.get(r_idx).and_then(|row| row.get(c)) {
                    Some(t) if which.contains(&t.as_str()) => {
                        c += 1;
                        span += 1;
                    }
                    _ => return span,
                }
            }
        };
    let count_down =
        |tokens: &[Vec<String>], c_idx: usize, r_idx: usize, which: &[&str]| -> usize {
            let mut span = 0usize;
            let mut r = r_idx;
            loop {
                match tokens.get(r).and_then(|row| row.get(c_idx)) {
                    Some(t) if which.contains(&t.as_str()) => {
                        r += 1;
                        span += 1;
                    }
                    _ => return span,
                }
            }
        };

    let mut table_cells: Vec<TableCell> = Vec::new();
    let mut r_idx = 0usize;
    let mut c_idx = 0usize;
    let mut i = 0usize;
    while i < texts.len() {
        let text = &texts[i];
        if text == OTSL_FCEL || text == OTSL_ECEL {
            let mut row_span = 1usize;
            let mut col_span = 1usize;
            let right_offset;
            let mut cell_text = String::new();
            if text != OTSL_ECEL {
                cell_text = texts.get(i + 1).cloned().unwrap_or_default();
                right_offset = 2;
            } else {
                right_offset = 1;
            }
            let next_right_cell = texts.get(i + right_offset).cloned().unwrap_or_default();
            let mut next_bottom_cell = String::new();
            if r_idx + 1 < split_row_tokens.len() {
                if let Some(t) = split_row_tokens[r_idx + 1].get(c_idx) {
                    next_bottom_cell = t.clone();
                }
            }
            if next_right_cell == OTSL_LCEL || next_right_cell == OTSL_XCEL {
                col_span +=
                    count_right(&split_row_tokens, c_idx + 1, r_idx, &[OTSL_LCEL, OTSL_XCEL]);
            }
            if next_bottom_cell == OTSL_UCEL || next_bottom_cell == OTSL_XCEL {
                row_span +=
                    count_down(&split_row_tokens, c_idx, r_idx + 1, &[OTSL_UCEL, OTSL_XCEL]);
            }
            table_cells.push(TableCell {
                text: cell_text.trim().to_string(),
                row_span,
                col_span,
                start_row: r_idx,
                end_row: r_idx + row_span,
                start_col: c_idx,
                end_col: c_idx + col_span,
            });
        }
        if is_otsl_tag(text) && text != OTSL_NL {
            c_idx += 1;
        }
        if text == OTSL_NL {
            r_idx += 1;
            c_idx = 0;
        }
        i += 1;
    }
    (table_cells, split_row_tokens)
}

/// Python html.escape(s, quote=True).
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn export_to_html(cells: &[TableCell], num_rows: usize, num_cols: usize) -> String {
    if cells.is_empty() {
        return String::new();
    }
    // Build grid.
    let default_cell = |i: usize, j: usize| TableCell {
        row_span: 1,
        col_span: 1,
        start_row: i,
        end_row: i + 1,
        start_col: j,
        end_col: j + 1,
        text: String::new(),
    };
    let mut grid: Vec<Vec<TableCell>> = (0..num_rows)
        .map(|i| (0..num_cols).map(|j| default_cell(i, j)).collect())
        .collect();
    for cell in cells {
        let r0 = cell.start_row.min(num_rows);
        let r1 = cell.end_row.min(num_rows);
        let c0 = cell.start_col.min(num_cols);
        let c1 = cell.end_col.min(num_cols);
        for row in grid[r0..r1].iter_mut() {
            for slot in row[c0..c1].iter_mut() {
                *slot = cell.clone();
            }
        }
    }
    let mut body = String::new();
    for (i, row) in grid.iter().enumerate() {
        body.push_str("<tr>");
        for (j, cell) in row.iter().enumerate() {
            if cell.start_row != i || cell.start_col != j {
                continue;
            }
            let content = html_escape(cell.text.trim());
            let mut opening = String::from("td");
            if cell.row_span > 1 {
                opening.push_str(&format!(" rowspan=\"{}\"", cell.row_span));
            }
            if cell.col_span > 1 {
                opening.push_str(&format!(" colspan=\"{}\"", cell.col_span));
            }
            body.push_str(&format!("<{}>{}</td>", opening, content));
        }
        body.push_str("</tr>");
    }
    format!("<table>{}</table>", body)
}

/// convert_otsl_to_html from utils.py.
pub(super) fn convert_otsl_to_html(otsl_content: &str) -> String {
    let padded = otsl_pad_to_sqr_v2(otsl_content);
    let (tokens, mixed) = otsl_extract_tokens_and_text(&padded);
    let (cells, split_row_tokens) = otsl_parse_texts(&mixed, &tokens);
    let num_rows = split_row_tokens.len();
    let num_cols = split_row_tokens.iter().map(|r| r.len()).max().unwrap_or(0);
    export_to_html(&cells, num_rows, num_cols)
}

pub(super) const IGNORE_LABELS: [&str; 8] = [
    "number",
    "footnote",
    "header",
    "footer",
    "aside_text",
    "footer_image",
    "header_image",
    "chart",
];

/// Strip a trailing `_NN` numeric suffix: "text_01" -> "text".
pub(super) fn base_label(label: &str) -> &str {
    if let Some((base, suffix)) = label.rsplit_once('_') {
        if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
            return base;
        }
    }
    label
}
