use gpui::{AnyElement, Entity, div, prelude::*};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::super::widgets::switch;
use super::super::widgets::{seg_item, setting_row, settings_group};
use super::{bool_row, patch_prefs, picker};
use crate::prefs::{ContentFontSize, Prefs, WindowCloseAction};
use crate::state::AppState;

pub(super) fn general_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            "Appearance",
            vec![
                setting_row(
                    "Font size",
                    "Source editor and preview use the same size.",
                    picker(
                        210.,
                        [
                            seg_item(
                                "pref-font-s",
                                "Small",
                                prefs.content_font == ContentFontSize::Small,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.content_font = ContentFontSize::Small
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-font-m",
                                "Medium",
                                prefs.content_font == ContentFontSize::Medium,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.content_font = ContentFontSize::Medium
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-font-l",
                                "Large",
                                prefs.content_font == ContentFontSize::Large,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.content_font = ContentFontSize::Large
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
        .child(settings_group(
            "Capture",
            vec![
                bool_row(
                    &state,
                    "pref-hide",
                    "Hide window while snipping",
                    "Show the window as soon as you finish selecting.",
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
                    "Copies the last text format you used for this kind of snip.",
                    prefs.autocopy,
                    |p, v| p.autocopy = v,
                ),
            ],
        ))
        .child(settings_group("Window", window_rows(&state, prefs)))
}

fn window_rows(state: &Entity<AppState>, prefs: &Prefs) -> Vec<AnyElement> {
    let mut rows = vec![bool_row(
        state,
        "pref-close-min",
        "Minimize on close",
        "Hides to the tray with no taskbar icon. Off quits; Ctrl+Q always quits.",
        prefs.close_action == WindowCloseAction::Minimize,
        |p, v| {
            p.close_action = if v {
                WindowCloseAction::Minimize
            } else {
                WindowCloseAction::Quit
            };
        },
    )];
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    rows.push(autostart_row(state, prefs.launch_at_startup));
    rows
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn autostart_row(state: &Entity<AppState>, value: bool) -> AnyElement {
    let state = state.clone();
    setting_row(
        "Launch at startup",
        "Keeps LocalTeX ready in the tray when you sign in.",
        switch(
            "pref-autostart",
            "Launch at startup",
            value,
            move |_, cx| {
                state.update(cx, |state, cx| state.set_launch_at_startup(!value, cx));
            },
        ),
    )
    .into_any_element()
}
