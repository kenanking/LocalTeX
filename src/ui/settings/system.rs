use gpui::{div, prelude::*, px, relative, rgb, AnyElement, Entity, PromptLevel, SharedString};

use super::super::theme;
use super::super::widgets::{btn, setting_row, settings_group, Tooltip};
use crate::state::AppState;
use crate::sysmon::{fmt_bytes, fmt_used_total, SysSnapshot};

pub(super) fn system_page(state: Entity<AppState>, snap: &SysSnapshot) -> impl IntoElement {
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
        .child(settings_group(
            "Library",
            vec![setting_row(
                "Delete all snips",
                "Removes every snip and the database. Settings stay. This cannot be undone.",
                btn("wipe-library", "Delete all…", false, true, {
                    let state = state.clone();
                    move |window, cx| {
                        let answer = window.prompt(
                            PromptLevel::Warning,
                            "Delete all snips?",
                            Some(
                                "Removes every snip and the local database. Settings and shortcuts stay. This cannot be undone.",
                            ),
                            &["Cancel", "Delete all"],
                            cx,
                        );
                        let state = state.clone();
                        cx.spawn(async move |cx| {
                            if answer.await == Ok(1) {
                                state.update(cx, |s, cx| s.wipe_library(cx));
                            }
                        })
                        .detach();
                    }
                }),
            )
            .into_any_element()],
        ))
}

const STAT_LABEL_W: f32 = 64.0;
const STAT_VALUE_W: f32 = 128.0;
const BAR_H: f32 = 6.0;

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
            crate::sysmon::CPU_HIST_LEN as u64 * crate::sysmon::SAMPLE_INTERVAL.as_millis() as u64
                / 1000
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
