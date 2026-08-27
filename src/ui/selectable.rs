//! Read-only mouse selection for preview labels (gpui 0.2 has no TextView).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    fill, point, prelude::*, rgb, App, Bounds, CursorStyle, DispatchPhase, Element, ElementId,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString, StyledText, TextLayout,
    Window,
};

use super::theme;

#[derive(Clone, Default)]
pub struct PreviewSel {
    pub key: String,
    pub text: SharedString,
    pub anchor: usize,
    pub head: usize,
    pub dragging: bool,
}

impl PreviewSel {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn range(&self) -> Range<usize> {
        if self.anchor <= self.head {
            self.anchor..self.head
        } else {
            self.head..self.anchor
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        let range = self.range();
        if range.start == range.end {
            return None;
        }
        self.text.get(range.clone()).map(str::to_string)
    }
}

pub fn selectable_text(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    sel: Rc<RefCell<PreviewSel>>,
) -> SelectableText {
    let id = id.into();
    let text = text.into();
    SelectableText {
        key: id.to_string(),
        element_id: id.into(),
        styled: StyledText::new(text.clone()),
        text,
        sel,
    }
}

pub struct SelectableText {
    key: String,
    element_id: ElementId,
    styled: StyledText,
    text: SharedString,
    sel: Rc<RefCell<PreviewSel>>,
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.element_id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.styled.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled
            .prepaint(None, inspector_id, bounds, state, window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        let key = self.key.clone();
        let text = self.text.clone();
        let sel = self.sel.clone();
        let view = window.current_view();
        let hitbox_id = hitbox.clone();

        window.set_cursor_style(CursorStyle::IBeam, hitbox);

        {
            let hitbox = hitbox_id.clone();
            let layout = layout.clone();
            let key = key.clone();
            let text = text.clone();
            let sel = sel.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let ix = snap_boundary(
                    &text,
                    layout
                        .index_for_position(event.position)
                        .unwrap_or_else(|e| e),
                );
                {
                    let mut state = sel.borrow_mut();
                    state.key = key.clone();
                    state.text = text.clone();
                    if event.click_count >= 2 {
                        state.anchor = 0;
                        state.head = text.len();
                        state.dragging = false;
                    } else {
                        state.anchor = ix;
                        state.head = ix;
                        state.dragging = true;
                    }
                }
                cx.notify(view);
            });
        }

        {
            let layout = layout.clone();
            let key = key.clone();
            let text = text.clone();
            let sel = sel.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let dragging = {
                    let state = sel.borrow();
                    state.dragging && state.key == key
                };
                if !dragging {
                    return;
                }
                let ix = snap_boundary(
                    &text,
                    layout
                        .index_for_position(event.position)
                        .unwrap_or_else(|e| e),
                );
                {
                    let mut state = sel.borrow_mut();
                    if state.head == ix {
                        return;
                    }
                    state.head = ix;
                }
                cx.notify(view);
            });
        }

        {
            let key = key.clone();
            let sel = sel.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let mut state = sel.borrow_mut();
                if state.dragging && state.key == key {
                    state.dragging = false;
                    cx.notify(view);
                }
            });
        }

        let highlight = {
            let state = sel.borrow();
            if state.key == key {
                Some(state.range())
            } else {
                None
            }
        };
        if let Some(range) = highlight {
            paint_selection(&layout, range, window);
        }

        self.styled
            .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);
    }
}

fn snap_boundary(s: &str, mut i: usize) -> usize {
    if i > s.len() {
        i = s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn paint_selection(layout: &TextLayout, range: Range<usize>, window: &mut Window) {
    if range.start == range.end {
        return;
    }
    let Some(p0) = layout.position_for_index(range.start) else {
        return;
    };
    let Some(p1) = layout.position_for_index(range.end) else {
        return;
    };
    let lh = layout.line_height();
    let bounds = layout.bounds();
    let mut color = rgb(theme::ACCENT);
    color.a = 0.22;

    let same_line = {
        let y0: f32 = p0.y.into();
        let y1: f32 = p1.y.into();
        (y0 - y1).abs() < 0.5
    };
    if same_line {
        window.paint_quad(fill(
            Bounds::from_corners(p0, point(p1.x, p0.y + lh)),
            color,
        ));
        return;
    }

    window.paint_quad(fill(
        Bounds::from_corners(p0, point(bounds.right(), p0.y + lh)),
        color,
    ));
    let mut y = p0.y + lh;
    while {
        let yf: f32 = y.into();
        let p1y: f32 = p1.y.into();
        yf + 0.5 < p1y
    } {
        window.paint_quad(fill(
            Bounds::from_corners(point(bounds.left(), y), point(bounds.right(), y + lh)),
            color,
        ));
        y += lh;
    }
    window.paint_quad(fill(
        Bounds::from_corners(point(bounds.left(), p1.y), point(p1.x, p1.y + lh)),
        color,
    ));
}
