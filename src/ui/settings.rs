use gpui::{div, prelude::*, px, rgb, App, Entity, SharedString};

use super::theme;
use super::widgets::{checkbox, kbd_chip, section_label, segmented, segmented3};
use crate::doc::ExportFmt;
use crate::prefs::{BlockDelim, InlineDelim, Prefs, WindowCloseAction};
use crate::state::AppState;

pub fn page(state: Entity<AppState>, cx: &App) -> impl IntoElement {
    let prefs = state.read(cx).prefs.clone();

    div()
        .id("settings")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_5()
        .p_6()
        .bg(rgb(theme::BG_RAISED))
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Settings"),
        )
        .child(capture_section(state.clone(), &prefs))
        .child(window_section(state.clone(), &prefs))
        .child(export_section(state.clone(), &prefs))
        .child(shortcuts_section())
}

fn capture_section(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(section_label("Capture"))
        .child(checkbox(
            "pref-hide",
            "Minimize to the taskbar while snipping, restore when recognition finishes",
            prefs.hide_on_capture,
            {
                let state = state.clone();
                let next = !prefs.hide_on_capture;
                move |_, cx| patch_prefs(&state, cx, move |p| p.hide_on_capture = next)
            },
        ))
        .child(checkbox(
            "pref-orig",
            "Show original image after recognize",
            prefs.show_original,
            {
                let state = state.clone();
                let next = !prefs.show_original;
                move |_, cx| patch_prefs(&state, cx, move |p| p.show_original = next)
            },
        ))
        .child(checkbox(
            "pref-copy",
            "Copy result automatically",
            prefs.autocopy,
            {
                let state = state.clone();
                let next = !prefs.autocopy;
                move |_, cx| patch_prefs(&state, cx, move |p| p.autocopy = next)
            },
        ))
}

fn window_section(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(section_label("Window"))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(theme::MUTED))
                        .w(px(110.))
                        .child("Close button"),
                )
                .child(segmented(
                    "pref-close-min",
                    "Minimize",
                    prefs.close_action == WindowCloseAction::Minimize,
                    {
                        let state = state.clone();
                        move |_, cx| {
                            patch_prefs(&state, cx, |p| {
                                p.close_action = WindowCloseAction::Minimize
                            })
                        }
                    },
                    "pref-close-quit",
                    "Quit",
                    prefs.close_action == WindowCloseAction::Quit,
                    {
                        let state = state.clone();
                        move |_, cx| {
                            patch_prefs(&state, cx, |p| p.close_action = WindowCloseAction::Quit)
                        }
                    },
                )),
        )
        .child(div().text_xs().text_color(rgb(theme::MUTED)).child(
            "Minimize keeps the tray and Ctrl+Shift+S. Quit exits the app. Ctrl+Q always quits.",
        ))
}

fn export_section(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(section_label("Export"))
        .child(pref_row(
            "Default format",
            segmented(
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
        ))
        .child(pref_row(
            "Inline math",
            segmented(
                "pref-inl-d",
                "$ … $",
                prefs.inline_delim == InlineDelim::Dollar,
                {
                    let state = state.clone();
                    move |_, cx| patch_prefs(&state, cx, |p| p.inline_delim = InlineDelim::Dollar)
                },
                "pref-inl-p",
                "\\( … \\)",
                prefs.inline_delim == InlineDelim::Paren,
                {
                    let state = state.clone();
                    move |_, cx| patch_prefs(&state, cx, |p| p.inline_delim = InlineDelim::Paren)
                },
            ),
        ))
        .child(pref_row(
            "Display math",
            segmented3(
                "pref-blk-d",
                "$$",
                prefs.block_delim == BlockDelim::Dollars,
                {
                    let state = state.clone();
                    move |_, cx| patch_prefs(&state, cx, |p| p.block_delim = BlockDelim::Dollars)
                },
                "pref-blk-b",
                "\\[ \\]",
                prefs.block_delim == BlockDelim::Brackets,
                {
                    let state = state.clone();
                    move |_, cx| patch_prefs(&state, cx, |p| p.block_delim = BlockDelim::Brackets)
                },
                "pref-blk-e",
                "equation*",
                prefs.block_delim == BlockDelim::Equation,
                {
                    let state = state.clone();
                    move |_, cx| patch_prefs(&state, cx, |p| p.block_delim = BlockDelim::Equation)
                },
            ),
        ))
}

fn shortcuts_section() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(section_label("Shortcuts"))
        .child(shortcut_row("Ctrl+Shift+S", "Create snip from screenshot"))
        .child(shortcut_row("Ctrl+O", "Upload snip"))
        .child(shortcut_row("Ctrl+D", "Create snip from drawing"))
        .child(shortcut_row("Delete", "Delete selected snip"))
        .child(shortcut_row("Ctrl+,", "Settings"))
        .child(shortcut_row("Ctrl+L", "Toggle Markdown / LaTeX"))
        .child(shortcut_row("Ctrl+C", "Copy export"))
}

fn pref_row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .w(px(110.))
                .child(label),
        )
        .child(control)
}

fn shortcut_row(keys: &str, label: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(kbd_chip(SharedString::from(keys.to_string())))
        .child(
            div()
                .text_sm()
                .text_color(rgb(theme::TEXT))
                .child(SharedString::from(label.to_string())),
        )
}

fn patch_prefs(state: &Entity<AppState>, cx: &mut App, f: impl FnOnce(&mut Prefs)) {
    state.update(cx, |s, cx| s.update_prefs(cx, f));
}
