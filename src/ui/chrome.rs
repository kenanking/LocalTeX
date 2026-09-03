#[cfg(target_os = "linux")]
use gpui::{canvas, Bounds, CursorStyle, Decorations, HitboxBehavior};
use gpui::{
    div, img, prelude::*, px, rgb, svg, AnyElement, Context, Entity, MouseButton, ObjectFit,
    SharedString, Window,
};
#[cfg(any(test, target_os = "linux"))]
use gpui::{point, Pixels, Point, ResizeEdge, Size, Tiling};

use super::main_window::{MainWindow, View};
use super::theme;
use super::widgets::{icon_btn, status_dot, IconKind};
use crate::doc::DocStatus;
use crate::keymap::{self, ShortcutId};
use crate::ocr::EngineStatus;
use crate::state::AppState;

pub(crate) const CAPTION_H: f32 = 32.0;
pub(crate) const TOOLBAR_H: f32 = 44.0;
pub(crate) const FOOTER_H: f32 = 28.0;

struct ToolbarState {
    capturing: bool,
    has_selected: bool,
    can_open_docx: bool,
    capture_tip: String,
    view: View,
    state: Entity<AppState>,
}

pub(crate) fn workspace_height(viewport_h: f32) -> f32 {
    let caption_h = if cfg!(target_os = "linux") {
        CAPTION_H
    } else {
        0.0
    };
    (viewport_h - caption_h - TOOLBAR_H - FOOTER_H).max(0.0)
}

impl MainWindow {
    pub(crate) fn render_topbar(
        &self,
        capturing: bool,
        has_selected: bool,
        can_open_docx: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.clone();
        let capture_tip = {
            let over = &self.state.read(cx).prefs.shortcuts;
            let label = keymap::spec(ShortcutId::Capture).label;
            match keymap::effective(over, ShortcutId::Capture) {
                Some(chord) => format!("{label}  {}", keymap::chips(&chord).join("+")),
                None => label.to_string(),
            }
        };
        let view = self.view.clone();

        div()
            .flex()
            .flex_col()
            .w_full()
            .when(cfg!(target_os = "linux"), |chrome| {
                chrome.child(self.render_caption(window))
            })
            .child(self.render_toolbar(
                ToolbarState {
                    capturing,
                    has_selected,
                    can_open_docx,
                    capture_tip,
                    view,
                    state,
                },
                cx,
            ))
    }

    fn render_caption(&self, window: &Window) -> impl IntoElement {
        let close_state = self.state.clone();
        let minimize_state = self.state.clone();
        // Move only after the pointer actually travels. Starting a compositor
        // move on mouse-down swallows the second click of a title-bar double-click.
        let pending_move = self.caption_pending_move.clone();
        div()
            .id("window-caption")
            .flex()
            .items_center()
            .h(px(CAPTION_H))
            .pl_3()
            .bg(rgb(theme::BG_SUNKEN))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .on_mouse_down(MouseButton::Left, {
                let pending_move = pending_move.clone();
                move |event, window, _| {
                    if event.click_count >= 2 {
                        pending_move.set(false);
                        window.zoom_window();
                        return;
                    }
                    pending_move.set(true);
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let pending_move = pending_move.clone();
                move |_, _, _| pending_move.set(false)
            })
            .on_mouse_move({
                let pending_move = pending_move.clone();
                move |event, window, _| {
                    if pending_move.get() && event.dragging() {
                        pending_move.set(false);
                        window.start_window_move();
                    }
                }
            })
            .child(
                div()
                    .id("window-title")
                    .pr_2()
                    .h(px(CAPTION_H))
                    .flex()
                    .items_center()
                    .text_sm()
                    .text_color(rgb(theme::MUTED))
                    .child(
                        img(crate::icon::app_tile_image())
                            .size(px(18.))
                            .mr_2()
                            .rounded_sm()
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(crate::identity::APP_NAME),
            )
            .child(div().flex_1())
            .child(window_control(
                "window-minimize",
                "Minimize",
                false,
                div()
                    .w(px(11.))
                    .h(px(1.))
                    .bg(rgb(theme::TEXT))
                    .into_any_element(),
                move |window, cx| {
                    minimize_state.update(cx, |state, cx| {
                        state.minimize_main(window, cx);
                    });
                },
            ))
            .child(window_control(
                "window-maximize",
                if window.is_maximized() {
                    "Restore"
                } else {
                    "Maximize"
                },
                false,
                div()
                    .size(px(10.))
                    .border_1()
                    .border_color(rgb(theme::TEXT))
                    .into_any_element(),
                |window, _| window.zoom_window(),
            ))
            .child(window_control(
                "window-close",
                "Close",
                true,
                svg()
                    .path(IconKind::Close.asset_path())
                    .size(px(14.))
                    .text_color(rgb(theme::TEXT))
                    .into_any_element(),
                move |window, cx| {
                    let action = close_state.read(cx).prefs.close_action;
                    close_state.update(cx, |state, cx| {
                        state.handle_main_close(action, window, cx);
                    });
                },
            ))
    }

    fn render_toolbar(&self, toolbar: ToolbarState, cx: &mut Context<Self>) -> impl IntoElement {
        let ToolbarState {
            capturing,
            has_selected,
            can_open_docx,
            capture_tip,
            view,
            state,
        } = toolbar;
        let tools = div()
            .flex()
            .items_center()
            .gap_1()
            .child(icon_btn(
                "topbar-home",
                IconKind::Library,
                "Library",
                matches!(view, View::Library),
                true,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| this.dismiss_sheet(cx));
                    }
                },
            ))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(rgb(theme::TRACK_OFF)))
            .child(icon_btn(
                "tool-snip",
                IconKind::Snip,
                capture_tip,
                false,
                !capturing,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.dismiss_sheet(cx);
                            this.state.update(cx, |s, cx| s.request_capture(cx));
                        });
                    }
                },
            ))
            .child(icon_btn(
                "tool-upload",
                IconKind::Upload,
                "Upload snip  Ctrl+O",
                false,
                !capturing,
                {
                    let state = state.clone();
                    move |_, cx| {
                        state.update(cx, |s, cx| s.request_upload(cx));
                    }
                },
            ))
            .child(icon_btn(
                "tool-paste",
                IconKind::Paste,
                "Paste image or path from clipboard  Ctrl+V",
                false,
                !capturing,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.dismiss_sheet(cx);
                            this.state.update(cx, |s, cx| s.request_paste(cx));
                        });
                    }
                },
            ))
            .child(icon_btn(
                "tool-draw",
                IconKind::Draw,
                "Create snip from drawing  Ctrl+D",
                matches!(view, View::Draw),
                !capturing,
                {
                    let entity = cx.entity();
                    move |window, cx| {
                        entity.update(cx, |this, cx| this.toggle_draw(window, cx));
                    }
                },
            ))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(rgb(theme::TRACK_OFF)))
            .child(icon_btn(
                "tool-word",
                IconKind::Word,
                "Open as Word document",
                false,
                can_open_docx,
                {
                    let state = state.clone();
                    move |_, cx| {
                        state.update(cx, |s, cx| s.open_docx_selected(cx));
                    }
                },
            ));
        div()
            .flex()
            .items_center()
            .h(px(TOOLBAR_H))
            .px_3()
            .bg(rgb(theme::BG_RAISED))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(div().w(px(64.)).h(px(32.)))
            .child(div().flex_1())
            .child(tools)
            .child(div().flex_1())
            .child(
                div()
                    .w(px(64.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(icon_btn(
                        "tool-delete",
                        IconKind::Delete,
                        "Delete snip  Delete",
                        false,
                        has_selected && matches!(view, View::Library),
                        {
                            let state = state.clone();
                            move |_, cx| {
                                state.update(cx, |s, cx| s.delete_selected(cx));
                            }
                        },
                    ))
                    .child(icon_btn(
                        "tool-settings",
                        IconKind::Settings,
                        "Settings  Ctrl+,",
                        matches!(view, View::Settings),
                        true,
                        {
                            let entity = cx.entity();
                            move |window, cx| {
                                entity.update(cx, |this, cx| this.toggle_settings(window, cx));
                            }
                        },
                    )),
            )
    }

    pub(crate) fn render_footer(
        &self,
        status_kind: theme::StatusKind,
        status_label: String,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .h(px(FOOTER_H))
            .px_4()
            .gap_2()
            .bg(rgb(theme::BG))
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .child(status_dot(status_kind))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_ellipsis()
                    .text_color(rgb(theme::MUTED))
                    .child(SharedString::from(status_label)),
            )
    }
}

fn window_control(
    id: &'static str,
    hint: &'static str,
    close: bool,
    glyph: AnyElement,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(40.))
        .h(px(CAPTION_H))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(close, |button| {
            button.hover(|button| {
                button
                    .bg(rgb(theme::DANGER))
                    .text_color(rgb(theme::ON_ACCENT))
            })
        })
        .when(!close, |button| {
            button.hover(|button| button.bg(theme::row_hover()))
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| on_click(window, cx))
        .tooltip(super::widgets::Tooltip::text(hint))
        .child(glyph)
}

pub(crate) fn client_frame(content: impl IntoElement, window: &mut Window) -> AnyElement {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        return content.into_any_element();
    }

    #[cfg(target_os = "linux")]
    {
        let tiling = match window.window_decorations() {
            Decorations::Client { tiling } => tiling,
            Decorations::Server => Tiling::default(),
        };

        const RESIZE_INSET: Pixels = px(5.);

        div()
            .id("client-frame")
            .relative()
            .size_full()
            .on_mouse_down(MouseButton::Left, move |event, window, _| {
                let size = window.window_bounds().get_bounds().size;
                if let Some(edge) = resize_edge(event.position, RESIZE_INSET, size, tiling) {
                    window.start_window_resize(edge);
                }
            })
            .child(
                div()
                    .size_full()
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .child(content),
            )
            .child(
                canvas(
                    |_bounds, window, _| {
                        window.insert_hitbox(
                            Bounds::new(
                                point(px(0.), px(0.)),
                                window.window_bounds().get_bounds().size,
                            ),
                            HitboxBehavior::Normal,
                        )
                    },
                    move |_bounds, hitbox, window, _| {
                        let size = window.window_bounds().get_bounds().size;
                        let Some(edge) =
                            resize_edge(window.mouse_position(), RESIZE_INSET, size, tiling)
                        else {
                            return;
                        };
                        window.set_cursor_style(resize_cursor(edge), &hitbox);
                    },
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }
}

#[cfg(any(test, target_os = "linux"))]
fn resize_edge(
    position: Point<Pixels>,
    inset: Pixels,
    window_size: Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let left = position.x < inset;
    let right = position.x > window_size.width - inset;
    let top = position.y < inset;
    let bottom = position.y > window_size.height - inset;

    if top && left && !tiling.top && !tiling.left {
        Some(ResizeEdge::TopLeft)
    } else if top && right && !tiling.top && !tiling.right {
        Some(ResizeEdge::TopRight)
    } else if bottom && left && !tiling.bottom && !tiling.left {
        Some(ResizeEdge::BottomLeft)
    } else if bottom && right && !tiling.bottom && !tiling.right {
        Some(ResizeEdge::BottomRight)
    } else if top && !tiling.top {
        Some(ResizeEdge::Top)
    } else if bottom && !tiling.bottom {
        Some(ResizeEdge::Bottom)
    } else if left && !tiling.left {
        Some(ResizeEdge::Left)
    } else if right && !tiling.right {
        Some(ResizeEdge::Right)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

pub(crate) fn chrome(state: &AppState) -> (theme::StatusKind, String) {
    if state.is_bootstrapping() {
        return (theme::StatusKind::Busy, "Loading library…".into());
    }
    if state.is_capturing() {
        return (theme::StatusKind::Busy, "Capturing…".into());
    }
    if let Some(err) = state.error_message() {
        return (theme::StatusKind::Error, err.to_string());
    }
    if let Some(doc) = state.selected_doc() {
        if let DocStatus::Failed(err) = &doc.status {
            return (theme::StatusKind::Error, err.clone());
        }
    }
    match state.engine_status() {
        status @ EngineStatus::MissingModels { .. } => (theme::StatusKind::Idle, status.label()),
        EngineStatus::Ready => (theme::StatusKind::Ready, "Ready when you are".into()),
    }
}

#[cfg(test)]
mod client_frame_tests {
    use super::*;
    use gpui::size;

    #[test]
    fn resize_edges_respect_tiling() {
        let window = size(px(800.), px(560.));
        assert_eq!(
            resize_edge(point(px(1.), px(1.)), px(5.), window, Tiling::default()),
            Some(ResizeEdge::TopLeft)
        );
        assert_eq!(
            resize_edge(point(px(400.), px(300.)), px(5.), window, Tiling::default()),
            None
        );
        assert_eq!(
            resize_edge(point(px(1.), px(1.)), px(5.), window, Tiling::tiled()),
            None
        );
    }

    #[test]
    fn caption_minimize_updates_tracked_visibility() {
        let src = include_str!("chrome.rs");
        let caption = src
            .split("fn render_caption")
            .nth(1)
            .expect("render_caption")
            .split("fn render_toolbar")
            .next()
            .expect("body");
        assert!(
            !caption.contains("window.minimize_window()"),
            "caption minimize must go through AppState so main_window_visible stays in sync"
        );
        assert!(caption.contains("minimize_main"));
    }

    #[test]
    fn workspace_excludes_rendered_chrome() {
        #[cfg(target_os = "linux")]
        assert_eq!(workspace_height(560.0), 456.0);
        #[cfg(not(target_os = "linux"))]
        assert_eq!(workspace_height(560.0), 488.0);
        assert_eq!(workspace_height(10.0), 0.0);
    }
}
