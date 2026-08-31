use uuid::Uuid;

use super::geom::{FOOTER_H, TOPBAR_H};

pub const STRIP_MIN: f32 = 64.0;
pub const STRIP_DEFAULT: f32 = 96.0;
pub const STRIP_AUTO_CAP: f32 = 280.0;
pub const STRIP_MIN_PREVIEW: f32 = 80.0;

/// Layout budget for the Copy chip row when the snip is Ready.
/// Not `geom::FILM_H` (overlay filmstrip).
pub const COPY_RESERVE_H: f32 = 72.0;

pub fn copy_reserve(ready: bool) -> f32 {
    if ready {
        COPY_RESERVE_H
    } else {
        0.0
    }
}

pub struct OrigStrip {
    doc: Option<Uuid>,
    pub drag: Option<(f32, f32)>,
    pub split_hover: bool,
}

impl OrigStrip {
    pub fn new() -> Self {
        Self {
            doc: None,
            drag: None,
            split_hover: false,
        }
    }

    pub fn bind_doc(&mut self, id: Option<Uuid>) {
        if self.doc == id {
            return;
        }
        self.doc = id;
        self.drag = None;
    }

    pub fn begin_drag(&mut self, y: f32, current_h: f32) {
        self.drag = Some((y, current_h));
    }

    pub fn drag_to(&self, y: f32, max_h: f32) -> Option<f32> {
        let (start_y, start_h) = self.drag?;
        Some(clamp_strip_h(start_h + (y - start_y), max_h))
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }
}

pub fn auto_strip_h(img_w: f32, img_h: f32, pane_w: f32, max_h: f32) -> f32 {
    let natural = if img_w <= 0.0 {
        STRIP_DEFAULT
    } else {
        pane_w * img_h / img_w
    };
    clamp_strip_h(natural, STRIP_AUTO_CAP.min(max_h))
}

pub fn clamp_strip_h(h: f32, max_h: f32) -> f32 {
    let lo = STRIP_MIN.min(max_h);
    let hi = max_h.max(lo);
    h.clamp(lo, hi)
}

pub fn max_strip_h(win_h: f32, copy_h: f32) -> f32 {
    (win_h - TOPBAR_H - FOOTER_H - copy_h - STRIP_MIN_PREVIEW - 24.0).max(STRIP_MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_image_auto_height_near_natural() {
        let h = auto_strip_h(1800.0, 220.0, 700.0, 500.0);
        assert!((h - 86.0).abs() < 2.0, "got {h}");
        assert!(h >= STRIP_MIN);
        assert!(h < 140.0);
    }

    #[test]
    fn tall_image_hits_auto_cap() {
        let h = auto_strip_h(900.0, 1280.0, 700.0, 500.0);
        assert_eq!(h, STRIP_AUTO_CAP);
    }

    #[test]
    fn tall_image_respects_tight_max_h() {
        let h = auto_strip_h(900.0, 1280.0, 700.0, 200.0);
        assert!(h <= 200.0);
        assert!(h >= STRIP_MIN);
    }

    #[test]
    fn unknown_image_uses_default() {
        let h = auto_strip_h(0.0, 100.0, 700.0, 500.0);
        assert_eq!(h, STRIP_DEFAULT);
    }

    #[test]
    fn bind_doc_clears_drag_only_on_doc_change() {
        let mut strip = OrigStrip::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        strip.bind_doc(Some(a));
        strip.begin_drag(10.0, 180.0);
        assert!(strip.drag.is_some());
        strip.bind_doc(Some(a));
        assert!(strip.drag.is_some());
        strip.bind_doc(Some(b));
        assert!(strip.drag.is_none());
    }

    #[test]
    fn drag_to_is_relative_to_press() {
        let mut strip = OrigStrip::new();
        strip.begin_drag(10.0, 180.0);
        let next = strip.drag_to(40.0, 500.0).unwrap();
        assert!((next - 210.0).abs() < 0.5, "got {next}");
    }

    #[test]
    fn clamp_strip_h_respects_max_h() {
        assert!(clamp_strip_h(400.0, 200.0) <= 200.0);
        assert_eq!(clamp_strip_h(400.0, 200.0), 200.0);
        assert_eq!(clamp_strip_h(10.0, 200.0), STRIP_MIN);
        assert_eq!(clamp_strip_h(50.0, 50.0), 50.0);
    }

    #[test]
    fn max_strip_h_short_window_stays_min() {
        assert!(max_strip_h(200.0, copy_reserve(true)) >= STRIP_MIN);
        assert_eq!(max_strip_h(200.0, copy_reserve(true)), STRIP_MIN);
    }

    #[test]
    fn copy_reserve_is_72_when_ready() {
        assert!((copy_reserve(true) - COPY_RESERVE_H).abs() < 0.5);
        assert!((copy_reserve(false) - 0.0).abs() < 0.5);
    }

    #[test]
    fn max_strip_h_uses_copy_reserve_constant() {
        assert_eq!(max_strip_h(200.0, copy_reserve(true)), STRIP_MIN);
        assert!(max_strip_h(200.0, copy_reserve(true)) >= STRIP_MIN);
    }

    #[test]
    fn stored_height_stays_when_max_h_grows() {
        let h1 = clamp_strip_h(220.0, 250.0);
        let h2 = clamp_strip_h(220.0, 500.0);
        assert!((h1 - 220.0).abs() < 0.5, "got {h1}");
        assert!((h2 - 220.0).abs() < 0.5, "got {h2}");
    }
}
