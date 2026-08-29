//! Original-image gallery overlay: contain-fit, wheel zoom, pan, click-to-close.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    canvas, div, img, prelude::*, px, rgb, rgba, svg, Bounds, Context, CursorStyle, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollHandle, ScrollWheelEvent,
    SharedString, Window,
};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::scroll::{overlay_scrollbar, ScrollAxis, ScrollThumbDrag, ScrollbarTone};
use super::theme;
use super::widgets::IconKind;
use crate::doc::ImageSlot;

pub(crate) const ZOOM_MIN: f32 = 0.5;
pub(crate) const ZOOM_MAX: f32 = 8.0;
pub(crate) const CLICK_SLOP: f32 = 6.0;
pub(crate) const ZOOM_PILL_W: f32 = 64.0;
pub(crate) const ZOOM_STEP: f32 = 1.12;

/// GPUI reports wheel-up as **positive** `delta.y` (X11 `ScrollDirection::Up`,
/// Win32 `WHEEL_DELTA`). CSS `wheel.deltaY` is the opposite sign.
pub(crate) fn zoom_factor_for_wheel(dy: f32) -> f32 {
    if dy > 0.0 {
        ZOOM_STEP
    } else {
        1.0 / ZOOM_STEP
    }
}

const FILM_H: f32 = 72.0;
const FILM_THUMB_W: f32 = 88.0;
const FILM_THUMB_H: f32 = 56.0;
const FILM_GAP: f32 = 8.0;
const FILM_PAD_X: f32 = 16.0;
const FILM_CELL_PAD: f32 = 4.0;

pub(crate) fn film_content_w(n: usize) -> f32 {
    if n == 0 {
        return FILM_PAD_X * 2.0;
    }
    let n = n as f32;
    FILM_PAD_X * 2.0 + n * (FILM_THUMB_W + FILM_CELL_PAD) + (n - 1.0) * FILM_GAP
}

/// Grab-the-strip pan: content follows the pointer 1:1. `offset` is GPUI's
/// negative scroll offset, clamped to `[-max, 0]`.
pub(crate) fn film_pan_offset(start_off: f32, dx: f32, max: f32) -> f32 {
    let max = if max.is_finite() && max > 0.0 {
        max
    } else {
        0.0
    };
    let next = start_off + dx;
    if !next.is_finite() {
        return 0.0;
    }
    next.clamp(-max, 0.0)
}

pub(crate) fn fit_scale(img_w: f32, img_h: f32, stage_w: f32, stage_h: f32) -> f32 {
    if img_w <= 0.0 || img_h <= 0.0 || stage_w <= 0.0 || stage_h <= 0.0 {
        return 1.0;
    }
    let s = (stage_w / img_w).min(stage_h / img_h);
    if s.is_finite() && s > 0.0 {
        s
    } else {
        1.0
    }
}

pub(crate) fn zoom_percent(scale: f32, fit: f32) -> i32 {
    if !fit.is_finite() || fit <= 0.0 || !scale.is_finite() {
        return 100;
    }
    ((scale / fit) * 100.0).round() as i32
}

pub(crate) fn release_is_click(drag_dist: f32) -> bool {
    drag_dist < CLICK_SLOP
}

pub(crate) struct OrigDrag {
    pub origin_x: f32,
    pub origin_y: f32,
    pub start_tx: f32,
    pub start_ty: f32,
    pub dist: f32,
}

struct FilmPan {
    origin_x: f32,
    start_off: f32,
    dist: f32,
    pending_id: Option<Uuid>,
}

pub(crate) struct OrigView {
    pub open: bool,
    pub strip_hover: bool,
    pub scale: f32,
    pub tx: f32,
    pub ty: f32,
    pub fit: f32,
    pub img_w: f32,
    pub img_h: f32,
    pub stage_w: f32,
    pub stage_h: f32,
    pub stage_ox: f32,
    pub stage_oy: f32,
    pub drag: Option<OrigDrag>,
    pub film: ScrollHandle,
    pub film_thumb: Rc<RefCell<Option<ScrollThumbDrag>>>,
    film_pan: Option<FilmPan>,
    film_view_w: f32,
    shown_id: Option<Uuid>,
}

impl OrigView {
    pub fn new() -> Self {
        Self {
            open: false,
            strip_hover: false,
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
            fit: 0.0,
            img_w: 0.0,
            img_h: 0.0,
            stage_w: 0.0,
            stage_h: 0.0,
            stage_ox: 0.0,
            stage_oy: 0.0,
            drag: None,
            film: ScrollHandle::new(),
            film_thumb: Rc::new(RefCell::new(None)),
            film_pan: None,
            film_view_w: 0.0,
            shown_id: None,
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.strip_hover = false;
        self.drag = None;
        self.film_pan = None;
        self.film_view_w = 0.0;
        self.shown_id = None;
        self.fit = 0.0;
        let mut off = self.film.offset();
        off.x = px(0.);
        off.y = px(0.);
        self.film.set_offset(off);
    }

    pub fn open_view(&mut self) {
        self.open = true;
        self.strip_hover = false;
        self.drag = None;
        self.film_pan = None;
        self.film_view_w = 0.0;
        self.shown_id = None;
        self.fit = 0.0;
    }

    pub fn fit_in_stage(&mut self) {
        self.fit = fit_scale(self.img_w, self.img_h, self.stage_w, self.stage_h);
        self.scale = self.fit;
        self.tx = (self.stage_w - self.img_w * self.scale) * 0.5;
        self.ty = (self.stage_h - self.img_h * self.scale) * 0.5;
    }

    /// Return true if stage/image size changed enough to warrant `cx.notify()`.
    pub fn set_geometry(&mut self, img_w: f32, img_h: f32, stage: Bounds<Pixels>) -> bool {
        let stage_w = f32::from(stage.size.width);
        let stage_h = f32::from(stage.size.height);
        let stage_ox = f32::from(stage.origin.x);
        let stage_oy = f32::from(stage.origin.y);
        let same = (self.img_w - img_w).abs() < 0.5
            && (self.img_h - img_h).abs() < 0.5
            && (self.stage_w - stage_w).abs() < 0.5
            && (self.stage_h - stage_h).abs() < 0.5
            && (self.stage_ox - stage_ox).abs() < 0.5
            && (self.stage_oy - stage_oy).abs() < 0.5;
        if same {
            return false;
        }
        let img_changed = (self.img_w - img_w).abs() > 0.5 || (self.img_h - img_h).abs() > 0.5;
        let was_fit = self.fit <= 0.0 || (self.scale - self.fit).abs() < 0.02 * self.fit.max(1e-3);
        self.img_w = img_w;
        self.img_h = img_h;
        self.stage_w = stage_w;
        self.stage_h = stage_h;
        self.stage_ox = stage_ox;
        self.stage_oy = stage_oy;
        if img_changed || was_fit {
            self.fit_in_stage();
        } else {
            let rel = self.scale / self.fit.max(1e-6);
            self.fit = fit_scale(img_w, img_h, stage_w, stage_h);
            self.scale = (self.fit * rel).clamp(self.fit * ZOOM_MIN, self.fit * ZOOM_MAX);
        }
        true
    }

    pub fn on_new_image(&mut self, id: Uuid) {
        if self.shown_id == Some(id) {
            return;
        }
        self.shown_id = Some(id);
        self.drag = None;
        self.fit = 0.0;
        if self.stage_w > 0.0 && self.img_w > 0.0 {
            self.fit_in_stage();
        }
    }

    pub fn apply_image_size(&mut self, img_w: f32, img_h: f32) {
        if (self.img_w - img_w).abs() < 0.5 && (self.img_h - img_h).abs() < 0.5 {
            if self.fit <= 0.0 && self.stage_w > 1.0 && img_w > 0.0 {
                self.fit_in_stage();
            }
            return;
        }
        self.img_w = img_w;
        self.img_h = img_h;
        if self.stage_w > 1.0 {
            self.fit_in_stage();
        }
    }

    pub fn seed_stage_if_empty(&mut self, w: f32, h: f32) {
        if self.stage_w > 1.0 || w < 1.0 || h < 1.0 {
            return;
        }
        self.stage_w = w;
        self.stage_h = h;
        if self.img_w > 0.0 {
            self.fit_in_stage();
        }
    }

    pub fn zoom_at(&mut self, cursor_x: f32, cursor_y: f32, factor: f32) {
        if self.fit <= 0.0 || self.scale <= 0.0 || !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let old = self.scale;
        let next = (old * factor).clamp(self.fit * ZOOM_MIN, self.fit * ZOOM_MAX);
        if (next - old).abs() < 1e-6 {
            return;
        }
        let k = next / old;
        self.tx = cursor_x - (cursor_x - self.tx) * k;
        self.ty = cursor_y - (cursor_y - self.ty) * k;
        self.scale = next;
    }

    pub fn begin_drag(&mut self, x: f32, y: f32) {
        self.drag = Some(OrigDrag {
            origin_x: x,
            origin_y: y,
            start_tx: self.tx,
            start_ty: self.ty,
            dist: 0.0,
        });
    }

    pub fn drag_to(&mut self, x: f32, y: f32) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let dx = x - drag.origin_x;
        let dy = y - drag.origin_y;
        drag.dist = dx.hypot(dy);
        self.tx = drag.start_tx + dx;
        self.ty = drag.start_ty + dy;
    }

    /// `true` if the pointer-up should close the overlay.
    pub fn end_drag(&mut self) -> bool {
        let dist = self.drag.take().map(|d| d.dist).unwrap_or(0.0);
        release_is_click(dist)
    }

    pub fn is_film_panning(&self) -> bool {
        self.film_pan.is_some()
    }

    pub fn begin_film_pan(&mut self, x: f32, pending_id: Option<Uuid>) {
        let start_off: f32 = self.film.offset().x.into();
        self.film_pan = Some(FilmPan {
            origin_x: x,
            start_off,
            dist: 0.0,
            pending_id,
        });
    }

    pub fn drag_film(&mut self, x: f32) {
        let Some(pan) = self.film_pan.as_mut() else {
            return;
        };
        let dx = x - pan.origin_x;
        pan.dist = dx.abs();
        let max: f32 = self.film.max_offset().width.into();
        let new_off = film_pan_offset(pan.start_off, dx, max);
        let mut off = self.film.offset();
        off.x = px(new_off);
        self.film.set_offset(off);
    }

    /// Clicked thumbnail id when the gesture stayed inside `CLICK_SLOP`.
    pub fn end_film_pan(&mut self) -> Option<Uuid> {
        let pan = self.film_pan.take()?;
        if release_is_click(pan.dist) {
            pan.pending_id
        } else {
            None
        }
    }

    /// Keep panning while the left button is held. If the button is up, drop
    /// the gesture without treating it as a thumbnail click — GPUI does not
    /// deliver `MouseUp` when the release happens outside the window.
    pub fn continue_film_pan(&mut self, x: f32, left_down: bool) {
        if self.film_pan.is_none() {
            return;
        }
        if !left_down {
            self.film_pan = None;
            return;
        }
        self.drag_film(x);
    }
}

impl MainWindow {
    pub(crate) fn render_orig_overlay(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        self.ensure_selected_full(cx);
        let (ids, selected, age, idx) = {
            let state = self.state.read(cx);
            let ids = state.visible_ids().to_vec();
            let selected = state.selected();
            let age = state
                .selected_doc()
                .map(|d| d.age_label())
                .unwrap_or_default();
            let idx = selected.and_then(|id| ids.iter().position(|x| *x == id));
            (ids, selected, age, idx)
        };
        if let Some(id) = selected {
            self.orig.on_new_image(id);
        }
        let full = selected.and_then(|id| self.full(id));
        if let Some(ref im) = full {
            let s = im.size(0);
            self.orig
                .apply_image_size(u32::from(s.width) as f32, u32::from(s.height) as f32);
        }
        let win_w: f32 = window.bounds().size.width.into();
        let win_h: f32 = window.bounds().size.height.into();
        self.orig
            .seed_stage_if_empty(win_w, (win_h - 44.0 - 28.0 - FILM_H).max(1.0));
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
        let panning = self.orig.drag.is_some();
        let img_w = self.orig.img_w;
        let img_h = self.orig.img_h;
        let scale = self.orig.scale;
        let tx = self.orig.tx;
        let ty = self.orig.ty;

        let thumbs = {
            let media = self.media.borrow();
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
                if this.orig.is_film_panning() {
                    this.orig
                        .continue_film_pan(f32::from(ev.position.x), ev.dragging());
                    cx.notify();
                    return;
                }
                if this.orig.drag.is_some() {
                    if !ev.dragging() {
                        this.orig.drag = None;
                        cx.notify();
                        return;
                    }
                    this.orig
                        .drag_to(f32::from(ev.position.x), f32::from(ev.position.y));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, window, cx| {
                    if this.orig.film_pan.is_some() {
                        if let Some(id) = this.orig.end_film_pan() {
                            this.state.update(cx, |s, cx| s.select(id, cx));
                        }
                        cx.notify();
                        return;
                    }
                    if this.orig.drag.is_none() {
                        return;
                    }
                    if this.orig.end_drag() {
                        this.unzoom();
                        window.focus(&this.snip_list_focus);
                    }
                    cx.notify();
                }),
            )
            .child(self.render_orig_stage(
                full, img_w, img_h, scale, tx, ty, panning, at_start, at_end, cx,
            ))
            .child(self.render_orig_hud(&counter, pct, cx))
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
            let mut keep = self.thumb_keep.borrow_mut();
            for id in &window_ids {
                if !keep.contains(id) {
                    keep.push(*id);
                }
            }
        }
        let mut need = Vec::new();
        {
            let state = self.state.read(cx);
            let media = self.media.borrow();
            for id in &window_ids {
                if media.thumb(*id).is_some() {
                    continue;
                }
                let Some(doc) = state.library.get(*id) else {
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
        let entity = cx.entity();
        cx.defer(move |cx| {
            entity.update(cx, |this, cx| {
                let mut request = Vec::new();
                {
                    let state = this.state.read(cx);
                    for id in need {
                        let Some(doc) = state.library.get(id) else {
                            continue;
                        };
                        if !doc.thumb_jpeg.is_empty() || doc.image.pixels().is_some() {
                            this.media.borrow_mut().ensure_thumb(
                                id,
                                &doc.thumb_jpeg,
                                doc.image.pixels().map(|p| p.as_ref()),
                            );
                        } else {
                            request.push(id);
                        }
                    }
                }
                for id in request {
                    this.state.update(cx, |s, cx| s.request_thumb(id, cx));
                }
                this.schedule_media_gc(cx);
                cx.notify();
            });
        });
    }

    fn render_orig_stage(
        &self,
        full: Option<std::sync::Arc<gpui::RenderImage>>,
        img_w: f32,
        img_h: f32,
        scale: f32,
        tx: f32,
        ty: f32,
        panning: bool,
        at_start: bool,
        at_end: bool,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        let entity = cx.entity();
        let img_data = full.clone();
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
                    window.focus(&this.orig_focus);
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
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
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
            .child(
                div()
                    .id("orig-close")
                    .size(px(28.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::hud_pill())
                    .border_1()
                    .border_color(rgba(0xffffff14))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.unzoom();
                        window.focus(&this.snip_list_focus);
                        cx.notify();
                    }))
                    .hover(|d| d.bg(theme::hud_pill_hover()))
                    .child(
                        svg()
                            .path(IconKind::Close.asset_path())
                            .size(px(14.))
                            .text_color(rgb(0xffffff)),
                    ),
            )
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
            .cursor(if self.orig.film_pan.is_some() {
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
                        let max_x: f32 = this.orig.film.max_offset().width.into();
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

fn hud_pill() -> gpui::Div {
    div()
        .h(px(26.))
        .px(px(10.))
        .rounded_full()
        .flex()
        .items_center()
        .bg(theme::hud_pill())
        .border_1()
        .border_color(rgba(0xffffff14))
        .text_color(rgb(0xececef))
        .text_xs()
        .whitespace_nowrap()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_is_contain() {
        assert!((fit_scale(200.0, 100.0, 100.0, 100.0) - 0.5).abs() < 1e-4);
        assert!((fit_scale(100.0, 200.0, 100.0, 100.0) - 0.5).abs() < 1e-4);
        assert_eq!(fit_scale(0.0, 10.0, 100.0, 100.0), 1.0);
    }

    #[test]
    fn wheel_up_zooms_in() {
        assert!(zoom_factor_for_wheel(3.0) > 1.0);
        assert!(zoom_factor_for_wheel(-3.0) < 1.0);
    }

    #[test]
    fn first_open_fits_without_waiting_for_canvas() {
        let mut v = OrigView::new();
        v.seed_stage_if_empty(400.0, 200.0);
        v.apply_image_size(200.0, 100.0);
        assert!((v.scale - 2.0).abs() < 1e-3);
        assert!((v.img_w * v.scale - 400.0).abs() < 1e-2);
    }

    #[test]
    fn percent_and_clamp() {
        assert_eq!(zoom_percent(1.0, 1.0), 100);
        assert_eq!(zoom_percent(8.0, 1.0), 800);
        assert_eq!(zoom_percent(0.5, 1.0), 50);
        assert!(release_is_click(5.9));
        assert!(!release_is_click(6.0));
    }

    #[test]
    fn zoom_at_keeps_cursor_point() {
        let mut v = OrigView::new();
        v.img_w = 200.0;
        v.img_h = 100.0;
        v.stage_w = 200.0;
        v.stage_h = 100.0;
        v.fit_in_stage();
        let before = v.scale;
        let img_x0 = (50.0 - v.tx) / before;
        let img_y0 = (25.0 - v.ty) / before;
        v.zoom_at(50.0, 25.0, 2.0);
        assert!((v.scale - (before * 2.0)).abs() < 1e-3);
        let img_x = (50.0 - v.tx) / v.scale;
        let img_y = (25.0 - v.ty) / v.scale;
        assert!((img_x - img_x0).abs() < 1e-2);
        assert!((img_y - img_y0).abs() < 1e-2);
    }

    #[test]
    fn film_content_width_counts_ring_padding() {
        assert!((film_content_w(0) - 32.0).abs() < 1e-3);
        assert!((film_content_w(1) - 124.0).abs() < 1e-3);
        assert!((film_content_w(2) - 224.0).abs() < 1e-3);
    }

    #[test]
    fn film_pan_follows_pointer_one_to_one() {
        assert_eq!(film_pan_offset(0.0, 40.0, 200.0), 0.0);
        assert!((film_pan_offset(-100.0, 40.0, 200.0) + 60.0).abs() < 1e-3);
        assert!((film_pan_offset(-100.0, -40.0, 200.0) + 140.0).abs() < 1e-3);
        assert_eq!(film_pan_offset(-100.0, -200.0, 200.0), -200.0);
        assert_eq!(film_pan_offset(-50.0, 10.0, 0.0), 0.0);
    }

    #[test]
    fn film_pan_drops_when_left_button_is_up() {
        let mut v = OrigView::new();
        v.begin_film_pan(10.0, Some(Uuid::nil()));
        assert!(v.is_film_panning());
        // Release happened outside the window: GPUI never delivers MouseUp.
        v.continue_film_pan(12.0, false);
        assert!(!v.is_film_panning());
        assert_eq!(v.end_film_pan(), None);
    }

    #[test]
    fn zoom_clamps_to_fit_range() {
        let mut v = OrigView::new();
        v.img_w = 100.0;
        v.img_h = 100.0;
        v.stage_w = 100.0;
        v.stage_h = 100.0;
        v.fit_in_stage();
        v.zoom_at(50.0, 50.0, 100.0);
        assert!((v.scale / v.fit - ZOOM_MAX).abs() < 1e-3);
        v.zoom_at(50.0, 50.0, 0.01);
        assert!((v.scale / v.fit - ZOOM_MIN).abs() < 1e-3);
    }
}
