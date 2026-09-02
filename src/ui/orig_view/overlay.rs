use gpui::{
    canvas, div, img, prelude::*, px, rgb, rgba, svg, Context, CursorStyle, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent, SharedString, Window,
};
use uuid::Uuid;

use super::super::main_window::MainWindow;
use super::super::scroll::{overlay_scrollbar, ScrollAxis, ScrollbarTone};
use super::super::theme;
use super::super::widgets::IconKind;
use super::chrome::{hud_pill, orig_action_capsule, orig_hud_disc};
use super::geom::{
    film_content_w, film_pan_offset, zoom_factor_for_wheel, zoom_percent, FILM_GAP, FILM_H,
    FILM_PAD_X, FILM_THUMB_H, FILM_THUMB_W, ZOOM_PILL_W,
};
use crate::doc::ImageSlot;

impl MainWindow {
    pub(crate) fn render_orig_overlay(
        &mut self,
        window: &Window,
        workspace_h: f32,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        self.ensure_selected_full(cx);
        let (ids, selected, age, idx, copy_flashed, reveal_enabled) = {
            let state = self.state.read(cx);
            let ids = state.visible_snapshot();
            let selected = state.selected();
            let age = state
                .selected_doc()
                .map(|d| d.age_label())
                .unwrap_or_default();
            let idx = selected.and_then(|id| ids.iter().position(|x| *x == id));
            let copy_flashed = selected.is_some_and(|id| state.orig_copy_flashed(id));
            let reveal_enabled = selected.is_some_and(|id| state.can_reveal_original(id));
            (ids, selected, age, idx, copy_flashed, reveal_enabled)
        };
        if let Some(id) = selected {
            if self.orig.on_new_image(id) {
                if let Some(i) = idx {
                    self.orig.request_film_reveal(i);
                }
            }
        }
        self.orig.apply_film_reveal();
        let full = selected.and_then(|id| self.full(id));
        if let Some(ref im) = full {
            let s = im.size(0);
            self.orig
                .apply_image_size(u32::from(s.width) as f32, u32::from(s.height) as f32);
        }
        let viewport_w: f32 = window.viewport_size().width.into();
        self.orig
            .seed_stage_if_empty(viewport_w, (workspace_h - FILM_H).max(1.0));
        let n = ids.len();
        let at_start = !idx.is_some_and(|i| i > 0);
        let at_end = !idx.is_some_and(|i| i + 1 < n);
        let counter = match idx {
            Some(i) => format!("{} / {n}  ·  {age}", i + 1),
            None => format!("— / {n}"),
        };
        let pct = if self.orig.fit > 0.0 {
            zoom_percent(self.orig.scale, self.orig.fit)
        } else {
            100
        };

        let thumbs = {
            let media = self.media.cache.borrow();
            ids.iter()
                .map(|id| (*id, media.thumb(*id)))
                .collect::<Vec<_>>()
        };
        self.ensure_film_thumbs(&ids, idx, cx);

        div()
            .id("orig-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .track_focus(&self.orig_focus)
            .key_context("OrigView")
            .flex()
            .flex_col()
            .bg(theme::overlay_scrim())
            .text_color(rgb(0xececef))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if !this.orig.has_pointer() {
                    return;
                }
                this.orig.pointer_move(
                    f32::from(ev.position.x),
                    f32::from(ev.position.y),
                    ev.dragging(),
                );
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, window, cx| {
                    this.orig_pointer_up(window, cx);
                }),
            )
            .child(self.render_orig_stage(full, at_start, at_end, cx))
            .child(self.render_orig_hud(&counter, pct, selected, copy_flashed, reveal_enabled, cx))
            .child(self.render_orig_film(&thumbs, selected, cx))
    }

    fn ensure_film_thumbs(&self, ids: &[Uuid], idx: Option<usize>, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let center = idx.unwrap_or(0);
        let start = center.saturating_sub(16);
        let end = (center + 17).min(ids.len());
        let window_ids = ids[start..end].to_vec();
        {
            let mut keep = self.media.thumb_keep.borrow_mut();
            for id in &window_ids {
                if !keep.contains(id) {
                    keep.push(*id);
                }
            }
        }
        let mut need = Vec::new();
        {
            let state = self.state.read(cx);
            let media = self.media.cache.borrow();
            for id in &window_ids {
                if media.thumb(*id).is_some() {
                    continue;
                }
                let Some(doc) = state.doc(*id) else {
                    continue;
                };
                if matches!(doc.image, ImageSlot::Missing) {
                    continue;
                }
                need.push(*id);
            }
        }
        if need.is_empty() {
            return;
        }
        self.ensure_thumbs(&need, cx);
    }

    fn render_orig_stage(
        &self,
        full: Option<std::sync::Arc<gpui::RenderImage>>,
        at_start: bool,
        at_end: bool,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let entity = cx.entity();
        let img_data = full.clone();
        let img_w = self.orig.img_w;
        let img_h = self.orig.img_h;
        let scale = self.orig.scale;
        let tx = self.orig.tx;
        let ty = self.orig.ty;
        let panning = self.orig.is_image_panning();
        div()
            .id("orig-stage")
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .cursor(if panning {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::PointingHand
            })
            .child(
                canvas(
                    {
                        let entity = entity.clone();
                        let img_data = img_data.clone();
                        move |bounds, _, cx| {
                            let (iw, ih) = img_data
                                .as_ref()
                                .map(|im| {
                                    let s = im.size(0);
                                    (u32::from(s.width) as f32, u32::from(s.height) as f32)
                                })
                                .unwrap_or((0.0, 0.0));
                            entity.update(cx, |this, cx| {
                                if this.orig.set_geometry(iw, ih, bounds) {
                                    cx.notify();
                                }
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .size_full(),
            )
            .when_some(full, |d, img_data| {
                d.child(
                    img(img_data)
                        .absolute()
                        .left(px(tx))
                        .top(px(ty))
                        .w(px((img_w * scale).max(1.0)))
                        .h(px((img_h * scale).max(1.0)))
                        .object_fit(gpui::ObjectFit::Fill),
                )
            })
            .child(nav_disc(
                "orig-prev",
                IconKind::Collapse,
                true,
                at_start,
                cx,
            ))
            .child(nav_disc("orig-next", IconKind::Expand, false, at_end, cx))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                let dy: f32 = ev.delta.pixel_delta(window.line_height()).y.into();
                if dy.abs() < 0.2 {
                    return;
                }
                let factor = zoom_factor_for_wheel(dy);
                let x = f32::from(ev.position.x) - this.orig.stage_ox;
                let y = f32::from(ev.position.y) - this.orig.stage_oy;
                this.orig.zoom_at(x, y, factor);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.orig_focus, cx);
                    this.orig
                        .begin_drag(f32::from(ev.position.x), f32::from(ev.position.y));
                    cx.notify();
                }),
            )
    }

    fn render_orig_hud(
        &self,
        counter: &str,
        pct: i32,
        selected: Option<Uuid>,
        copy_flashed: bool,
        reveal_enabled: bool,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let entity = cx.entity();
        div()
            .absolute()
            .top(px(12.))
            .left(px(12.))
            .right(px(12.))
            .flex()
            .items_center()
            .gap_2()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(hud_pill().child(SharedString::from(counter.to_string())))
            .child(
                hud_pill()
                    .id("orig-zoom-pct")
                    .w(px(ZOOM_PILL_W))
                    .flex_shrink_0()
                    .justify_center()
                    .font_family("monospace")
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.orig.fit_in_stage();
                        cx.notify();
                    }))
                    .hover(|d| d.bg(theme::hud_pill_hover()))
                    .child(SharedString::from(format!("{pct}%"))),
            )
            .child(div().flex_1())
            .when_some(selected, |d, doc_id| {
                d.child(orig_action_capsule(
                    doc_id,
                    copy_flashed,
                    reveal_enabled,
                    "hud",
                    cx,
                ))
            })
            .child(orig_hud_disc("hud-close", IconKind::Close, "Close", {
                move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.unzoom();
                        window.focus(&this.snip_list_focus, cx);
                        cx.notify();
                    });
                }
            }))
    }

    fn render_orig_film(
        &self,
        thumbs: &[(Uuid, Option<std::sync::Arc<gpui::RenderImage>>)],
        selected: Option<Uuid>,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let content_w = film_content_w(thumbs.len());
        let entity = cx.entity();
        let mut row = div()
            .id("orig-film-inner")
            .w(px(content_w))
            .h(px(FILM_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(FILM_GAP))
            .px(px(FILM_PAD_X));
        for (id, thumb) in thumbs {
            let id = *id;
            let on = selected == Some(id);
            row = row.child(
                div()
                    .id(SharedString::from(format!("film-{id}")))
                    .p(px(2.))
                    .flex_shrink_0()
                    .rounded_md()
                    .cursor_pointer()
                    .opacity(if on { 1.0 } else { 0.7 })
                    .when(on, |d| d.bg(theme::film_selected_ring()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.orig.begin_film_pan(f32::from(ev.position.x), Some(id));
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(FILM_THUMB_W))
                            .h(px(FILM_THUMB_H))
                            .rounded_sm()
                            .overflow_hidden()
                            .when_some(thumb.clone(), |d, img_data| {
                                d.child(
                                    img(img_data)
                                        .w(px(FILM_THUMB_W))
                                        .h(px(FILM_THUMB_H))
                                        .object_fit(gpui::ObjectFit::Cover),
                                )
                            }),
                    ),
            );
        }
        div()
            .id("orig-film")
            .relative()
            .h(px(FILM_H))
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .bg(rgba(0x1010146b))
            .cursor(if self.orig.is_film_panning() {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::OpenHand
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.orig.begin_film_pan(f32::from(ev.position.x), None);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    {
                        let entity = entity.clone();
                        move |bounds, _, cx| {
                            let w: f32 = bounds.size.width.into();
                            entity.update(cx, |this, cx| {
                                if (this.orig.film_view_w - w).abs() > 0.5 {
                                    this.orig.film_view_w = w;
                                    cx.notify();
                                }
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .id("orig-film-h")
                    .w_full()
                    .h(px(FILM_H))
                    .min_w_0()
                    .overflow_x_hidden()
                    .track_scroll(&self.orig.film)
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                        let delta = ev.delta.pixel_delta(window.line_height());
                        let dx: f32 = delta.x.into();
                        let dy: f32 = delta.y.into();
                        let pan = if dx.abs() > 0.5 { dx } else { dy };
                        cx.stop_propagation();
                        if !pan.is_finite() || pan.abs() < 0.5 {
                            return;
                        }
                        let max_x: f32 = this.orig.film.max_offset().x.into();
                        if !max_x.is_finite() || max_x <= 1.0 {
                            return;
                        }
                        let x: f32 = this.orig.film.offset().x.into();
                        let mut off = this.orig.film.offset();
                        off.x = px(film_pan_offset(x, pan, max_x));
                        this.orig.film.set_offset(off);
                        cx.notify();
                    }))
                    .child(row),
            )
            .child(overlay_scrollbar(
                "orig-film-thumb",
                ScrollAxis::Horizontal,
                &self.orig.film,
                &self.orig.film_thumb,
                true,
                ScrollbarTone::OnDark,
            ))
    }
}

fn nav_disc(
    id: &'static str,
    kind: IconKind,
    prev: bool,
    disabled: bool,
    cx: &mut Context<MainWindow>,
) -> impl gpui::IntoElement {
    div()
        .id(id)
        .absolute()
        .when(prev, |d| d.left(px(16.)))
        .when(!prev, |d| d.right(px(16.)))
        .top(gpui::relative(0.5))
        .mt(px(-18.))
        .size(px(36.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::nav_disc())
        .border_1()
        .border_color(rgba(0xfffffff2))
        .shadow_sm()
        .when(disabled, |d| d.opacity(0.4))
        .when(!disabled, |d| {
            d.cursor_pointer().hover(|d| d.bg(rgb(0xffffff)))
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(!disabled, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| {
                this.state.update(cx, |state, cx| {
                    state.select_delta(if prev { -1 } else { 1 }, cx)
                });
                cx.notify();
            }))
        })
        .child(
            svg()
                .path(kind.asset_path())
                .size(px(15.))
                .text_color(rgb(theme::TEXT)),
        )
}
