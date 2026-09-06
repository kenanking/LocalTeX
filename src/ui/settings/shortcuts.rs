use gpui::{AnyElement, App, Entity, SharedString, div, prelude::*, px, rgb};

use super::super::theme;
use super::super::widgets::{IconBtnSize, IconKind, btn, icon_btn_sized, kbd_chip, settings_group};
use super::SettingsPane;
use crate::keymap::{self, Group, ShortcutId};
use crate::prefs::Prefs;
use crate::state::AppState;

pub(crate) fn intercept_recording(
    settings: Entity<SettingsPane>,
    state: Entity<AppState>,
    cx: &mut App,
) {
    cx.intercept_keystrokes(move |event, _, cx| {
        let Some(id) = settings.read(cx).listening() else {
            return;
        };
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            settings.update(cx, |pane, cx| {
                pane.set_listen(None);
                cx.notify();
            });
            return;
        }
        if matches!(
            key,
            "control" | "shift" | "alt" | "platform" | "fn" | "function"
        ) {
            return;
        }
        if matches!(key, "backspace" | "delete") {
            state.update(cx, |s, cx| {
                s.unbind_shortcut(id, cx);
            });
            settings.update(cx, |pane, cx| {
                pane.set_listen(None);
                cx.notify();
            });
            return;
        }
        let chord = event.keystroke.unparse();
        state.update(cx, |s, cx| {
            let _ = s.bind_shortcut(id, chord, cx);
        });
        settings.update(cx, |pane, cx| {
            pane.set_listen(None);
            cx.notify();
        });
    })
    .detach();
}

pub(super) fn shortcuts_page<F: Fn(Option<ShortcutId>, &mut App) + Clone + 'static>(
    state: Entity<AppState>,
    prefs: &Prefs,
    listen: Option<ShortcutId>,
    on_listen: F,
) -> impl IntoElement + use<F> {
    let over = &prefs.shortcuts;
    let reset_state = state.clone();
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .px_1()
                .w_full()
                .min_w_0()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(rgb(theme::MUTED))
                        .whitespace_normal()
                        .child(
                            "Click a shortcut, then press the new keys. Esc cancels. × removes it.",
                        ),
                )
                .child(div().flex_shrink_0().child(btn(
                    "sc-reset-all",
                    "Reset all",
                    false,
                    true,
                    move |_, cx| {
                        reset_state.update(cx, |s, cx| s.reset_shortcuts(cx));
                    },
                ))),
        )
        .child(shortcut_group(
            "Capture",
            Group::Capture,
            over,
            listen,
            &state,
            &on_listen,
        ))
        .child(shortcut_group(
            "Document",
            Group::Document,
            over,
            listen,
            &state,
            &on_listen,
        ))
        .child(shortcut_group(
            "Window",
            Group::Window,
            over,
            listen,
            &state,
            &on_listen,
        ))
}

fn shortcut_group<F: Fn(Option<ShortcutId>, &mut App) + Clone + 'static>(
    title: &'static str,
    group: Group,
    over: &keymap::Overrides,
    listen: Option<ShortcutId>,
    state: &Entity<AppState>,
    on_listen: &F,
) -> impl IntoElement + use<F> {
    let rows = keymap::CATALOG
        .iter()
        .filter(|s| s.group == group)
        .map(|s| {
            shortcut_row(
                s,
                over,
                listen == Some(s.id),
                state.clone(),
                on_listen.clone(),
            )
        })
        .collect();
    settings_group(title, rows)
}

fn shortcut_row(
    spec: &keymap::Spec,
    over: &keymap::Overrides,
    listening: bool,
    state: Entity<AppState>,
    on_listen: impl Fn(Option<ShortcutId>, &mut App) + Clone + 'static,
) -> AnyElement {
    let id = spec.id;
    let chord = keymap::effective(over, id);
    let customized = keymap::is_customized(over, id);
    let mut keys = div()
        .flex()
        .items_center()
        .gap(px(4.))
        .px_2()
        .h(px(28.))
        .rounded_md()
        .when(listening, |d| {
            d.border_1()
                .border_color(rgb(theme::ACCENT))
                .bg(theme::accent_soft())
        });
    if listening {
        keys = keys
            .child(
                div()
                    .size(px(7.))
                    .rounded_full()
                    .bg(rgb(theme::ACCENT))
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .w(px(1.5))
                    .h(px(13.))
                    .bg(rgb(theme::ACCENT))
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(theme::ACCENT))
                    .child("Press a shortcut"),
            );
    } else {
        keys = match &chord {
            Some(c) => {
                let mut row = keys;
                for chip in keymap::chips(c) {
                    row = row.child(kbd_chip(chip));
                }
                row
            }
            None => keys.child(
                div()
                    .px_2()
                    .text_sm()
                    .italic()
                    .text_color(rgb(theme::MUTED))
                    .child("Unbound"),
            ),
        };
    }

    let mut right = div().flex().items_center().gap_1().child(keys);
    if !listening {
        if chord.is_some() {
            let close_state = state.clone();
            right = right.child(
                div()
                    .id(SharedString::from(format!(
                        "sc-unbind-hit-{}",
                        spec.id.as_str()
                    )))
                    .on_click(move |_, _, cx| cx.stop_propagation())
                    .child(icon_btn_sized(
                        SharedString::from(format!("sc-unbind-{}", spec.id.as_str())),
                        IconKind::Close,
                        "Remove shortcut",
                        false,
                        true,
                        IconBtnSize {
                            hit: px(24.),
                            glyph: px(14.),
                            kbd: None,
                        },
                        move |_, cx| {
                            cx.stop_propagation();
                            close_state.update(cx, |s, cx| {
                                s.unbind_shortcut(id, cx);
                            });
                        },
                    )),
            );
        }
        if customized {
            right = right.child(
                div()
                    .id(SharedString::from(format!(
                        "sc-reset-hit-{}",
                        spec.id.as_str()
                    )))
                    .on_click(move |_, _, cx| cx.stop_propagation())
                    .child(icon_btn_sized(
                        SharedString::from(format!("sc-reset-{}", spec.id.as_str())),
                        IconKind::Reset,
                        "Reset to default",
                        false,
                        true,
                        IconBtnSize {
                            hit: px(24.),
                            glyph: px(14.),
                            kbd: None,
                        },
                        move |_, cx| {
                            cx.stop_propagation();
                            state.update(cx, |s, cx| {
                                let _ = s.restore_shortcut(id, cx);
                            });
                        },
                    )),
            );
        }
    }

    div()
        .id(SharedString::from(format!("sc-bind-{}", spec.id.as_str())))
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .min_w_0()
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            on_listen(if listening { None } else { Some(id) }, cx);
            cx.stop_propagation();
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(theme::TEXT))
                        .whitespace_normal()
                        .child(SharedString::from(spec.label.to_string())),
                )
                .when(spec.global(), |d| {
                    d.child(
                        div()
                            .px_2()
                            .h(px(18.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .bg(theme::accent_soft())
                            .text_color(rgb(theme::ACCENT))
                            .text_xs()
                            .child("Global"),
                    )
                }),
        )
        .child(right)
        .into_any_element()
}
