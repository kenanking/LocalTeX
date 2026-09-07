use gpui::{
    AnyElement, Context, Entity, MouseButton, Window, anchored, deferred, div, point, prelude::*,
    px, rgb, svg,
};

use super::{SettingsPane, patch_ui_lang};
use crate::i18n::{UiLang, t};
use crate::state::AppState;
use crate::ui::{theme, widgets::IconKind};

impl SettingsPane {
    fn step_language(&mut self, selected: UiLang, backwards: bool, cx: &mut Context<Self>) {
        if self.language_open {
            let step = if backwards { UiLang::ALL.len() - 1 } else { 1 };
            self.language_index = (self.language_index + step) % UiLang::ALL.len();
        } else {
            self.language_open = true;
            self.language_index = UiLang::ALL
                .iter()
                .position(|&lang| lang == selected)
                .unwrap_or(0);
        }
        cx.notify();
    }

    pub(super) fn language_picker(
        &self,
        state: Entity<AppState>,
        selected: UiLang,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.language_open;
        let focused = self.language_focus.is_focused(window);
        let keyboard_state = state.clone();
        div()
            .relative()
            .w(px(180.))
            .child(
                div()
                    .id("pref-language")
                    .role(gpui::Role::ComboBox)
                    .aria_label(t("settings.language"))
                    .aria_value(selected.picker_label())
                    .aria_expanded(open)
                    .track_focus(&self.language_focus)
                    .on_action(
                        cx.listener(move |this, _: &crate::actions::SelectNext, _, cx| {
                            this.step_language(selected, false, cx);
                        }),
                    )
                    .on_action(
                        cx.listener(move |this, _: &crate::actions::SelectPrev, _, cx| {
                            this.step_language(selected, true, cx);
                        }),
                    )
                    .on_action(cx.listener(|this, _: &crate::actions::CloseSheet, _, cx| {
                        if this.language_open {
                            this.language_open = false;
                            cx.notify();
                        } else {
                            cx.propagate();
                        }
                    }))
                    .w_full()
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if focused || open {
                        theme::ACCENT
                    } else {
                        theme::BORDER
                    }))
                    .bg(rgb(theme::BG_RAISED))
                    .text_sm()
                    .text_color(rgb(theme::TEXT))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            window.focus(&this.language_focus, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .on_click(cx.listener(move |this, event, _, cx| {
                        // GPUI maps Enter/Space to ClickEvent::Keyboard on key release.
                        if matches!(event, gpui::ClickEvent::Keyboard(_)) && this.language_open {
                            patch_ui_lang(&keyboard_state, cx, UiLang::ALL[this.language_index]);
                            this.language_open = false;
                        } else {
                            this.language_open = !open;
                            this.language_index = UiLang::ALL
                                .iter()
                                .position(|&lang| lang == selected)
                                .unwrap_or(0);
                        }
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        let key = event.keystroke.key.as_str();
                        match key {
                            "home" | "end" => {
                                this.language_open = true;
                                this.language_index = if key == "home" {
                                    0
                                } else {
                                    UiLang::ALL.len() - 1
                                };
                            }
                            "tab" => {
                                this.language_open = false;
                                cx.notify();
                                return;
                            }
                            _ => return,
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .child(selected.picker_label())
                    .child(
                        svg()
                            .path(IconKind::Expand.asset_path())
                            .size(px(14.))
                            .text_color(rgb(theme::MUTED))
                            .with_transformation(gpui::Transformation::rotate(gpui::Radians(
                                std::f32::consts::FRAC_PI_2,
                            ))),
                    ),
            )
            .when(open, |d| {
                d.child(
                    deferred(
                        anchored()
                            .offset(point(px(0.), px(4.)))
                            .snap_to_window()
                            .child(
                                div()
                                    .id("language-options")
                                    .role(gpui::Role::ListBox)
                                    .aria_label(t("settings.language"))
                                    .w(px(180.))
                                    .p_1()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(theme::BORDER))
                                    .bg(rgb(theme::BG_RAISED))
                                    .occlude()
                                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                        this.language_open = false;
                                        cx.notify();
                                    }))
                                    .children(UiLang::ALL.into_iter().enumerate().map(
                                        |(index, lang)| {
                                            let state = state.clone();
                                            div()
                                                .id(("language-option", index))
                                                .role(gpui::Role::ListBoxOption)
                                                .aria_selected(lang == selected)
                                                .h(px(32.))
                                                .px_2()
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .rounded_sm()
                                                .text_sm()
                                                .text_color(rgb(theme::TEXT))
                                                .when(index == self.language_index, |d| {
                                                    d.bg(rgb(theme::ACCENT_SOFT_FILL))
                                                })
                                                .hover(|s| s.bg(rgb(theme::BG_SUNKEN)))
                                                .cursor_pointer()
                                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                    cx.stop_propagation()
                                                })
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.language_open = false;
                                                        patch_ui_lang(&state, cx, lang);
                                                        window.focus(&this.language_focus, cx);
                                                        cx.notify();
                                                    },
                                                ))
                                                .child(lang.picker_label())
                                                .when(lang == selected, |d| {
                                                    d.child(
                                                        svg()
                                                            .path(IconKind::Check.asset_path())
                                                            .size(px(14.))
                                                            .text_color(rgb(theme::ACCENT)),
                                                    )
                                                })
                                        },
                                    )),
                            ),
                    )
                    .with_priority(1),
                )
            })
            .into_any_element()
    }
}
