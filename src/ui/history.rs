use gpui::{div, img, prelude::*, px, rgb, Context, MouseButton, SharedString};

use super::main_window::MainWindow;
use super::theme;
use super::widgets::{missing_image_slot, section_label, seg_item, segmented};
use crate::doc::ImageSlot;
use crate::state::DatePreset;

impl MainWindow {
    pub(crate) fn render_history(&self, n_docs: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let date_preset = state.date_preset;
        let mut list = div()
            .id("history")
            .key_context("SnipList")
            .track_focus(&self.snip_list_focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    window.focus(&this.snip_list_focus);
                }),
            )
            .w(px(168.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG))
            .border_r_1()
            .border_color(rgb(theme::BORDER));

        list = list.child(
            div()
                .flex()
                .items_center()
                .h(px(36.))
                .px_3()
                .child(section_label("Snips"))
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child(SharedString::from(n_docs.to_string())),
                ),
        );

        list = list.child(div().px_2().pb_1().child(self.search.clone()));

        let state_ent = self.state.clone();
        list = list.child(div().px_2().pb_2().child(segmented([
            seg_item("date-all", "All", date_preset == DatePreset::All, {
                let state = state_ent.clone();
                move |_, cx| {
                    state.update(cx, |s, cx| s.set_date_preset(DatePreset::All, cx));
                }
            }),
            seg_item("date-today", "Today", date_preset == DatePreset::Today, {
                let state = state_ent.clone();
                move |_, cx| {
                    state.update(cx, |s, cx| s.set_date_preset(DatePreset::Today, cx));
                }
            }),
            seg_item("date-7d", "7d", date_preset == DatePreset::Last7Days, {
                let state = state_ent.clone();
                move |_, cx| {
                    state.update(cx, |s, cx| s.set_date_preset(DatePreset::Last7Days, cx));
                }
            }),
            seg_item("date-30d", "30d", date_preset == DatePreset::Last30Days, {
                let state = state_ent.clone();
                move |_, cx| {
                    state.update(cx, |s, cx| s.set_date_preset(DatePreset::Last30Days, cx));
                }
            }),
        ])));

        let mut rows = div()
            .id("history-rows")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .pb_2()
            .overflow_y_scroll();

        if state.visible_ids.is_empty() {
            rows = rows.child(
                div()
                    .px_2()
                    .pt_4()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child("No snips in this range"),
            );
        }

        for doc in state.visible_docs() {
            let id = doc.id;
            let selected = state.selected == Some(id);
            let thumb = self.thumbs.get(&id).cloned();
            let missing = matches!(doc.image, ImageSlot::Missing) && thumb.is_none();
            let title = doc.first_line();
            let age = doc.age_label();
            let state_ent = self.state.clone();
            let list_focus = self.snip_list_focus.clone();

            rows = rows.child(
                div()
                    .id(SharedString::from(id.to_string()))
                    .flex()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme::accent_soft()))
                    .when(!selected, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        window.focus(&list_focus);
                        state_ent.update(cx, |s, cx| s.select(id, cx));
                    })
                    .child(if missing {
                        missing_image_slot(px(56.), px(40.))
                    } else {
                        div()
                            .w(px(56.))
                            .h(px(40.))
                            .rounded_sm()
                            .bg(rgb(theme::BG_SUNKEN))
                            .border_1()
                            .border_color(rgb(theme::BORDER))
                            .overflow_hidden()
                            .when_some(thumb, |d, img_data| {
                                d.child(
                                    img(img_data)
                                        .w(px(56.))
                                        .h(px(40.))
                                        .object_fit(gpui::ObjectFit::Cover),
                                )
                            })
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_ellipsis()
                                    .text_color(rgb(theme::TEXT))
                                    .child(SharedString::from(title)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme::MUTED))
                                    .child(SharedString::from(age)),
                            ),
                    ),
            );
        }
        list.child(rows)
    }
}
