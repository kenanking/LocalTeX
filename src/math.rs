//! Mixed `$` / `$$` splitting and TeX canonicalize (`\tag` repair).
//! One owner: OCR, preview, and export all call these functions.
//! Equation numbers are folded in the OCR pipeline (`formula_number` or a
//! trailing `(n)` on the formula crop → `\tag`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MathRun {
    Text(String),
    Inline(String),
    Display(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Sub,
    Super,
}

/// Lone `_{…}` / `^{…}` (no nucleus). `x^2` is not this.
/// OCR often emits an empty group: `{}_{50}` for AP$_{50}$.
pub fn script_kind(tex: &str) -> Option<ScriptKind> {
    let t = strip_empty_nucleus(tex.trim());
    if t.starts_with('_') {
        Some(ScriptKind::Sub)
    } else if t.starts_with('^') {
        Some(ScriptKind::Super)
    } else {
        None
    }
}

/// Body to typeset for a detected script: `_2` → `2`, `{}_{50}` → `50`.
pub fn unwrap_script_body(tex: &str) -> &str {
    let t = strip_empty_nucleus(tex.trim());
    let rest = t
        .strip_prefix('_')
        .or_else(|| t.strip_prefix('^'))
        .unwrap_or(t);
    rest.strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'))
        .unwrap_or(rest)
}

fn strip_empty_nucleus(tex: &str) -> &str {
    tex.strip_prefix("{}").unwrap_or(tex)
}

/// Split mixed OCR text into prose and `$` / `$$` math.
pub fn split_math(input: &str) -> Vec<MathRun> {
    let mut runs = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    let mut buf = String::new();
    let push_text = |buf: &mut String, runs: &mut Vec<MathRun>| {
        if !buf.is_empty() {
            runs.push(MathRun::Text(std::mem::take(buf)));
        }
    };
    while i < chars.len() {
        if chars[i] == '$' {
            let display = i + 1 < chars.len() && chars[i + 1] == '$';
            let delim_len = if display { 2 } else { 1 };
            if let Some(end) = find_closer(&chars, i + delim_len, display) {
                push_text(&mut buf, &mut runs);
                let body: String = chars[i + delim_len..end].iter().collect();
                if display {
                    let body = canonicalize_tex(body.trim());
                    runs.push(MathRun::Display(body));
                } else {
                    runs.push(MathRun::Inline(body.trim().to_string()));
                }
                i = end + delim_len;
                continue;
            }
        }
        buf.push(chars[i]);
        i += 1;
    }
    push_text(&mut buf, &mut runs);
    runs
}

/// Reconstruct mixed text after splitting `$` / `$$` runs.
pub fn canonicalize_mixed_text(input: &str) -> String {
    let mut out = String::new();
    for run in split_math(input) {
        match run {
            MathRun::Text(t) => out.push_str(&t),
            MathRun::Inline(s) => {
                out.push('$');
                out.push_str(&s);
                out.push('$');
            }
            MathRun::Display(s) => {
                out.push_str("$$");
                out.push_str(&s);
                out.push_str("$$");
            }
        }
    }
    out
}

/// Strip `$` / `$$` wrappers. The bool is true for display dollars, `\tag`,
/// or a multi-line body.
pub fn unwrap_formula(text: &str) -> (String, bool) {
    let t = text.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() >= 2 && chars[0] == '$' && chars[1] == '$' {
        if let Some(end) = find_closer(&chars, 2, true) {
            if chars[end + 2..].iter().all(|c| c.is_whitespace()) {
                let body: String = chars[2..end].iter().collect();
                return finish_formula(body.trim().to_string(), true);
            }
        }
    } else if chars.first() == Some(&'$') {
        if let Some(end) = find_closer(&chars, 1, false) {
            if chars[end + 1..].iter().all(|c| c.is_whitespace()) {
                return finish_formula(
                    chars[1..end].iter().collect::<String>().trim().to_string(),
                    false,
                );
            }
        }
    }
    finish_formula(t.to_string(), false)
}

fn finish_formula(body: String, display: bool) -> (String, bool) {
    let body = canonicalize_tex(&body);
    let display = display || is_display_body(&body);
    (body, display)
}

pub fn is_display_body(tex: &str) -> bool {
    let t = tex.trim();
    t.contains('\n') || t.contains(r"\begin{") || t.contains(r"\tag{")
}

/// OCR often emits a paper number as a trailing `\\`. A Rust raw
/// string `r"\\tag"` is two backslashes, so a Python port can produce `\\tag{1}`
/// which KaTeX parses as a line-break plus the letters "tag", not `\tag`.
pub fn canonicalize_tex(s: &str) -> String {
    let mut t = s.replace(r"\\tag{", r"\tag{");
    loop {
        let trimmed = t.trim_end_matches([' ', '\t', '\n', '\r']);
        if let Some(rest) = trimmed.strip_suffix(r"\\") {
            t = rest.to_string();
            continue;
        }
        t = trimmed.to_string();
        break;
    }
    if let Some(i) = t.rfind(r"\tag{") {
        if i > 0 && !t.as_bytes()[i - 1].is_ascii_whitespace() {
            t.insert(i, ' ');
        }
    }
    t
}

/// Strip every `\tag{...}` from display TeX. Unbalanced braces: leave `tex`
/// unchanged and return no tags (RaTeX can still fail-soft).
pub fn split_display_tag(tex: &str) -> (String, Vec<String>) {
    let needle = r"\tag{";
    let mut tags = Vec::new();
    let mut out = String::with_capacity(tex.len());
    let mut rest = tex;
    loop {
        let Some(at) = rest.find(needle) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..at]);
        let inner_start = at + needle.len();
        let Some((inner, after)) = split_braced(&rest[inner_start..]) else {
            return (tex.to_string(), Vec::new());
        };
        tags.push(inner.trim().to_string());
        rest = after;
    }
    (out.trim().to_string(), tags)
}

/// UI / typeset form of a `\tag` payload (`1` → `(1)`).
pub fn format_eqno(tag: &str) -> String {
    let t = tag.trim();
    if t.starts_with('(') && t.ends_with(')') && t.len() >= 2 {
        t.to_string()
    } else {
        format!("({t})")
    }
}

pub(crate) fn split_braced(s: &str) -> Option<(String, &str)> {
    let mut depth = 1i32;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((s[..i].to_string(), &s[i + c.len_utf8()..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn find_closer(chars: &[char], start: usize, display: bool) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '$' {
            if display {
                if i + 1 < chars.len() && chars[i + 1] == '$' {
                    return Some(i);
                }
            } else {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_braced_finds_matching_close() {
        let (inner, rest) = super::split_braced("foo} bar").expect("close");
        assert_eq!(inner, "foo");
        assert_eq!(rest, " bar");
        assert!(super::split_braced("no close").is_none());
    }

    #[test]
    fn split_math_inline_and_display() {
        let runs = split_math("Because $c$ and $$1-c$$.");
        assert_eq!(
            runs,
            vec![
                MathRun::Text("Because ".into()),
                MathRun::Inline("c".into()),
                MathRun::Text(" and ".into()),
                MathRun::Display("1-c".into()),
                MathRun::Text(".".into()),
            ]
        );
    }

    #[test]
    fn split_math_leaves_trailing_eq_number_in_prose() {
        let runs = split_math(r"Hence $$E=mc^2$$ (1) holds.");
        assert_eq!(
            runs,
            vec![
                MathRun::Text("Hence ".into()),
                MathRun::Display(r"E=mc^2".into()),
                MathRun::Text(" (1) holds.".into()),
            ]
        );
    }

    #[test]
    fn unwrap_formula_strips_dollars_and_repairs_tag() {
        let (body, display) = unwrap_formula("$$ E=mc^2 $$");
        assert_eq!(body, "E=mc^2");
        assert!(display);
        let (body, display) = unwrap_formula("$x$");
        assert_eq!(body, "x");
        assert!(!display);
        let (body, display) = unwrap_formula(
            r"{\rm ACC}=\frac{1}{N}I\left[\hat{y}_{i}=y_{i}\right],\\tag{1}\\
\\",
        );
        assert_eq!(
            body,
            r"{\rm ACC}=\frac{1}{N}I\left[\hat{y}_{i}=y_{i}\right], \tag{1}"
        );
        assert!(display, "\\tag implies display math");
        let (body, display) = unwrap_formula(r"$$a+b \tag{1}$$");
        assert_eq!(body, r"a+b \tag{1}");
        assert!(display);
        let (body, _) = unwrap_formula("f(1)");
        assert_eq!(body, "f(1)", "do not treat f(1) as an equation number");
        let (body, _) = unwrap_formula(r"{\rm ACC}=1 (1)");
        assert_eq!(
            body, r"{\rm ACC}=1 (1)",
            "trailing (n) is not folded here; layout pairing owns tags"
        );
    }

    #[test]
    fn split_display_tag_strips_all_tags() {
        let (body, tags) = split_display_tag(r"E=mc^2 \tag{1}");
        assert_eq!(body, "E=mc^2");
        assert_eq!(tags, vec!["1".to_string()]);
    }

    #[test]
    fn split_display_tag_none() {
        let (body, tags) = split_display_tag("a+b");
        assert_eq!(body, "a+b");
        assert!(tags.is_empty());
    }

    #[test]
    fn split_display_tag_two_tags() {
        let (body, tags) = split_display_tag(r"a+b \tag{1} \tag{2}");
        assert!(!body.contains(r"\tag"));
        assert_eq!(tags, vec!["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn split_display_tag_after_canonicalize() {
        let t = canonicalize_tex(r"x\\tag{3}\\");
        let (body, tags) = split_display_tag(&t);
        assert_eq!(tags, vec!["3".to_string()]);
        assert!(!body.contains(r"\tag"));
    }

    #[test]
    fn split_display_tag_unbalanced_leaves_source() {
        let src = r"E=mc^2 \tag{1";
        let (body, tags) = split_display_tag(src);
        assert_eq!(body, src);
        assert!(tags.is_empty());
    }

    #[test]
    fn format_eqno_parenthesizes() {
        assert_eq!(format_eqno("1"), "(1)");
        assert_eq!(format_eqno("(11)"), "(11)");
        assert_eq!(format_eqno(" 2 "), "(2)");
    }

    #[test]
    fn script_kind_only_when_tex_is_a_lone_script() {
        assert_eq!(script_kind("_2"), Some(ScriptKind::Sub));
        assert_eq!(script_kind("_{ij}"), Some(ScriptKind::Sub));
        assert_eq!(script_kind("^2"), Some(ScriptKind::Super));
        assert_eq!(script_kind("^{th}"), Some(ScriptKind::Super));
        assert_eq!(script_kind("{}_{50}"), Some(ScriptKind::Sub));
        assert_eq!(script_kind("{}^{th}"), Some(ScriptKind::Super));
        assert_eq!(script_kind("{x}_{50}"), None);
        assert_eq!(script_kind("x^2"), None);
        assert_eq!(script_kind("E=mc^2"), None);
        assert_eq!(script_kind("2"), None);
    }

    #[test]
    fn unwrap_script_body_strips_marker() {
        assert_eq!(unwrap_script_body("_2"), "2");
        assert_eq!(unwrap_script_body("_{ij}"), "ij");
        assert_eq!(unwrap_script_body("^2"), "2");
        assert_eq!(unwrap_script_body("^{th}"), "th");
        assert_eq!(unwrap_script_body("{}_{50}"), "50");
        assert_eq!(unwrap_script_body("{}^{th}"), "th");
        assert_eq!(unwrap_script_body("2"), "2");
    }
}
