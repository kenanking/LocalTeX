use std::ops::Range;

use gpui::{
    div, img, prelude::*, px, rgb, uniform_list, App, Context, MouseButton, SharedString, Window,
};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::theme;
use super::widgets::{missing_image_slot, section_label, seg_item, segmented};
use crate::cache::{ROW_HEIGHT_PX, THUMB_VIEWPORT_MULT};
use crate::doc::ImageSlot;
use crate::library::DATE_PRESETS;

impl MainWindow {
    pub(crate) fn render_history(&self, n_docs: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let date_preset = state.date_preset();
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
        list = list.child(div().px_2().pb_2().child(segmented(DATE_PRESETS.map(
            |(id, label, preset)| {
                let state = state_ent.clone();
                seg_item(id, label, date_preset == preset, move |_, cx| {
                    state.update(cx, |s, cx| s.set_date_preset(preset, cx));
                })
            },
        ))));

        let ids = state.visible_ids().to_vec();
        let selected = state.selected();
        let rows = if ids.is_empty() {
            div()
                .id("history-rows")
                .flex_1()
                .min_h_0()
                .px_2()
                .pt_4()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child("No snips in this range")
                .into_any_element()
        } else {
            let media = self.media.clone();
            let state_ent = self.state.clone();
            let list_focus = self.snip_list_focus.clone();
            let thumb_keep = self.thumb_keep.clone();
            let thumb_need = self.thumb_need.clone();
            uniform_list(
                "history-rows",
                ids.len(),
                move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                    let vis = (range.end - range.start).max(1);
                    let pad = vis.saturating_mul(THUMB_VIEWPORT_MULT.saturating_sub(1)) / 2;
                    let keep_start = range.start.saturating_sub(pad);
                    let keep_end = (range.end + pad).min(ids.len());
                    *thumb_keep.borrow_mut() = ids[keep_start..keep_end].to_vec();
                    let mut need_thumbs = Vec::new();
                    {
                        let state = state_ent.read(cx);
                        for idx in keep_start..keep_end {
                            let id = ids[idx];
                            let Some(doc) = state.library.get(id) else {
                                continue;
                            };
                            if media.borrow().thumb(id).is_some() {
                                continue;
                            }
                            if !doc.thumb_jpeg.is_empty() || doc.image.pixels().is_some() {
                                media.borrow_mut().ensure_thumb(
                                    id,
                                    &doc.thumb_jpeg,
                                    doc.image.pixels().map(|p| p.as_ref()),
                                );
                            } else {
                                need_thumbs.push(id);
                            }
                        }
                    }
                    if !need_thumbs.is_empty() {
                        thumb_need.borrow_mut().extend(need_thumbs);
                    }
                    let state = state_ent.read(cx);
                    range
                        .map(|idx| {
                            let id = ids[idx];
                            history_row(
                                id,
                                selected == Some(id),
                                &state,
                                media.borrow().thumb(id),
                                list_focus.clone(),
                                state_ent.clone(),
                            )
                        })
                        .collect()
                },
            )
            .h_full()
            .into_any_element()
        };

        list.child(
            div()
                .id("history-rows-wrap")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .px_2()
                .pb_2()
                .child(rows),
        )
    }
}

fn history_row(
    id: Uuid,
    selected: bool,
    state: &crate::state::AppState,
    thumb: Option<std::sync::Arc<gpui::RenderImage>>,
    list_focus: gpui::FocusHandle,
    state_ent: gpui::Entity<crate::state::AppState>,
) -> gpui::AnyElement {
    let doc = state.library.get(id);
    let missing = doc.is_some_and(|d| matches!(d.image, ImageSlot::Missing)) && thumb.is_none();
    let title = doc.map(|d| d.first_line()).unwrap_or_default();
    let age = doc.map(|d| d.age_label()).unwrap_or_default();

    div()
        .id(SharedString::from(id.to_string()))
        .h(px(ROW_HEIGHT_PX))
        .flex()
        .items_center()
        .gap_2()
        .px_2()
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
        )
        .into_any_element()
}
