//! Mixed `$` / `$$` splitting and display-math canonicalize (`\tag`).
//! One owner: OCR, preview, and export all call these functions.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MathRun {
    Text(String),
    Inline(String),
    Display(String),
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
                    let mut body = canonicalize_tex(body.trim());
                    i = end + delim_len;
                    if let Some((next, tag)) = take_eq_number(&chars, i) {
                        fold_eq_tag(&mut body, &tag);
                        i = next;
                    }
                    runs.push(MathRun::Display(body));
                } else {
                    runs.push(MathRun::Inline(body.trim().to_string()));
                    i = end + delim_len;
                }
                continue;
            }
        }
        buf.push(chars[i]);
        i += 1;
    }
    push_text(&mut buf, &mut runs);
    runs
}

/// Reconstruct mixed text after folding `$$…$$ (n)` into `\tag`.
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

/// Strip `$` / `$$` wrappers and fold a trailing paper number into `\tag`.
/// The bool is true for display dollars, `\tag`, or a multi-line body.
pub fn unwrap_formula(text: &str) -> (String, bool) {
    let t = text.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() >= 2 && chars[0] == '$' && chars[1] == '$' {
        if let Some(end) = find_closer(&chars, 2, true) {
            if let Some((after, tag)) = take_eq_number(&chars, end + 2) {
                if after == chars.len() {
                    let body: String = chars[2..end].iter().collect();
                    let mut body = body.trim().to_string();
                    fold_eq_tag(&mut body, &tag);
                    return finish_formula(body, true);
                }
            } else if end + 2 == chars.len() {
                let body: String = chars[2..end].iter().collect();
                return finish_formula(body.trim().to_string(), true);
            }
        }
    } else if chars.first() == Some(&'$') {
        if let Some(end) = find_closer(&chars, 1, false) {
            if end + 1 == chars.len() {
                return finish_formula(
                    chars[1..end].iter().collect::<String>().trim().to_string(),
                    false,
                );
            }
        }
    }
    if let Some(body) = strip_dangling_display_closer(t) {
        return finish_formula(body, true);
    }
    let mut body = t.to_string();
    fold_trailing_eq_number(&mut body);
    finish_formula(body, false)
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

fn strip_dangling_display_closer(t: &str) -> Option<String> {
    let chars: Vec<char> = t.chars().collect();
    let mut i = chars.len();
    while i >= 2 {
        i -= 1;
        if chars[i] == '$' && chars[i - 1] == '$' {
            if let Some((after, tag)) = take_eq_number(&chars, i + 1) {
                if after == chars.len() {
                    let body: String = chars[..i - 1].iter().collect();
                    let body = body.trim();
                    if !body.is_empty() {
                        let mut body = body.to_string();
                        fold_eq_tag(&mut body, &tag);
                        return Some(body);
                    }
                }
            }
            break;
        }
    }
    None
}

/// Fold `formula (1)` after `\[…\]` is stripped. Do not treat `f(1)` as a tag.
fn fold_trailing_eq_number(body: &mut String) {
    let chars: Vec<char> = body.chars().collect();
    for start in 0..chars.len() {
        if !matches!(chars[start], ' ' | '\t' | '\n' | '\r' | '\\') {
            continue;
        }
        if let Some((after, tag)) = take_eq_number(&chars, start) {
            if after == chars.len() {
                let prefix: String = chars[..start].iter().collect();
                let mut prefix = prefix.trim_end().to_string();
                if prefix.is_empty() {
                    return;
                }
                fold_eq_tag(&mut prefix, &tag);
                *body = prefix;
                return;
            }
        }
    }
}

/// `(1)`, `(2.1)`, `(A1)`, `(1a)` after a display closer, including one newline.
fn take_eq_number(chars: &[char], start: usize) -> Option<(usize, String)> {
    let mut i = start;
    let mut saw_nl = false;
    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' => i += 1,
            '\n' if !saw_nl => {
                saw_nl = true;
                i += 1;
            }
            '\r' => i += 1,
            _ => break,
        }
    }
    if i < chars.len() && chars[i] == '\\' {
        let rest: String = chars[i..].iter().collect();
        if let Some(n) = rest
            .strip_prefix("\\qquad")
            .map(|_| 6)
            .or_else(|| rest.strip_prefix("\\quad").map(|_| 5))
        {
            i += n;
            while i < chars.len() && matches!(chars[i], ' ' | '\t') {
                i += 1;
            }
        }
    }
    if chars.get(i) != Some(&'(') {
        return None;
    }
    let open = i;
    let mut j = i + 1;
    if j < chars.len() && chars[j].is_ascii_alphabetic() {
        j += 1;
    }
    if j >= chars.len() || !chars[j].is_ascii_digit() {
        return None;
    }
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
    }
    while j + 1 < chars.len() && chars[j] == '.' && chars[j + 1].is_ascii_digit() {
        j += 1;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
    }
    if j < chars.len() && (chars[j].is_ascii_alphabetic() || chars[j] == '\'') {
        j += 1;
    }
    if chars.get(j) != Some(&')') {
        return None;
    }
    let tag: String = chars[open..=j].iter().collect();
    j += 1;
    if chars.get(j) == Some(&'\r') {
        j += 1;
    }
    if chars.get(j) == Some(&'\n') {
        j += 1;
    }
    Some((j, tag))
}

fn fold_eq_tag(body: &mut String, tag: &str) {
    *body = canonicalize_tex(body);
    let inner = tag
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(tag);
    let cmd = format!("\\tag{{{inner}}}");
    if body.contains(&cmd) {
        return;
    }
    if !body.is_empty() && !body.ends_with([' ', '\n']) {
        body.push(' ');
    }
    body.push_str(&cmd);
}

/// OCR often emits a paper number as `\] (1)` or a trailing `\\`. A Rust raw
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
    fn split_math_folds_eq_number_into_display() {
        let runs = split_math(r"Hence $$E=mc^2$$ (1) holds.");
        assert_eq!(
            runs,
            vec![
                MathRun::Text("Hence ".into()),
                MathRun::Display(r"E=mc^2 \tag{1}".into()),
                MathRun::Text(" holds.".into()),
            ]
        );
        let runs = split_math("$$\\sum_i x_i$$\n(2.1)\nNext.");
        assert_eq!(
            runs,
            vec![
                MathRun::Display(r"\sum_i x_i \tag{2.1}".into()),
                MathRun::Text("Next.".into()),
            ]
        );
    }

    #[test]
    fn unwrap_formula_strips_dollars_and_folds_eq_number() {
        let (body, display) = unwrap_formula("$$ E=mc^2 $$ (1)");
        assert_eq!(body, r"E=mc^2 \tag{1}");
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
        let (body, _) = unwrap_formula(r"{\rm ACC}=1 (1)");
        assert!(
            body.contains(r"\tag{1}"),
            "number after a separator should fold, got {body:?}"
        );
        let (body, _) = unwrap_formula("f(1)");
        assert_eq!(body, "f(1)", "do not treat f(1) as an equation number");
    }
}
