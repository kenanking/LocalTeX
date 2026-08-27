//! Overlay scrollbars and nested horizontal panes for the preview.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, AnyElement, App, DispatchPhase, Element, EntityId, LayoutId, MouseButton,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle, SharedString, Style, Window,
};

use super::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxis {
    Horizontal,
    Vertical,
}

/// GPUI 0.2 `overflow_y_scroll` enables wheel scrolling but does not
/// paint a native thumb. Overlay this on a `relative` parent that
/// `track_scroll`s the same handle. Drag is stored in `drag` and applied
/// from the window `on_mouse_move` (thumbs are too thin to track moves).
pub struct ScrollThumbDrag {
    pub vertical: bool,
    pub start_mouse: f32,
    pub start_offset: f32,
    pub travel: f32,
    pub max: f32,
    pub handle: ScrollHandle,
}

pub fn apply_thumb_drag(drag: &RefCell<Option<ScrollThumbDrag>>, pos: Point<Pixels>) -> bool {
    let Some(d) = drag.borrow().as_ref().map(|d| {
        (
            d.vertical,
            d.start_mouse,
            d.start_offset,
            d.travel,
            d.max,
            d.handle.clone(),
        )
    }) else {
        return false;
    };
    let (vertical, start_mouse, start_offset, travel, max, handle) = d;
    let mouse = if vertical {
        f32::from(pos.y)
    } else {
        f32::from(pos.x)
    };
    let ratio = if travel > 0.5 {
        (mouse - start_mouse) / travel
    } else {
        0.0
    };
    let new_off = clamp_neg(start_offset - ratio * max, max);
    let mut off = handle.offset();
    if vertical {
        off.y = px(new_off);
    } else {
        off.x = px(new_off);
    }
    handle.set_offset(off);
    true
}

fn clamp_neg(offset: f32, max: f32) -> f32 {
    if !offset.is_finite() {
        return 0.0;
    }
    if !max.is_finite() || max <= 0.0 {
        return 0.0;
    }
    offset.clamp(-max, 0.0)
}

/// Window-level mouse capture so scrollbar thumbs stay draggable after the
/// cursor leaves the 12px hit target. GPUI `on_mouse_move` only fires when
/// that element's hitbox is hovered.
///
/// `view` is captured at render. Do not call `window.current_view()` from
/// these mouse callbacks — the view stack can be empty (`panic = "abort"`).
pub struct ThumbDragCatcher {
    drag: Rc<RefCell<Option<ScrollThumbDrag>>>,
    view: EntityId,
}

impl ThumbDragCatcher {
    pub fn new(drag: Rc<RefCell<Option<ScrollThumbDrag>>>, view: EntityId) -> Self {
        Self { drag, view }
    }
}

impl IntoElement for ThumbDragCatcher {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ThumbDragCatcher {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (window.request_layout(Style::default(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<Pixels>,
        _state: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<Pixels>,
        _request: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        let drag = self.drag.clone();
        let view = self.view;
        window.on_mouse_event({
            let drag = drag.clone();
            move |event: &MouseMoveEvent, phase, _window, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                if apply_thumb_drag(&drag, event.position) {
                    cx.notify(view);
                }
            }
        });
        window.on_mouse_event({
            let drag = drag.clone();
            move |event: &MouseUpEvent, phase, _, _| {
                if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                    drag.borrow_mut().take();
                }
            }
        });
    }
}

fn thumb_visible(hovered: bool, drag: &RefCell<Option<ScrollThumbDrag>>, vertical: bool) -> bool {
    hovered
        || drag
            .borrow()
            .as_ref()
            .is_some_and(|d| d.vertical == vertical)
}

fn begin_thumb_drag(
    drag: &Rc<RefCell<Option<ScrollThumbDrag>>>,
    vertical: bool,
    mouse: f32,
    travel: f32,
    max: f32,
    handle: &ScrollHandle,
) {
    let start_offset = if vertical {
        f32::from(handle.offset().y)
    } else {
        f32::from(handle.offset().x)
    };
    *drag.borrow_mut() = Some(ScrollThumbDrag {
        vertical,
        start_mouse: mouse,
        start_offset,
        travel,
        max,
        handle: handle.clone(),
    });
}

pub fn overlay_scrollbar(
    id: impl Into<SharedString>,
    axis: ScrollAxis,
    handle: &ScrollHandle,
    drag: &Rc<RefCell<Option<ScrollThumbDrag>>>,
    visible: bool,
) -> AnyElement {
    let vertical = matches!(axis, ScrollAxis::Vertical);
    let max: f32 = if vertical {
        handle.max_offset().height.into()
    } else {
        handle.max_offset().width.into()
    };
    let view: f32 = if vertical {
        handle.bounds().size.height.into()
    } else {
        handle.bounds().size.width.into()
    };
    let offset: f32 = if vertical {
        handle.offset().y.into()
    } else {
        handle.offset().x.into()
    };
    let show = max > 1.0 && view > 1.0 && visible;
    let thumb_len = if show {
        (view * view / (view + max)).clamp(24.0, view)
    } else {
        0.0
    };
    let travel = (view - thumb_len).max(0.0);
    let pos = if max > 0.0 {
        (-offset / max).clamp(0.0, 1.0) * travel
    } else {
        0.0
    };

    if !show {
        return div().into_any();
    }
    let handle = handle.clone();
    let drag = drag.clone();
    let hit = div()
        .id(id.into())
        .absolute()
        .occlude()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, {
            let drag = drag.clone();
            let handle = handle.clone();
            move |ev, _, cx| {
                let mouse = if vertical {
                    f32::from(ev.position.y)
                } else {
                    f32::from(ev.position.x)
                };
                begin_thumb_drag(&drag, vertical, mouse, travel, max, &handle);
                cx.stop_propagation();
            }
        });
    if vertical {
        hit.top(px(pos))
            .right(px(0.))
            .w(px(12.))
            .h(px(thumb_len))
            .child(
                div()
                    .ml(px(3.))
                    .w(px(6.))
                    .h_full()
                    .rounded_full()
                    .bg(theme::scrollbar_thumb()),
            )
            .into_any()
    } else {
        hit.left(px(pos))
            .bottom(px(0.))
            .h(px(12.))
            .w(px(thumb_len))
            .child(
                div()
                    .mt(px(3.))
                    .h(px(6.))
                    .w_full()
                    .rounded_full()
                    .bg(theme::scrollbar_thumb()),
            )
            .into_any()
    }
}

/// Parent-width horizontal scroller. The inner child must have an explicit
/// `content_w` or GPUI will not create a scroll region. Uses `overflow_x_hidden`
/// (not `overflow_x_scroll`) so a vertical wheel is not remapped onto X —
/// GPUI's default `overflow_x_scroll` maps `delta.y → delta.x`.
pub fn h_scroll_pane(
    id: impl Into<SharedString>,
    handle: &ScrollHandle,
    content_w: f32,
    content_h: f32,
    center: bool,
    drag: &Rc<RefCell<Option<ScrollThumbDrag>>>,
    hover: &Rc<RefCell<HashSet<String>>>,
    view: EntityId,
    content: impl IntoElement,
) -> AnyElement {
    let id = id.into();
    let pane_key = id.to_string();
    let scroll_id = SharedString::from(format!("{id}-h"));
    let inner_id = SharedString::from(format!("{id}-inner"));
    let thumb_id = SharedString::from(format!("{id}-thumb"));
    let content_w = content_w.max(1.0);
    let content_h = content_h.max(1.0);
    let view_w: f32 = handle.bounds().size.width.into();
    let inner_w = if center && view_w > content_w {
        view_w
    } else {
        content_w
    };

    let inner = div()
        .id(inner_id)
        .w(px(inner_w))
        .h(px(content_h))
        .flex_none()
        .when(center && inner_w > content_w + 0.5, |d| {
            d.flex().justify_center()
        })
        .child(
            div()
                .w(px(content_w))
                .h(px(content_h))
                .flex_none()
                .child(content),
        );

    let scroller = div()
        .id(scroll_id)
        .w_full()
        .min_w_0()
        .overflow_x_hidden()
        .track_scroll(handle)
        .on_scroll_wheel({
            let handle = handle.clone();
            move |event, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height());
                let dx: f32 = delta.x.into();
                let dy: f32 = delta.y.into();
                let mut pan = dx;
                if pan.abs() < 0.5 && window.modifiers().shift {
                    pan = dy;
                }
                if !pan.is_finite() || pan.abs() < 0.5 {
                    return;
                }
                let max_x: f32 = handle.max_offset().width.into();
                if !max_x.is_finite() || max_x <= 1.0 {
                    return;
                }
                let x: f32 = handle.offset().x.into();
                let mut off = handle.offset();
                off.x = px(clamp_neg(x + pan, max_x));
                handle.set_offset(off);
                cx.stop_propagation();
                cx.notify(view);
            }
        })
        .child(inner);

    let hovered = hover.borrow().contains(&pane_key);
    let chrome_id = SharedString::from(format!("{id}-chrome"));
    div()
        .id(chrome_id)
        .relative()
        .w_full()
        .min_w_0()
        .pb(px(10.))
        .on_hover({
            let hover = hover.clone();
            let pane_key = pane_key.clone();
            move |hovered, _, cx| {
                let mut set = hover.borrow_mut();
                let was = set.contains(&pane_key);
                if *hovered {
                    set.insert(pane_key.clone());
                } else {
                    set.remove(&pane_key);
                }
                if was != *hovered {
                    cx.notify(view);
                }
            }
        })
        .child(scroller)
        .child(overlay_scrollbar(
            thumb_id,
            ScrollAxis::Horizontal,
            handle,
            drag,
            thumb_visible(hovered, drag, false),
        ))
        .into_any()
}
