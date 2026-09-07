use std::sync::Arc;

use gpui::{
    Context, Corners, CursorStyle, MouseButton, MouseDownEvent, ObjectFit, RenderImage, canvas,
    div, prelude::*, px, rgb,
};
use uuid::Uuid;

use super::super::main_window::MainWindow;
use super::super::theme;
use super::super::widgets::{IconKind, missing_image_slot};
use super::super::window_drag::WindowDrag;
use super::chrome::{orig_action_capsule, orig_hud_disc};
use crate::i18n::t;

pub const STRIP_MIN: f32 = 64.0;
pub const STRIP_DEFAULT: f32 = 96.0;
pub const STRIP_AUTO_CAP: f32 = 280.0;
pub const STRIP_MIN_PREVIEW: f32 = 80.0;

/// Layout budget for the Copy chip row when the snip is Ready.
/// Not `geom::FILM_H` (overlay filmstrip).
pub const COPY_RESERVE_H: f32 = 72.0;

pub fn copy_reserve(ready: bool) -> f32 {
    if ready { COPY_RESERVE_H } else { 0.0 }
}

pub struct OrigStrip {
    doc: Option<Uuid>,
    pub split_hover: bool,
}

impl OrigStrip {
    pub fn new() -> Self {
        Self {
            doc: None,
            split_hover: false,
        }
    }

    /// `true` when the bound document changed. Caller must cancel `WindowDrag::Strip`.
    pub fn bind_doc(&mut self, id: Option<Uuid>) -> bool {
        if self.doc == id {
            return false;
        }
        self.doc = id;
        true
    }
}

pub(crate) struct OrigStripFrame {
    pub doc_id: Uuid,
    pub full: Option<Arc<RenderImage>>,
    pub hovered: bool,
    pub copied: bool,
    pub reveal_enabled: bool,
    pub missing: bool,
    pub img_w: f32,
    pub img_h: f32,
    pub pane_w: f32,
    pub strip_h: f32,
    pub max_h: f32,
}

impl MainWindow {
    pub(crate) fn render_orig_strip(
        &self,
        frame: OrigStripFrame,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let OrigStripFrame {
            doc_id,
            full,
            hovered,
            copied,
            reveal_enabled,
            missing,
            img_w,
            img_h,
            pane_w,
            strip_h,
            max_h,
        } = frame;
        let split_on =
            self.orig_strip.split_hover || self.window_drag.as_ref().is_some_and(|d| d.is_strip());
        let card = if missing {
            missing_image_slot(px(720.), px(strip_h))
                .id("orig-thumb")
                .size_full()
                .when(split_on, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .right(px(8.))
                            .bottom_0()
                            .h(px(2.))
                            .rounded(px(2.))
                            .bg(rgb(theme::ACCENT)),
                    )
                })
                .child(self.orig_strip_handle(strip_h, img_w, img_h, pane_w, max_h, cx))
                .into_any()
        } else {
            let entity = cx.entity();
            div()
                .id("orig-thumb")
                .size_full()
                .overflow_hidden()
                .rounded_md()
                .bg(rgb(theme::BG_SUNKEN))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .when_some(full.clone(), |d, img_data| {
                    d.child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                let fitted =
                                    ObjectFit::Contain.get_bounds(bounds, img_data.size(0));
                                window
                                    .paint_image(
                                        fitted,
                                        fitted,
                                        Corners::default(),
                                        img_data,
                                        0,
                                        false,
                                    )
                                    .ok();
                            },
                        )
                        .size_full(),
                    )
                })
                .when(full.is_some(), |d| {
                    d.child(
                        div()
                            .id("orig-hit")
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom(px(12.))
                            .occlude()
                            .cursor_pointer()
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                if this.orig.strip_hover != *hovered {
                                    this.orig.strip_hover = *hovered;
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.zoom_original(doc_id, window, cx);
                            }))
                            .flex()
                            .items_start()
                            .justify_end()
                            .pt(px(8.))
                            .pr(px(8.))
                            .when(hovered, |d| {
                                d.child(
                                    div()
                                        .id("strip-actions")
                                        .h(px(28.))
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .child(orig_action_capsule(
                                            doc_id,
                                            copied,
                                            reveal_enabled,
                                            "strip",
                                            cx,
                                        ))
                                        .child(orig_hud_disc(
                                            "strip-zoom",
                                            IconKind::Corners,
                                            t("orig.open"),
                                            {
                                                let entity = entity.clone();
                                                move |_, window, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.zoom_original(doc_id, window, cx);
                                                    });
                                                }
                                            },
                                        )),
                                )
                            }),
                    )
                })
                .when(split_on, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .right(px(8.))
                            .bottom_0()
                            .h(px(2.))
                            .rounded(px(2.))
                            .bg(rgb(theme::ACCENT)),
                    )
                })
                .child(self.orig_strip_handle(strip_h, img_w, img_h, pane_w, max_h, cx))
                .into_any()
        };

        div()
            .id("orig-wrap")
            .px_4()
            .pt_3()
            .flex_none()
            .h(px(strip_h))
            .overflow_hidden()
            .child(card)
    }

    fn orig_strip_handle(
        &self,
        strip_h: f32,
        img_w: f32,
        img_h: f32,
        pane_w: f32,
        max_h: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .id("orig-strip-resize")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(12.))
            .occlude()
            .cursor(CursorStyle::ResizeUpDown)
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.orig_strip.split_hover != *hovered {
                    this.orig_strip.split_hover = *hovered;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    if ev.click_count >= 2 {
                        let next = auto_strip_h(img_w, img_h, pane_w, max_h);
                        this.state.update(cx, |s, cx| {
                            s.prefs.orig_strip_h = next;
                            s.persist_prefs();
                            cx.notify();
                        });
                    } else {
                        this.window_drag = Some(WindowDrag::Strip {
                            start_y: f32::from(ev.position.y),
                            start_h: strip_h,
                        });
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
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

pub fn max_strip_h(workspace_h: f32, copy_h: f32) -> f32 {
    (workspace_h - copy_h - STRIP_MIN_PREVIEW - 24.0).max(STRIP_MIN)
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
    fn bind_doc_reports_change_only() {
        let mut strip = OrigStrip::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert!(strip.bind_doc(Some(a)));
        assert!(!strip.bind_doc(Some(a)));
        assert!(strip.bind_doc(Some(b)));
        assert!(strip.bind_doc(None));
        assert!(!strip.bind_doc(None));
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
    fn max_strip_h_uses_workspace_budget() {
        assert_eq!(max_strip_h(456.0, copy_reserve(true)), 280.0);
    }

    #[test]
    fn stored_height_stays_when_max_h_grows() {
        let h1 = clamp_strip_h(220.0, 250.0);
        let h2 = clamp_strip_h(220.0, 500.0);
        assert!((h1 - 220.0).abs() < 0.5, "got {h1}");
        assert!((h2 - 220.0).abs() < 0.5, "got {h2}");
    }
}
