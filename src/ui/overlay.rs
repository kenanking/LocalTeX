use std::sync::Arc;

use gpui::{
    div, img, prelude::*, px, rgb, App, Context, CursorStyle, Entity, FocusHandle, Focusable,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderImage,
    SharedString, Window,
};

use super::theme;
use crate::actions::{CancelOverlay, ConfirmOverlay};
use crate::capture::Grab;
use crate::doc::Rect;
use crate::imgutil;
use crate::state::AppState;

pub struct Overlay {
    state: Entity<AppState>,
    focus: FocusHandle,
    image: Arc<image::RgbaImage>,
    render: Arc<RenderImage>,
    origin_x: i32,
    origin_y: i32,
    anchor: Option<Point<Pixels>>,
    current: Point<Pixels>,
    closing: bool,
}

impl Overlay {
    pub fn new(
        state: Entity<AppState>,
        grab: Grab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let app_state = state.clone();
        // Do not `AppState::update` here: WM_DELETE is handled while GPUI's
        // X11 client is already borrowed (`RefCell`). Defer teardown.
        window.on_window_should_close(cx, move |_, cx| {
            let app_state = app_state.clone();
            cx.defer(move |cx| {
                app_state.update(cx, |s, cx| s.cancel_capture(cx));
            });
            true
        });
        let image = Arc::new(grab.image);
        let render = imgutil::rgba_to_render(&image);
        Self {
            state,
            focus,
            image,
            render,
            origin_x: grab.origin_x,
            origin_y: grab.origin_y,
            anchor: None,
            current: Point::default(),
            closing: false,
        }
    }

    pub fn reset(&mut self, grab: Grab, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_grab(grab);
        window.focus(&self.focus);
        cx.notify();
    }

    fn apply_grab(&mut self, grab: Grab) {
        self.closing = false;
        self.anchor = None;
        self.current = Point::default();
        self.origin_x = grab.origin_x;
        self.origin_y = grab.origin_y;
        self.image = Arc::new(grab.image);
        self.render = imgutil::rgba_to_render(&self.image);
    }

    fn selection_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let anchor = self.anchor?;
        let x0 = f32::from(anchor.x).min(f32::from(self.current.x));
        let y0 = f32::from(anchor.y).min(f32::from(self.current.y));
        let x1 = f32::from(anchor.x).max(f32::from(self.current.x));
        let y1 = f32::from(anchor.y).max(f32::from(self.current.y));
        let w = x1 - x0;
        let h = y1 - y0;
        if w < 8.0 || h < 8.0 {
            return None;
        }
        Some((x0, y0, w, h))
    }

    /// Map a point in overlay-window logical pixels to a physical pixel in the shot.
    fn window_to_image(&self, window: &Window, x: f32, y: f32) -> (f32, f32) {
        let scale = window.scale_factor().max(0.01);
        let win = window.bounds();
        let screen_x = f32::from(win.origin.x) + x;
        let screen_y = f32::from(win.origin.y) + y;
        (
            screen_x * scale - self.origin_x as f32,
            screen_y * scale - self.origin_y as f32,
        )
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.close_ephemeral_window(window);
        self.state.update(cx, |state, cx| state.cancel_capture(cx));
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        let Some((x, y, w, h)) = self.selection_rect() else {
            self.dismiss(window, cx);
            return;
        };
        let (x0, y0) = self.window_to_image(window, x, y);
        let (x1, y1) = self.window_to_image(window, x + w, y + h);
        let ix = x0.min(x1).round().max(0.0) as u32;
        let iy = y0.min(y1).round().max(0.0) as u32;
        let iw = ((x0 - x1).abs().round() as u32).max(1);
        let ih = ((y0 - y1).abs().round() as u32).max(1);
        let rect = Rect {
            x: ix,
            y: iy,
            w: iw,
            h: ih,
        };
        let crop = imgutil::crop(&self.image, rect.x, rect.y, rect.w, rect.h);
        self.closing = true;
        self.close_ephemeral_window(window);
        self.state
            .update(cx, |state, cx| state.finish_capture(crop, cx));
    }

    fn close_ephemeral_window(&self, window: &mut Window) {
        if !crate::desktop::overlay_keeps_window() {
            window.remove_window();
        }
    }

    fn cancel(&mut self, _: &CancelOverlay, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }

    fn confirm_key(&mut self, _: &ConfirmOverlay, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx);
    }

    fn on_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.button == MouseButton::Right {
            return;
        }
        self.anchor = Some(ev.position);
        self.current = ev.position;
        cx.notify();
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.current = ev.position;
        if self.anchor.is_some() {
            cx.notify();
        }
    }

    fn on_up(&mut self, ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.current = ev.position;
        if ev.button == MouseButton::Right {
            self.dismiss(window, cx);
            return;
        }
        if self.selection_rect().is_some() {
            self.confirm(window, cx);
        } else {
            self.dismiss(window, cx);
        }
    }
}

impl Focusable for Overlay {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl gpui::Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = window.scale_factor().max(0.01);
        let win = window.bounds();
        let view = window.viewport_size();
        let sel = self.selection_rect();

        // 1:1 with the desktop. Size is the shot, not the window — GNOME may
        // still place this client in the work area (below the top panel).
        let (img_left, img_top, img_w, img_h) = freeze_placement(
            self.image.width(),
            self.image.height(),
            self.origin_x,
            self.origin_y,
            scale,
            f32::from(win.origin.x),
            f32::from(win.origin.y),
        );

        if !self.focus.is_focused(window) {
            window.focus(&self.focus);
        }

        div()
            .id("overlay")
            .key_context("Overlay")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm_key))
            .cursor(CursorStyle::Arrow)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .occlude()
            .child(
                img(self.render.clone())
                    .absolute()
                    .left(px(img_left))
                    .top(px(img_top))
                    .min_w(px(img_w))
                    .min_h(px(img_h))
                    .w(px(img_w))
                    .h(px(img_h))
                    .object_fit(gpui::ObjectFit::Fill),
            )
            .children(dim_panels(
                f32::from(view.width),
                f32::from(view.height),
                sel,
            ))
            .when_some(sel, |d, (x, y, w, h)| {
                d.child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w))
                        .h(px(h))
                        .border_1()
                        .border_color(rgb(theme::ACCENT)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(x + 6.0))
                        .top(px((y - 22.0).max(6.0)))
                        .px_2()
                        .rounded_sm()
                        .bg(rgb(theme::ACCENT))
                        .text_xs()
                        .text_color(rgb(0xffffff))
                        .child(SharedString::from(format!("{}×{}", w as i32, h as i32))),
                )
            })
            .child(
                // Explicit shot size, not `size_full`: absolute % height is 0
                // when the parent layout has not resolved after fullscreen.
                div()
                    .id("overlay-hit")
                    .absolute()
                    .left(px(img_left))
                    .top(px(img_top))
                    .w(px(img_w))
                    .h(px(img_h))
                    .occlude()
                    .cursor(CursorStyle::Arrow)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::on_down))
                    .on_mouse_move(cx.listener(Self::on_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
                    .on_mouse_up(MouseButton::Right, cx.listener(Self::on_up)),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(24.))
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x1c1917))
                            .text_xs()
                            .text_color(rgb(0xe7e5e4))
                            .child("Drag to select · click or Esc to cancel"),
                    ),
            )
    }
}

/// Overlay-local box for the freeze-frame: always shot/scale, offset so a
/// screen pixel shows the matching shot pixel. Independent of client size.
fn freeze_placement(
    shot_w: u32,
    shot_h: u32,
    origin_x: i32,
    origin_y: i32,
    scale: f32,
    win_origin_x: f32,
    win_origin_y: f32,
) -> (f32, f32, f32, f32) {
    let scale = scale.max(0.01);
    (
        origin_x as f32 / scale - win_origin_x,
        origin_y as f32 / scale - win_origin_y,
        shot_w as f32 / scale,
        shot_h as f32 / scale,
    )
}

fn dim_panels(
    fw: f32,
    fh: f32,
    sel: Option<(f32, f32, f32, f32)>,
) -> Vec<gpui::Stateful<gpui::Div>> {
    let (x, y, w, h) = sel.unwrap_or((0.0, 0.0, 0.0, 0.0));
    let mut panels = Vec::new();
    if sel.is_none() {
        panels.push(dim_rect("dim-full", 0.0, 0.0, fw, fh));
        return panels;
    }
    if y > 0.0 {
        panels.push(dim_rect("dim-t", 0.0, 0.0, fw, y));
    }
    if y + h < fh {
        panels.push(dim_rect("dim-b", 0.0, y + h, fw, fh - y - h));
    }
    if x > 0.0 {
        panels.push(dim_rect("dim-l", 0.0, y, x, h));
    }
    if x + w < fw {
        panels.push(dim_rect("dim-r", x + w, y, fw - x - w, h));
    }
    panels
}

fn dim_rect(id: &'static str, x: f32, y: f32, w: f32, h: f32) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w.max(0.0)))
        .h(px(h.max(0.0)))
        .bg(theme::dim())
}

#[cfg(test)]
mod tests {
    use super::freeze_placement;

    #[test]
    fn workarea_window_does_not_change_freeze_size() {
        let (left, top, w, h) = freeze_placement(1920, 1080, 0, 0, 1.0, 0.0, 32.0);
        assert_eq!((w, h), (1920.0, 1080.0));
        assert_eq!((left, top), (0.0, -32.0));
    }

    #[test]
    fn hidpi_placement_uses_gpui_scale() {
        let (left, top, w, h) = freeze_placement(3840, 2160, 0, 0, 2.0, 0.0, 0.0);
        assert_eq!((left, top, w, h), (0.0, 0.0, 1920.0, 1080.0));
    }
}
