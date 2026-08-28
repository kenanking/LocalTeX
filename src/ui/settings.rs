use gpui::{div, prelude::*, px, relative, rgb, AnyElement, App, Entity, SharedString};

use super::theme;
use super::widgets::{
    kbd_chip, pill_tab, seg_item, segmented, setting_row, settings_group, switch, Tooltip,
};
use crate::doc::ExportFmt;
use crate::prefs::{BlockDelim, InlineDelim, Prefs, WindowCloseAction};
use crate::state::AppState;
use crate::sysmon::{fmt_bytes, fmt_used_total, SysSnapshot};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Formatting,
    Shortcuts,
    System,
}

const TABS: [(&str, &str, SettingsTab); 4] = [
    ("set-general", "General", SettingsTab::General),
    ("set-formatting", "Formatting", SettingsTab::Formatting),
    ("set-shortcuts", "Shortcuts", SettingsTab::Shortcuts),
    ("set-system", "System", SettingsTab::System),
];

pub fn page(
    state: Entity<AppState>,
    tab: SettingsTab,
    snap: &SysSnapshot,
    on_tab: impl Fn(SettingsTab, &mut App) + Clone + 'static,
    cx: &App,
) -> impl IntoElement {
    let prefs = state.read(cx).prefs.clone();

    let mut tabs_row = div()
        .flex()
        .gap_1()
        .p_1()
        .rounded_full()
        .bg(rgb(theme::BG_SUNKEN))
        .border_1()
        .border_color(rgb(theme::BORDER));
    for (id, label, t) in TABS {
        let on_tab = on_tab.clone();
        tabs_row = tabs_row.child(pill_tab(id, label, tab == t, move |_, cx| on_tab(t, cx)));
    }

    div()
        .id("settings")
        .flex_1()
        .min_h_0()
        .min_w_0()
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(rgb(theme::BG_RAISED))
        .child(
            div()
                .w_full()
                .flex_shrink_0()
                .flex()
                .justify_center()
                .py_2()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::BG))
                .child(tabs_row),
        )
        .child(
            div()
                .id("settings-body")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .items_center()
                .p_4()
                .child(
                    // Content column caps out so meter bars stay scannable
                    // instead of stretching across a wide window.
                    div()
                        .w_full()
                        .max_w(px(600.))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .when(tab == SettingsTab::General, |d| {
                            d.child(general_page(state.clone(), &prefs))
                        })
                        .when(tab == SettingsTab::Formatting, |d| {
                            d.child(formatting_page(state.clone(), &prefs))
                        })
                        .when(tab == SettingsTab::Shortcuts, |d| d.child(shortcuts_page()))
                        .when(tab == SettingsTab::System, |d| d.child(system_page(snap))),
                ),
        )
}

fn general_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            "Capture",
            vec![
                bool_row(
                    &state,
                    "pref-hide",
                    "Hide window while snipping",
                    "Restore when recognition finishes.",
                    prefs.hide_on_capture,
                    |p, v| p.hide_on_capture = v,
                ),
                bool_row(
                    &state,
                    "pref-orig",
                    "Show original after recognize",
                    "Snip image above the recognized document.",
                    prefs.show_original,
                    |p, v| p.show_original = v,
                ),
                bool_row(
                    &state,
                    "pref-copy",
                    "Copy result automatically",
                    "Copies the primary format for this snip (Ctrl+C).",
                    prefs.autocopy,
                    |p, v| p.autocopy = v,
                ),
            ],
        ))
        .child(settings_group(
            "Window",
            vec![bool_row(
                &state,
                "pref-close-min",
                "Minimize on close",
                "Keeps the tray and Ctrl+Shift+S. Off quits; Ctrl+Q always quits.",
                prefs.close_action == WindowCloseAction::Minimize,
                |p, v| {
                    p.close_action = if v {
                        WindowCloseAction::Minimize
                    } else {
                        WindowCloseAction::Quit
                    };
                },
            )],
        ))
}

fn formatting_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            "Export",
            vec![setting_row(
                "Primary format",
                "Used for Ctrl+C and auto-copy. Extra formats stay available on each snip.",
                picker(
                    150.,
                    [
                        seg_item(
                            "pref-md",
                            ExportFmt::Markdown.label(),
                            prefs.default_fmt == ExportFmt::Markdown,
                            {
                                let state = state.clone();
                                move |_, cx| {
                                    state.update(cx, |s, cx| {
                                        s.update_prefs(cx, |p| p.default_fmt = ExportFmt::Markdown);
                                        s.set_format(ExportFmt::Markdown, cx);
                                    });
                                }
                            },
                        ),
                        seg_item(
                            "pref-tex",
                            ExportFmt::Latex.label(),
                            prefs.default_fmt == ExportFmt::Latex,
                            {
                                let state = state.clone();
                                move |_, cx| {
                                    state.update(cx, |s, cx| {
                                        s.update_prefs(cx, |p| p.default_fmt = ExportFmt::Latex);
                                        s.set_format(ExportFmt::Latex, cx);
                                    });
                                }
                            },
                        ),
                    ],
                ),
            )
            .into_any_element()],
        ))
        .child(settings_group(
            "Math delimiters",
            vec![
                setting_row(
                    "Inline math",
                    "Wraps inline formulas in Markdown and mixed LaTeX export.",
                    picker(
                        150.,
                        [
                            seg_item(
                                "pref-inl-d",
                                "$ … $",
                                prefs.inline_delim == InlineDelim::Dollar,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.inline_delim = InlineDelim::Dollar
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-inl-p",
                                "\\( … \\)",
                                prefs.inline_delim == InlineDelim::Paren,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.inline_delim = InlineDelim::Paren
                                        })
                                    }
                                },
                            ),
                        ],
                    ),
                )
                .into_any_element(),
                setting_row(
                    "Display math",
                    "Wraps longer formulas in document export. Snip copy also offers equation.",
                    picker(
                        210.,
                        [
                            seg_item(
                                "pref-blk-d",
                                "$$",
                                prefs.block_delim == BlockDelim::Dollars,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.block_delim = BlockDelim::Dollars
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-blk-b",
                                "\\[ \\]",
                                prefs.block_delim == BlockDelim::Brackets,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.block_delim = BlockDelim::Brackets
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-blk-e",
                                "equation*",
                                prefs.block_delim == BlockDelim::Equation,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.block_delim = BlockDelim::Equation
                                        })
                                    }
                                },
                            ),
                        ],
                    ),
                )
                .into_any_element(),
            ],
        ))
}

fn shortcuts_page() -> impl IntoElement {
    settings_group(
        "Keyboard",
        vec![
            shortcut_row("Create snip from screenshot", "Ctrl+Shift+S"),
            shortcut_row("Upload snip", "Ctrl+O"),
            shortcut_row("Paste image or path from clipboard", "Ctrl+V"),
            shortcut_row("Create snip from drawing", "Ctrl+D"),
            shortcut_row("Delete selected snip", "Delete"),
            shortcut_row("Settings", "Ctrl+,"),
            shortcut_row("Toggle Markdown / LaTeX", "Ctrl+L"),
            shortcut_row("Copy primary format", "Ctrl+C"),
        ],
    )
}

fn shortcut_row(label: &str, keys: &str) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .min_w_0()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(rgb(theme::TEXT))
                .whitespace_normal()
                .child(SharedString::from(label.to_string())),
        )
        .child(kbd_chip(SharedString::from(keys.to_string())))
        .into_any_element()
}

/// One dense card. Form follows the metric: rate → sparkline (CPU),
/// capacity → meter bar (drive), composition → stacked bar with a dot
/// legend (memory split out into LocalTeX vs the rest; app data on disk).
fn system_page(snap: &SysSnapshot) -> impl IntoElement {
    let mem_label = match (snap.mem_used, snap.mem_total) {
        (Some(used), Some(total)) => fmt_used_total(used, total),
        _ => "—".into(),
    };
    let memory = match (snap.mem_used, snap.mem_total) {
        (Some(used), Some(total)) => {
            let rss = snap.app_rss.unwrap_or(0).min(used);
            composition_block(
                "Memory",
                mem_label,
                total,
                vec![
                    ("set-mem-sys", "System", used - rss, theme::SEG_NEUTRAL),
                    ("set-mem-app", "LocalTeX", rss, theme::ACCENT),
                ],
            )
        }
        _ => stat_line("Memory", None, mem_label),
    };

    let mut panel = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .w_full()
        .min_w_0()
        .child(cpu_line(snap))
        .child(memory);

    if let Some(disk) = &snap.disk {
        panel = panel.child(hairline());
        if let (Some(total), Some(free)) = (disk.drive_total, disk.drive_free) {
            let used_frac = (total - free.min(total)) as f32 / total.max(1) as f32;
            panel = panel.child(stat_line(
                "Data drive",
                Some(used_frac),
                format!("{:.0}% · {} free", used_frac * 100., fmt_bytes(free)),
            ));
        }
        panel = panel.child(composition_block(
            "App data",
            fmt_bytes(disk.app_footprint()),
            disk.app_footprint(),
            vec![
                ("set-disk-models", "Models", disk.models, theme::ACCENT),
                ("set-disk-snips", "Snips", disk.snips, theme::OK),
                ("set-disk-bin", "Binary", disk.binary, theme::WARN),
            ],
        ));
    } else {
        panel = panel.child(hairline()).child(stat_row_shell(
            "Storage",
            div().into_any_element(),
            "measuring…".into(),
        ));
    }

    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            "This machine",
            vec![panel.into_any_element()],
        ))
}

const STAT_LABEL_W: f32 = 64.0;
const STAT_VALUE_W: f32 = 128.0;
const BAR_H: f32 = 6.0;

/// label ── viz ── value on one 18px line; viz fills the middle column.
fn stat_row_shell(label: &'static str, viz: AnyElement, value: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .h(px(18.))
        .w_full()
        .min_w_0()
        .child(
            div()
                .w(px(STAT_LABEL_W))
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(viz))
        .child(
            div()
                .w(px(STAT_VALUE_W))
                .flex_shrink_0()
                .text_xs()
                .text_right()
                .whitespace_nowrap()
                .text_color(rgb(theme::TEXT))
                .child(SharedString::from(value)),
        )
        .into_any_element()
}

/// Capacity meter, BAR_H tall like every other bar in the card.
fn stat_line(label: &'static str, frac: Option<f32>, value: String) -> AnyElement {
    stat_row_shell(
        label,
        div()
            .w_full()
            .h(px(BAR_H))
            .rounded_full()
            .bg(rgb(theme::BG_SUNKEN))
            .overflow_hidden()
            .child(
                div()
                    .h_full()
                    .w(relative(frac.unwrap_or(0.0).clamp(0.0, 1.0)))
                    .rounded_full()
                    .bg(rgb(theme::ACCENT)),
            )
            .into_any_element(),
        value,
    )
}

/// Task-Manager mini graph: one histogram bar per 1.5 s sample, latest at
/// the right edge, filling the whole viz column.
fn cpu_line(snap: &SysSnapshot) -> AnyElement {
    let hist = &snap.cpu_hist;
    let mut graph = div()
        .id("cpu-spark")
        .w_full()
        .h(px(18.))
        .flex()
        .gap(px(1.))
        .tooltip(Tooltip::text(format!(
            "CPU history — last {} s",
            crate::sysmon::CPU_HIST_LEN * 3 / 2
        )));
    // Each bar sits in a full-height slot pinned to the bottom: gpui 0.2
    // `items_end` does not bottom-align flex_1 children here, and without a
    // shared baseline the bars ragged-hang from the top.
    for _ in hist.len()..crate::sysmon::CPU_HIST_LEN {
        graph = graph.child(spark_slot(1., theme::ACCENT));
    }
    for &v in hist {
        let h = (v / 100. * 17.).clamp(1., 17.);
        graph = graph.child(spark_slot(h, cpu_color(v)));
    }
    stat_row_shell(
        "CPU",
        graph.into_any_element(),
        snap.cpu_pct
            .map(|p| format!("{p:.0}%"))
            .unwrap_or_else(|| "—".into()),
    )
}

/// Full-height column slot with the bar glued to the bottom.
fn spark_slot(h: f32, color: u32) -> AnyElement {
    div()
        .flex_1()
        .h_full()
        .flex()
        .flex_col()
        .justify_end()
        .child(div().w_full().h(px(h)).bg(rgb(color)))
        .into_any_element()
}

/// Utilization-adaptive color: calm blue, busy amber, saturated red.
fn cpu_color(pct: f32) -> u32 {
    if pct >= 85.0 {
        theme::DANGER
    } else if pct >= 50.0 {
        theme::WARN
    } else {
        theme::ACCENT
    }
}

fn hairline() -> AnyElement {
    div()
        .h(px(1.))
        .w_full()
        .bg(rgb(theme::BORDER))
        .into_any_element()
}

/// Stacked composition bar (same BAR_H as the meters) + dot legend beneath.
/// Segments share `denom`; if they sum to less, the remainder stays track
/// (e.g. free RAM after the used split). Hover a segment for its size.
fn composition_block(
    row_label: &'static str,
    value: String,
    denom: u64,
    segs: Vec<(&'static str, &'static str, u64, u32)>,
) -> AnyElement {
    let denom = denom.max(1) as f32;
    let cap = px(BAR_H / 2.);
    // gpui 0.2 `overflow_hidden` clips to a rect, not the rounded track —
    // round the two end segments instead so the bar keeps pill caps.
    let visible: Vec<_> = segs
        .into_iter()
        .filter(|(_, _, bytes, _)| *bytes > 0)
        .collect();
    let last = visible.len().saturating_sub(1);
    let mut bar = div()
        .w_full()
        .h(px(BAR_H))
        .rounded_full()
        .overflow_hidden()
        .flex()
        .bg(rgb(theme::BG_SUNKEN));
    let mut legend = div().flex().items_center().gap_3().min_w_0();
    for (i, (id, label, bytes, color)) in visible.into_iter().enumerate() {
        bar = bar.child(
            div()
                .id(id)
                .h_full()
                .w(relative(bytes as f32 / denom))
                .min_w(px(3.))
                .when(i == 0, |d| d.rounded_l(cap))
                .when(i == last, |d| d.rounded_r(cap))
                .bg(rgb(color))
                .tooltip(Tooltip::text(format!("{label} — {}", fmt_bytes(bytes)))),
        );
        legend = legend.child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
                .child(
                    div()
                        .text_xs()
                        .whitespace_nowrap()
                        .text_color(rgb(theme::MUTED))
                        .child(format!("{label} {}", fmt_bytes(bytes))),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .min_w_0()
        .child(stat_row_shell(row_label, bar.into_any_element(), value))
        .child(
            div()
                .flex()
                .w_full()
                .min_w_0()
                .pl(px(STAT_LABEL_W + 12.))
                .child(legend),
        )
        .into_any_element()
}

/// Segmented control pinned to a fixed width (row-right picker).
fn picker(width: f32, items: impl IntoIterator<Item = AnyElement>) -> impl IntoElement {
    div().w(px(width)).flex_shrink_0().child(segmented(items))
}

fn bool_row(
    state: &Entity<AppState>,
    id: &'static str,
    title: &'static str,
    hint: &'static str,
    value: bool,
    set: fn(&mut Prefs, bool),
) -> AnyElement {
    let state = state.clone();
    setting_row(
        title,
        hint,
        switch(id, value, move |_, cx| {
            state.update(cx, |s, cx| s.update_prefs(cx, |p| set(p, !value)));
        }),
    )
    .into_any_element()
}

fn patch_prefs(state: &Entity<AppState>, cx: &mut App, f: impl FnOnce(&mut Prefs)) {
    state.update(cx, |s, cx| s.update_prefs(cx, f));
}
