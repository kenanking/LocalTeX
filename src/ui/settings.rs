use gpui::{div, prelude::*, px, rgb, App, Entity, SharedString};

use super::theme;
use super::widgets::{
    checkbox, kbd_chip, seg_item, segmented, setting_row, settings_card, settings_nav_item,
};
use crate::doc::ExportFmt;
use crate::prefs::{BlockDelim, InlineDelim, Prefs, WindowCloseAction};
use crate::state::AppState;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Formatting,
    Shortcuts,
}

pub fn page(
    state: Entity<AppState>,
    tab: SettingsTab,
    on_tab: impl Fn(SettingsTab, &mut App) + Clone + 'static,
    cx: &App,
) -> impl IntoElement {
    let prefs = state.read(cx).prefs.clone();

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
                .flex()
                .items_center()
                .h(px(36.))
                .w_full()
                .flex_shrink_0()
                .px_1()
                .gap_1()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::BG))
                .child(nav(
                    "set-general",
                    "General",
                    tab == SettingsTab::General,
                    {
                        let on_tab = on_tab.clone();
                        move |_, cx| on_tab(SettingsTab::General, cx)
                    },
                ))
                .child(nav(
                    "set-formatting",
                    "Formatting",
                    tab == SettingsTab::Formatting,
                    {
                        let on_tab = on_tab.clone();
                        move |_, cx| on_tab(SettingsTab::Formatting, cx)
                    },
                ))
                .child(nav(
                    "set-shortcuts",
                    "Shortcuts",
                    tab == SettingsTab::Shortcuts,
                    move |_, cx| on_tab(SettingsTab::Shortcuts, cx),
                )),
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
                .gap_4()
                .p_4()
                .when(tab == SettingsTab::General, |d| {
                    d.child(general_page(state.clone(), &prefs))
                })
                .when(tab == SettingsTab::Formatting, |d| {
                    d.child(formatting_page(state.clone(), &prefs))
                })
                .when(tab == SettingsTab::Shortcuts, |d| d.child(shortcuts_page())),
        )
}

fn nav(
    id: &'static str,
    label: &'static str,
    active: bool,
    on_click: impl Fn(&mut gpui::Window, &mut App) + 'static,
) -> impl IntoElement {
    settings_nav_item(id, label, active, on_click)
}

fn general_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_card(
            "Capture",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(checkbox(
                    "pref-hide",
                    "Hide the window while snipping",
                    prefs.hide_on_capture,
                    {
                        let state = state.clone();
                        let next = !prefs.hide_on_capture;
                        move |_, cx| patch_prefs(&state, cx, move |p| p.hide_on_capture = next)
                    },
                ))
                .child(
                    div()
                        .pl(px(24.))
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child("Restore when recognition finishes."),
                )
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
                .child(
                    div()
                        .pl(px(24.))
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child("Copies the primary format for this snip (Ctrl+C)."),
                ),
        ))
        .child(settings_card(
            "Window",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(checkbox(
                    "pref-close-min",
                    "Minimize on close",
                    prefs.close_action == WindowCloseAction::Minimize,
                    {
                        let state = state.clone();
                        let next = if prefs.close_action == WindowCloseAction::Minimize {
                            WindowCloseAction::Quit
                        } else {
                            WindowCloseAction::Minimize
                        };
                        move |_, cx| patch_prefs(&state, cx, move |p| p.close_action = next)
                    },
                ))
                .child(
                    div()
                        .pl(px(24.))
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child(
                        "Keeps the tray and Ctrl+Shift+S. Uncheck to quit. Ctrl+Q always quits.",
                    ),
                ),
        ))
}

fn formatting_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_card(
            "Default copy",
            setting_row(
                "Primary format",
                "Used for Ctrl+C and auto-copy. Extra formats stay available on each snip.",
                segmented([
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
                ]),
            ),
        ))
        .child(settings_card(
            "Math delimiters",
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(setting_row(
                    "Inline math",
                    "Wraps inline formulas in Markdown and mixed LaTeX export.",
                    segmented([
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
                                    patch_prefs(&state, cx, |p| p.inline_delim = InlineDelim::Paren)
                                }
                            },
                        ),
                    ]),
                ))
                .child(setting_row(
                    "Display math",
                    "Wraps longer formulas in document export. Snip copy also offers equation.",
                    segmented([
                        seg_item(
                            "pref-blk-d",
                            "$$",
                            prefs.block_delim == BlockDelim::Dollars,
                            {
                                let state = state.clone();
                                move |_, cx| {
                                    patch_prefs(&state, cx, |p| p.block_delim = BlockDelim::Dollars)
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
                    ]),
                )),
        ))
}

fn shortcuts_page() -> impl IntoElement {
    settings_card(
        "Keyboard",
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(shortcut_row("Ctrl+Shift+S", "Create snip from screenshot"))
            .child(shortcut_row("Ctrl+O", "Upload snip"))
            .child(shortcut_row("Ctrl+D", "Create snip from drawing"))
            .child(shortcut_row("Delete", "Delete selected snip"))
            .child(shortcut_row("Ctrl+,", "Settings"))
            .child(shortcut_row("Ctrl+L", "Toggle Markdown / LaTeX"))
            .child(shortcut_row("Ctrl+C", "Copy primary format")),
    )
}

fn shortcut_row(keys: &str, label: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .min_w_0()
        .child(kbd_chip(SharedString::from(keys.to_string())))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(rgb(theme::TEXT))
                .whitespace_normal()
                .child(SharedString::from(label.to_string())),
        )
}

fn patch_prefs(state: &Entity<AppState>, cx: &mut App, f: impl FnOnce(&mut Prefs)) {
    state.update(cx, |s, cx| s.update_prefs(cx, f));
}
