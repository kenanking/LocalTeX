use super::super::scroll::clamp_neg;

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

/// Must match `MainWindow` chrome; used only to seed the first overlay frame.
pub(crate) const TOPBAR_H: f32 = 44.0;
pub(crate) const FOOTER_H: f32 = 28.0;
pub(crate) const FILM_H: f32 = 72.0;
pub(crate) const FILM_THUMB_W: f32 = 88.0;
pub(crate) const FILM_THUMB_H: f32 = 56.0;
pub(crate) const FILM_GAP: f32 = 8.0;
pub(crate) const FILM_PAD_X: f32 = 16.0;
pub(crate) const FILM_CELL_PAD: f32 = 4.0;

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
    clamp_neg(start_off + dx, max)
}

pub(crate) fn film_cell_w() -> f32 {
    FILM_THUMB_W + FILM_CELL_PAD
}

pub(crate) fn film_thumb_left(i: usize) -> f32 {
    FILM_PAD_X + i as f32 * (film_cell_w() + FILM_GAP)
}

/// Keep `thumb` fully visible with the smallest scroll. Already inside the
/// viewport: no-op. Off the left: pin to the left edge. Off the right: pin
/// to the right edge. (`overflow: nearest`.)
pub(crate) fn film_scroll_to_show(
    offset: f32,
    max: f32,
    view_w: f32,
    thumb_left: f32,
    thumb_w: f32,
) -> f32 {
    if view_w <= 1.0 {
        return clamp_neg(offset, max);
    }
    let vis_l = -offset;
    let vis_r = vis_l + view_w;
    let thumb_r = thumb_left + thumb_w;
    if thumb_w >= view_w - 1.0 {
        return clamp_neg(-thumb_left, max);
    }
    if thumb_left >= vis_l - 0.5 && thumb_r <= vis_r + 0.5 {
        return clamp_neg(offset, max);
    }
    if thumb_left < vis_l {
        return clamp_neg(-thumb_left, max);
    }
    clamp_neg(view_w - thumb_r, max)
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
    fn percent_and_clamp() {
        assert_eq!(zoom_percent(1.0, 1.0), 100);
        assert_eq!(zoom_percent(8.0, 1.0), 800);
        assert_eq!(zoom_percent(0.5, 1.0), 50);
        assert!(release_is_click(5.9));
        assert!(!release_is_click(6.0));
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
    fn film_scroll_to_show_is_nearest_edge() {
        let cell = 92.0;
        assert_eq!(film_scroll_to_show(0.0, 400.0, 200.0, 40.0, cell), 0.0);
        assert_eq!(
            film_scroll_to_show(-100.0, 400.0, 200.0, 150.0, cell),
            -100.0
        );
        assert!((film_scroll_to_show(-200.0, 400.0, 200.0, 0.0, cell) - 0.0).abs() < 1e-3);
        assert!((film_scroll_to_show(0.0, 400.0, 200.0, 300.0, cell) + 192.0).abs() < 1e-3);
        assert!((film_scroll_to_show(0.0, 400.0, 200.0, 150.0, cell) + 42.0).abs() < 1e-3);
        assert!((film_scroll_to_show(-50.0, 400.0, 200.0, 10.0, cell) + 10.0).abs() < 1e-3);
    }

    #[test]
    fn film_thumb_left_matches_content_width_gaps() {
        assert!((film_thumb_left(0) - 16.0).abs() < 1e-3);
        assert!((film_thumb_left(1) - (16.0 + 92.0 + 8.0)).abs() < 1e-3);
        let n = 2;
        let right = film_thumb_left(n - 1) + film_cell_w() + 16.0;
        assert!((right - film_content_w(n)).abs() < 1e-3);
    }
}
