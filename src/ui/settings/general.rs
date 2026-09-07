use gpui::{AnyElement, Entity, div, prelude::*};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::super::widgets::switch;
use super::super::widgets::{seg_item, setting_row, settings_group};
use super::{bool_row, patch_prefs, picker};
use crate::i18n::t;
use crate::prefs::{ContentFontSize, Prefs, WindowCloseAction};
use crate::state::AppState;

pub(super) fn general_page(
    state: Entity<AppState>,
    prefs: &Prefs,
    language_picker: AnyElement,
) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            t("settings.appearance"),
            vec![
                setting_row(
                    t("settings.language"),
                    t("settings.language_hint"),
                    language_picker,
                )
                .into_any_element(),
                setting_row(
                    t("settings.font_size"),
                    t("settings.font_size_hint"),
                    picker(
                        210.,
                        [
                            seg_item(
                                "pref-font-s",
                                t("settings.font_small"),
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
                                t("settings.font_medium"),
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
                                t("settings.font_large"),
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
            t("settings.capture"),
            vec![
                bool_row(
                    &state,
                    "pref-hide",
                    "settings.hide_while_snipping",
                    "settings.hide_while_snipping_hint",
                    prefs.hide_on_capture,
                    |p, v| p.hide_on_capture = v,
                ),
                bool_row(
                    &state,
                    "pref-orig",
                    "settings.show_original",
                    "settings.show_original_hint",
                    prefs.show_original,
                    |p, v| p.show_original = v,
                ),
                bool_row(
                    &state,
                    "pref-copy",
                    "settings.autocopy",
                    "settings.autocopy_hint",
                    prefs.autocopy,
                    |p, v| p.autocopy = v,
                ),
            ],
        ))
        .child(settings_group(
            t("settings.window"),
            window_rows(&state, prefs),
        ))
}

fn window_rows(state: &Entity<AppState>, prefs: &Prefs) -> Vec<AnyElement> {
    let mut rows = vec![bool_row(
        state,
        "pref-close-min",
        "settings.minimize_on_close",
        "settings.minimize_on_close_hint",
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
        t("settings.launch_at_startup"),
        t("settings.launch_at_startup_hint"),
        switch(
            "pref-autostart",
            t("settings.launch_at_startup"),
            value,
            move |_, cx| {
                state.update(cx, |state, cx| state.set_launch_at_startup(!value, cx));
            },
        ),
    )
    .into_any_element()
}
