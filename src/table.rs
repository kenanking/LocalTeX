//! HTML tables from UniRec → Markdown / LaTeX / TSV.

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub rowspan: usize,
    pub colspan: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Origin {
        text: String,
        rowspan: usize,
        colspan: usize,
    },
    /// Occupied by a rowspan from a previous row — still a tabular column.
    RowSpan,
    /// Occupied by a colspan on this row — do not emit another `&` cell.
    ColSpan,
}

impl Slot {
    fn origin_text(&self) -> &str {
        match self {
            Slot::Origin { text, .. } => text,
            Slot::RowSpan | Slot::ColSpan => "",
        }
    }
}

impl Table {
    /// Occupied grid that keeps span metadata for LaTeX `\multirow` / `\multicolumn`
    /// and for preview cells that should look merged. Continuation rows that
    /// `\multirow` spans are kept (an Origin-only filter would drop them).
    pub fn slot_grid(&self) -> Vec<Vec<Slot>> {
        if self.rows.is_empty() {
            return Vec::new();
        }
        let mut occupied: Vec<Vec<Option<Slot>>> = Vec::new();
        for (r, row) in self.rows.iter().enumerate() {
            while occupied.len() <= r {
                occupied.push(Vec::new());
            }
            let mut c = 0usize;
            for cell in row {
                while occupied[r].len() <= c {
                    occupied[r].push(None);
                }
                while occupied[r].get(c).is_some_and(|x| x.is_some()) {
                    c += 1;
                    while occupied[r].len() <= c {
                        occupied[r].push(None);
                    }
                }
                let rs = cell.rowspan.max(1);
                let cs = cell.colspan.max(1);
                for dr in 0..rs {
                    let rr = r + dr;
                    while occupied.len() <= rr {
                        occupied.push(Vec::new());
                    }
                    for dc in 0..cs {
                        let cc = c + dc;
                        while occupied[rr].len() <= cc {
                            occupied[rr].push(None);
                        }
                        if occupied[rr][cc].is_none() {
                            occupied[rr][cc] = Some(if dr == 0 && dc == 0 {
                                Slot::Origin {
                                    text: cell.text.clone(),
                                    rowspan: rs,
                                    colspan: cs,
                                }
                            } else if dr == 0 {
                                Slot::ColSpan
                            } else {
                                Slot::RowSpan
                            });
                        }
                    }
                }
                c += cs;
            }
        }
        occupied
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|c| c.unwrap_or(Slot::RowSpan))
                    .collect()
            })
            .collect()
    }

    /// Markdown / TSV projection: Origin text, empty for span occupancy.
    /// All-empty rows (pure continuation) are dropped from these formats.
    fn plain_rows(&self) -> Vec<Vec<String>> {
        let slots = self.slot_grid();
        let width = slots.iter().map(|r| r.len()).max().unwrap_or(0);
        slots
            .into_iter()
            .map(|row| {
                (0..width)
                    .map(|c| row.get(c).map(Slot::origin_text).unwrap_or("").to_string())
                    .collect()
            })
            .filter(|row: &Vec<String>| !row.iter().all(|c| c.is_empty()))
            .collect()
    }

    pub fn to_markdown(&self) -> String {
        let grid = self.plain_rows();
        if grid.is_empty() {
            return String::new();
        }
        let width = grid.iter().map(|r| r.len()).max().unwrap_or(0);
        if width == 0 {
            return String::new();
        }
        let mut out = String::new();
        for (i, row) in grid.iter().enumerate() {
            out.push('|');
            for c in 0..width {
                let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
                out.push(' ');
                out.push_str(&md_cell(cell));
                out.push_str(" |");
            }
            out.push('\n');
            if i == 0 {
                out.push('|');
                for _ in 0..width {
                    out.push_str(" :--- |");
                }
                out.push('\n');
            }
        }
        out
    }

    pub fn to_latex(&self) -> String {
        let slots = self.slot_grid();
        if slots.is_empty() {
            return String::new();
        }
        let width = slots.iter().map(|r| r.len()).max().unwrap_or(0);
        if width == 0 {
            return String::new();
        }
        let cols = vec!["l"; width].join("|");
        let mut out = format!("\\begin{{tabular}}[t]{{|{cols}|}}\n\\hline\n");
        for row in &slots {
            let mut cells = Vec::new();
            let mut c = 0usize;
            while c < width {
                let slot = row.get(c).cloned().unwrap_or(Slot::RowSpan);
                match slot {
                    Slot::ColSpan => c += 1,
                    Slot::RowSpan => {
                        cells.push(String::new());
                        c += 1;
                    }
                    Slot::Origin {
                        text,
                        rowspan,
                        colspan,
                    } => {
                        let mut body = latex_cell(&text);
                        if rowspan > 1 {
                            body = format!("\\multirow{{{rowspan}}}{{*}}{{{body}}}");
                        }
                        if colspan > 1 {
                            body = format!("\\multicolumn{{{colspan}}}{{|c|}}{{{body}}}");
                        }
                        cells.push(body);
                        c += colspan.max(1);
                    }
                }
            }
            out.push_str(&cells.join(" & "));
            out.push_str(" \\\\\n\\hline\n");
        }
        out.push_str("\\end{tabular}");
        out
    }

    pub fn to_tsv(&self) -> String {
        let grid = self.plain_rows();
        grid.iter()
            .map(|row| row.join("\t"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ").trim().to_string()
}

fn latex_cell(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    // Keep $...$ math; escape the rest of a cell that is mostly prose.
    if s.contains('$') {
        return s.replace('\n', " ");
    }
    escape_latex_cell(s)
}

fn escape_latex_cell(text: &str) -> String {
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
            '\n' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

pub fn looks_like_html_table(s: &str) -> bool {
    let t = s.to_ascii_lowercase();
    t.contains("<table")
}

pub fn parse_html(html: &str) -> Option<Table> {
    static TABLE: OnceLock<Regex> = OnceLock::new();
    static TR: OnceLock<Regex> = OnceLock::new();
    static CELL: OnceLock<Regex> = OnceLock::new();
    static ATTR: OnceLock<Regex> = OnceLock::new();
    let table_re =
        TABLE.get_or_init(|| Regex::new(r"(?is)<table\b[^>]*>(.*?)</table>").expect("table re"));
    let tr_re = TR.get_or_init(|| Regex::new(r"(?is)<tr\b[^>]*>(.*?)</tr>").expect("tr re"));
    let cell_re = CELL
        .get_or_init(|| Regex::new(r"(?is)<(td|th)\b([^>]*)>(.*?)</(?:td|th)>").expect("cell re"));
    let attr_re = ATTR
        .get_or_init(|| Regex::new(r#"(?i)(rowspan|colspan)\s*=\s*["']?(\d+)"#).expect("attr re"));

    let inner = table_re
        .captures(html)
        .and_then(|c| c.get(1).map(|m| m.as_str()))
        .unwrap_or(html);

    let mut rows = Vec::new();
    for tr in tr_re.captures_iter(inner) {
        let row_html = tr.get(1).map(|m| m.as_str()).unwrap_or("");
        let mut row = Vec::new();
        for cap in cell_re.captures_iter(row_html) {
            let attrs = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let raw = cap.get(3).map(|m| m.as_str()).unwrap_or("");
            let mut rowspan = 1usize;
            let mut colspan = 1usize;
            for a in attr_re.captures_iter(attrs) {
                let n: usize = a.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
                match a.get(1).map(|m| m.as_str().to_ascii_lowercase()).as_deref() {
                    Some("rowspan") => rowspan = n.max(1),
                    Some("colspan") => colspan = n.max(1),
                    _ => {}
                }
            }
            row.push(Cell {
                text: decode_html(&strip_tags(raw)),
                rowspan,
                colspan,
            });
        }
        if !row.is_empty() {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        None
    } else {
        Some(Table { rows })
    }
}

pub fn html_to_markdown(html: &str) -> String {
    parse_html(html)
        .map(|t| t.to_markdown())
        .unwrap_or_else(|| html.trim().to_string())
}

pub fn html_to_latex(html: &str) -> String {
    parse_html(html)
        .map(|t| t.to_latex())
        .unwrap_or_else(|| html.trim().to_string())
}

fn strip_tags(s: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let re = TAG.get_or_init(|| Regex::new(r"(?is)<[^>]+>").expect("tag re"));
    re.replace_all(s, "").into_owned()
}

fn decode_html(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<table><tr><td></td><td>AP</td><td>AP50</td><td>AP75</td></tr><tr><td>初版（错误归一化）</td><td>48.48</td><td>83.68</td><td>50.96</td></tr><tr><td>800 定稿（本次）</td><td>54.20</td><td>88.09</td><td>58.14</td></tr></table>"#;

    #[test]
    fn markdown_pipe_table() {
        let md = html_to_markdown(SAMPLE);
        assert!(md.contains("| AP | AP50 | AP75 |"), "{md}");
        assert!(md.contains("| :--- | :--- | :--- | :--- |"), "{md}");
        assert!(
            md.contains("| 初版（错误归一化） | 48.48 | 83.68 | 50.96 |"),
            "{md}"
        );
        assert!(
            md.contains("| 800 定稿（本次） | 54.20 | 88.09 | 58.14 |"),
            "{md}"
        );
    }

    #[test]
    fn latex_tabular() {
        let tex = html_to_latex(SAMPLE);
        assert!(tex.contains("\\begin{tabular}[t]{|l|l|l|l|}"), "{tex}");
        assert!(tex.contains(" & AP & AP50 & AP75 \\\\"), "{tex}");
        assert!(tex.contains("初版（错误归一化） & 48.48"), "{tex}");
        assert!(tex.contains("\\end{tabular}"), "{tex}");
    }

    #[test]
    fn colspan_expands() {
        let html =
            r#"<table><tr><td colspan="2">ab</td></tr><tr><td>a</td><td>b</td></tr></table>"#;
        let t = parse_html(html).unwrap();
        let grid = t.plain_rows();
        assert_eq!(grid[0], vec!["ab".to_string(), String::new()]);
        assert_eq!(grid[1], vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn math_in_cell_kept() {
        let html = r#"<table><tr><th>x</th></tr><tr><td>$\lambda=1$</td></tr></table>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("$\\lambda=1$"), "{md}");
        let tex = html_to_latex(html);
        assert!(tex.contains("$\\lambda=1$"), "{tex}");
    }

    #[test]
    fn latex_keeps_rowspan_and_colspan() {
        let html = r#"<table><tr><td rowspan="2">A</td><td colspan="2">BC</td></tr><tr><td>B</td><td>C</td></tr></table>"#;
        let tex = html_to_latex(html);
        assert!(tex.contains("\\multirow{2}{*}{A}"), "{tex}");
        assert!(tex.contains("\\multicolumn{2}{|c|}{BC}"), "{tex}");
        assert!(tex.contains("B & C"), "{tex}");
        let md = html_to_markdown(html);
        assert!(
            md.contains("| A | BC |  |"),
            "markdown stays a flattened pipe table: {md}"
        );
    }
}
