use gpui::{div, prelude::*, px, rgb, AnyElement, App, Entity, SharedString};

use super::super::theme;
use super::super::widgets::{btn, icon_btn_sized, kbd_chip, settings_group, IconBtnSize, IconKind};
use crate::keymap::{self, Group, ShortcutId};
use crate::prefs::Prefs;
use crate::state::AppState;

pub(super) fn shortcuts_page(
    state: Entity<AppState>,
    prefs: &Prefs,
    listen: Option<ShortcutId>,
    on_listen: impl Fn(Option<ShortcutId>, &mut App) + Clone + 'static,
) -> impl IntoElement {
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
                .items_center()
                .justify_between()
                .gap_3()
                .px_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(theme::MUTED))
                        .child("Click a shortcut to rebind it. Esc cancels."),
                )
                .child(btn(
                    "sc-reset-all",
                    "Reset all",
                    false,
                    true,
                    move |_, cx| {
                        reset_state.update(cx, |s, cx| s.reset_shortcuts(cx));
                    },
                )),
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

fn shortcut_group(
    title: &'static str,
    group: Group,
    over: &keymap::Overrides,
    listen: Option<ShortcutId>,
    state: &Entity<AppState>,
    on_listen: &(impl Fn(Option<ShortcutId>, &mut App) + Clone + 'static),
) -> impl IntoElement {
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
        .py(px(3.))
        .rounded_md()
        .when(listening, |d| {
            d.border_1()
                .border_color(rgb(theme::ACCENT))
                .bg(theme::accent_soft())
        });
    if listening {
        keys = keys.child(
            div()
                .size(px(7.))
                .rounded_full()
                .bg(rgb(theme::ACCENT))
                .flex_shrink_0(),
        );
    }
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

    let mut right = div().flex().items_center().gap_1().child(keys);
    if customized && !listening {
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
                .when(spec.global, |d| {
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
