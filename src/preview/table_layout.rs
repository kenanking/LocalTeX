use crate::prefs::ContentFontSize;
use crate::table::{self, Slot, Table};

use super::{segs_from_cell, PlacedCell, PreviewLayout};

pub(crate) fn table_preview_layout(
    table: &Table,
    dpr: f64,
    font: ContentFontSize,
) -> PreviewLayout {
    const PAD_X: f32 = 16.0;
    const PAD_Y: f32 = 8.0;
    const MIN_COL: f32 = 56.0;
    const MAX_COL: f32 = 220.0;
    const MIN_ROW: f32 = 28.0;
    let m = font.metrics();
    let line = m.table_line;
    let caption = m.caption;

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
                    col_w[c] = col_w[c]
                        .max(estimate_text_width(text, caption) + PAD_X)
                        .min(MAX_COL);
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
                    let need =
                        (estimate_text_width(text, caption) + PAD_X).min(MAX_COL * span as f32);
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
            return (n as f32) * line + PAD_Y;
        }
        let inner = (w - PAD_X).max(24.0);
        let lines = ((estimate_text_width(&display, caption) / inner).ceil() as usize).max(1);
        (lines as f32) * line + PAD_Y
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
                        segs: segs_from_cell(text, dpr, font),
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

fn estimate_text_width(s: &str, caption: f32) -> f32 {
    let scale = caption / 12.0;
    s.lines()
        .map(|line| {
            line.chars()
                .map(|ch| {
                    if ch == '$' {
                        5.0 * scale
                    } else if ch.is_ascii() {
                        7.2 * scale
                    } else {
                        12.0 * scale
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
