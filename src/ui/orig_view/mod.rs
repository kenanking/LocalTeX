//! Original-image gallery overlay: contain-fit, wheel zoom, pan, click-to-close.

mod chrome;
mod geom;
mod overlay;
mod strip;

pub(crate) use chrome::{orig_action_capsule, orig_hud_disc};
pub(crate) use geom::fit_scale;
pub(crate) use strip::{max_strip_h, OrigStrip};

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{px, Bounds, Pixels, ScrollHandle};
use uuid::Uuid;

use super::scroll::ScrollThumbDrag;

use geom::{
    film_cell_w, film_pan_offset, film_scroll_to_show, film_thumb_left, release_is_click, ZOOM_MAX,
    ZOOM_MIN,
};

pub(crate) struct OrigDrag {
    origin_x: f32,
    origin_y: f32,
    start_tx: f32,
    start_ty: f32,
    dist: f32,
}

struct FilmPan {
    origin_x: f32,
    start_off: f32,
    dist: f32,
    pending_id: Option<Uuid>,
}

enum OrigPointer {
    Idle,
    Image(OrigDrag),
    Film(FilmPan),
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
    pointer: OrigPointer,
    pub film: ScrollHandle,
    pub film_thumb: Rc<RefCell<Option<ScrollThumbDrag>>>,
    film_view_w: f32,
    film_reveal: Option<usize>,
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
            pointer: OrigPointer::Idle,
            film: ScrollHandle::new(),
            film_thumb: Rc::new(RefCell::new(None)),
            film_view_w: 0.0,
            film_reveal: None,
            shown_id: None,
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.strip_hover = false;
        self.pointer = OrigPointer::Idle;
        self.film_view_w = 0.0;
        self.film_reveal = None;
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
        self.pointer = OrigPointer::Idle;
        self.film_view_w = 0.0;
        self.film_reveal = None;
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

    pub fn on_new_image(&mut self, id: Uuid) -> bool {
        if self.shown_id == Some(id) {
            return false;
        }
        self.shown_id = Some(id);
        if !matches!(self.pointer, OrigPointer::Film(_)) {
            self.pointer = OrigPointer::Idle;
        }
        self.fit = 0.0;
        if self.stage_w > 0.0 && self.img_w > 0.0 {
            self.fit_in_stage();
        }
        true
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
        self.pointer = OrigPointer::Image(OrigDrag {
            origin_x: x,
            origin_y: y,
            start_tx: self.tx,
            start_ty: self.ty,
            dist: 0.0,
        });
    }

    pub fn drag_to(&mut self, x: f32, y: f32) {
        let OrigPointer::Image(drag) = &mut self.pointer else {
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
        match std::mem::replace(&mut self.pointer, OrigPointer::Idle) {
            OrigPointer::Image(drag) => release_is_click(drag.dist),
            other => {
                self.pointer = other;
                false
            }
        }
    }

    pub fn has_pointer(&self) -> bool {
        !matches!(self.pointer, OrigPointer::Idle)
    }

    pub fn is_film_panning(&self) -> bool {
        matches!(self.pointer, OrigPointer::Film(_))
    }

    pub fn is_image_panning(&self) -> bool {
        matches!(self.pointer, OrigPointer::Image(_))
    }

    pub fn begin_film_pan(&mut self, x: f32, pending_id: Option<Uuid>) {
        let start_off: f32 = self.film.offset().x.into();
        self.pointer = OrigPointer::Film(FilmPan {
            origin_x: x,
            start_off,
            dist: 0.0,
            pending_id,
        });
    }

    pub fn drag_film(&mut self, x: f32) {
        let OrigPointer::Film(pan) = &mut self.pointer else {
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
        match std::mem::replace(&mut self.pointer, OrigPointer::Idle) {
            OrigPointer::Film(pan) if release_is_click(pan.dist) => pan.pending_id,
            OrigPointer::Film(_) => None,
            other => {
                self.pointer = other;
                None
            }
        }
    }

    /// Continue an in-flight gesture, or drop it if the button is up (GPUI
    /// does not deliver `MouseUp` when the release happens outside the window).
    pub fn pointer_move(&mut self, x: f32, y: f32, left_down: bool) {
        if matches!(self.pointer, OrigPointer::Idle) {
            return;
        }
        if !left_down {
            self.pointer = OrigPointer::Idle;
            return;
        }
        if matches!(self.pointer, OrigPointer::Film(_)) {
            self.drag_film(x);
        } else if matches!(self.pointer, OrigPointer::Image(_)) {
            self.drag_to(x, y);
        }
    }

    pub fn request_film_reveal(&mut self, index: usize) {
        self.film_reveal = Some(index);
    }

    pub fn apply_film_reveal(&mut self) {
        if matches!(self.pointer, OrigPointer::Film(_)) || self.film_thumb.borrow().is_some() {
            return;
        }
        let Some(i) = self.film_reveal else {
            return;
        };
        let view_w: f32 = self.film.bounds().size.width.into();
        if view_w <= 1.0 {
            return;
        }
        let max: f32 = self.film.max_offset().width.into();
        let cur: f32 = self.film.offset().x.into();
        let next = film_scroll_to_show(cur, max, view_w, film_thumb_left(i), film_cell_w());
        if (next - cur).abs() > 0.5 {
            let mut off = self.film.offset();
            off.x = px(next);
            self.film.set_offset(off);
        }
        self.film_reveal = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_open_fits_without_waiting_for_canvas() {
        let mut v = OrigView::new();
        v.seed_stage_if_empty(400.0, 200.0);
        v.apply_image_size(200.0, 100.0);
        assert!((v.scale - 2.0).abs() < 1e-3);
        assert!((v.img_w * v.scale - 400.0).abs() < 1e-2);
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
    fn film_pan_drops_when_left_button_is_up() {
        let mut v = OrigView::new();
        v.begin_film_pan(10.0, Some(Uuid::nil()));
        assert!(v.is_film_panning());
        v.pointer_move(12.0, 0.0, false);
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
