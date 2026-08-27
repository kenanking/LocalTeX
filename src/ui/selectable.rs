//! Read-only mouse selection for preview labels (gpui 0.2 has no TextView).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    fill, point, prelude::*, rgb, App, Bounds, CursorStyle, DispatchPhase, Element, ElementId,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString, StyledText, TextLayout,
    Window,
};

use super::theme;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caret {
    pub key: String,
    pub offset: usize,
}

#[derive(Clone, Default)]
pub struct PreviewSel {
    pub anchor: Caret,
    pub head: Caret,
    pub dragging: bool,
    /// Document order of paragraph / fallback blocks. Display math and
    /// tables are omitted (block gaps). Table cells keep their own keys
    /// and never appear here.
    pub blocks: Vec<(String, SharedString)>,
    texts: HashMap<String, SharedString>,
}

impl PreviewSel {
    pub fn clear(&mut self) {
        self.anchor = Caret::default();
        self.head = Caret::default();
        self.dragging = false;
    }

    pub fn remember(&mut self, key: impl Into<String>, text: SharedString) {
        self.texts.insert(key.into(), text);
    }

    pub fn set_flow(&mut self, blocks: Vec<(String, SharedString)>) {
        self.blocks = blocks;
        for (k, t) in &self.blocks {
            self.texts.insert(k.clone(), t.clone());
        }
        if self.anchor.key.is_empty() {
            return;
        }
        if is_flow_key(&self.anchor.key) && self.flow_index(&self.anchor.key).is_none() {
            self.clear();
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.ordered()?;
        if start == end {
            return None;
        }
        if start.key == end.key {
            let text = self.text_for(&start.key)?;
            return text.get(start.offset..end.offset).map(str::to_string);
        }
        let ia = self.flow_index(&start.key)?;
        let ib = self.flow_index(&end.key)?;
        let mut out = String::new();
        for i in ia..=ib {
            let (_, text) = &self.blocks[i];
            if i > ia {
                out.push_str("\n\n");
            }
            let slice = if i == ia {
                text.get(start.offset..).unwrap_or("")
            } else if i == ib {
                text.get(..end.offset).unwrap_or("")
            } else {
                text.as_ref()
            };
            out.push_str(slice);
        }
        Some(out)
    }

    /// Byte range inside `key`'s paragraph string, if that block is selected.
    pub fn span_in(&self, key: &str, para_len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered()?;
        if start.key == end.key {
            if start.key != key || start.offset == end.offset {
                return None;
            }
            return Some(start.offset..end.offset);
        }
        let ia = self.flow_index(&start.key)?;
        let ib = self.flow_index(&end.key)?;
        let i = self.flow_index(key)?;
        if i < ia || i > ib {
            return None;
        }
        let range = if i == ia {
            start.offset..para_len
        } else if i == ib {
            0..end.offset
        } else {
            0..para_len
        };
        (range.start < range.end).then_some(range)
    }

    fn ordered(&self) -> Option<(Caret, Caret)> {
        if self.anchor.key.is_empty() || self.head.key.is_empty() {
            return None;
        }
        if self.anchor.key == self.head.key {
            return if self.anchor.offset <= self.head.offset {
                Some((self.anchor.clone(), self.head.clone()))
            } else {
                Some((self.head.clone(), self.anchor.clone()))
            };
        }
        let ia = self.flow_index(&self.anchor.key)?;
        let ib = self.flow_index(&self.head.key)?;
        if (ia, self.anchor.offset) <= (ib, self.head.offset) {
            Some((self.anchor.clone(), self.head.clone()))
        } else {
            Some((self.head.clone(), self.anchor.clone()))
        }
    }

    fn flow_index(&self, key: &str) -> Option<usize> {
        self.blocks.iter().position(|(k, _)| k == key)
    }

    fn text_for(&self, key: &str) -> Option<&SharedString> {
        self.texts.get(key)
    }
}

fn is_flow_key(key: &str) -> bool {
    key.starts_with("p-") || key.starts_with("f-")
}

fn can_extend(anchor_key: &str, target_key: &str) -> bool {
    anchor_key == target_key || (is_flow_key(anchor_key) && is_flow_key(target_key))
}

pub fn selectable_text(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    sel: Rc<RefCell<PreviewSel>>,
) -> SelectableText {
    let id = id.into();
    let text = text.into();
    selectable_run(id.clone(), id, text.clone(), text, 0, sel)
}

/// One text run inside a paragraph (or table cell). `para_key` is the
/// `PreviewSel` identity; `element_id` must be unique so hitboxes do not
/// collapse. Mouse-move only updates `head` when this run's hitbox is
/// hovered, so a drag can cross paragraphs without last-writer-wins.
pub fn selectable_run(
    element_id: impl Into<SharedString>,
    para_key: impl Into<SharedString>,
    run_text: impl Into<SharedString>,
    para_text: impl Into<SharedString>,
    seg_start: usize,
    sel: Rc<RefCell<PreviewSel>>,
) -> SelectableText {
    let element_id = element_id.into();
    let para_key = para_key.into();
    let run_text = run_text.into();
    let para_text = para_text.into();
    SelectableText {
        key: para_key.to_string(),
        element_id: element_id.into(),
        styled: StyledText::new(run_text.clone()),
        run_text,
        para_text,
        seg_start,
        sel,
    }
}

pub struct SelectableText {
    key: String,
    element_id: ElementId,
    styled: StyledText,
    run_text: SharedString,
    para_text: SharedString,
    seg_start: usize,
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
        let run_text = self.run_text.clone();
        let para_text = self.para_text.clone();
        let seg_start = self.seg_start;
        let sel = self.sel.clone();
        let view = window.current_view();
        let hitbox_id = hitbox.clone();

        window.set_cursor_style(CursorStyle::IBeam, hitbox);

        {
            let hitbox = hitbox_id.clone();
            let layout = layout.clone();
            let key = key.clone();
            let run_text = run_text.clone();
            let para_text = para_text.clone();
            let sel = sel.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let local = snap_boundary(
                    &run_text,
                    layout
                        .index_for_position(event.position)
                        .unwrap_or_else(|e| e),
                );
                {
                    let mut state = sel.borrow_mut();
                    state.remember(key.clone(), para_text.clone());
                    if event.click_count >= 2 {
                        state.anchor = Caret {
                            key: key.clone(),
                            offset: 0,
                        };
                        state.head = Caret {
                            key: key.clone(),
                            offset: para_text.len(),
                        };
                        state.dragging = false;
                    } else {
                        let ix = seg_start + local;
                        let caret = Caret {
                            key: key.clone(),
                            offset: ix,
                        };
                        state.anchor = caret.clone();
                        state.head = caret;
                        state.dragging = true;
                    }
                }
                cx.notify(view);
            });
        }

        {
            let hitbox = hitbox_id.clone();
            let layout = layout.clone();
            let key = key.clone();
            let run_text = run_text.clone();
            let para_text = para_text.clone();
            let sel = sel.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if !sel.borrow().dragging {
                    return;
                }
                if !hitbox.is_hovered(window) {
                    return;
                }
                let local = snap_boundary(
                    &run_text,
                    layout
                        .index_for_position(event.position)
                        .unwrap_or_else(|e| e),
                );
                let ix = seg_start + local;
                {
                    let mut state = sel.borrow_mut();
                    if !can_extend(&state.anchor.key, &key) {
                        return;
                    }
                    if state.head.key == key && state.head.offset == ix {
                        return;
                    }
                    state.remember(key.clone(), para_text.clone());
                    state.head = Caret {
                        key: key.clone(),
                        offset: ix,
                    };
                }
                cx.notify(view);
            });
        }

        {
            let sel = sel.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let mut state = sel.borrow_mut();
                if state.dragging {
                    state.dragging = false;
                    cx.notify(view);
                }
            });
        }

        let highlight = {
            let state = sel.borrow();
            let run_end = seg_start + run_text.len();
            state
                .span_in(&key, para_text.len())
                .and_then(|para| overlap(para, seg_start..run_end))
                .map(|ov| (ov.start - seg_start)..(ov.end - seg_start))
        };
        if let Some(range) = highlight {
            paint_selection(&layout, range, window);
        }

        self.styled
            .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);
    }
}

fn overlap(a: Range<usize>, b: Range<usize>) -> Option<Range<usize>> {
    let start = a.start.max(b.start);
    let end = a.end.min(b.end);
    (start < end).then_some(start..end)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn flow_sel(blocks: &[(&str, &str)]) -> PreviewSel {
        let mut sel = PreviewSel::default();
        sel.set_flow(
            blocks
                .iter()
                .map(|(k, t)| (k.to_string(), SharedString::from((*t).to_string())))
                .collect(),
        );
        sel
    }

    #[test]
    fn selected_text_single_block() {
        let mut sel = flow_sel(&[("p-0", "abcdef")]);
        sel.anchor = Caret {
            key: "p-0".into(),
            offset: 1,
        };
        sel.head = Caret {
            key: "p-0".into(),
            offset: 4,
        };
        assert_eq!(sel.selected_text().as_deref(), Some("bcd"));
    }

    #[test]
    fn selected_text_joins_paragraphs() {
        let mut sel = flow_sel(&[("p-0", "aaa"), ("p-1", "bbb")]);
        sel.anchor = Caret {
            key: "p-0".into(),
            offset: 1,
        };
        sel.head = Caret {
            key: "p-1".into(),
            offset: 2,
        };
        assert_eq!(sel.selected_text().as_deref(), Some("aa\n\nbb"));
    }

    #[test]
    fn selected_text_reverse_drag() {
        let mut sel = flow_sel(&[("p-0", "aaa"), ("p-1", "bbb")]);
        sel.anchor = Caret {
            key: "p-1".into(),
            offset: 2,
        };
        sel.head = Caret {
            key: "p-0".into(),
            offset: 1,
        };
        assert_eq!(sel.selected_text().as_deref(), Some("aa\n\nbb"));
    }

    #[test]
    fn span_in_middle_paragraph_is_full() {
        let mut sel = flow_sel(&[("p-0", "aa"), ("p-1", "bb"), ("p-2", "cc")]);
        sel.anchor = Caret {
            key: "p-0".into(),
            offset: 1,
        };
        sel.head = Caret {
            key: "p-2".into(),
            offset: 1,
        };
        assert_eq!(sel.span_in("p-0", 2), Some(1..2));
        assert_eq!(sel.span_in("p-1", 2), Some(0..2));
        assert_eq!(sel.span_in("p-2", 2), Some(0..1));
        assert_eq!(sel.span_in("p-9", 2), None);
    }

    #[test]
    fn display_gap_is_not_in_copy() {
        let mut sel = flow_sel(&[("p-0", "before"), ("p-2", "after")]);
        sel.anchor = Caret {
            key: "p-0".into(),
            offset: 0,
        };
        sel.head = Caret {
            key: "p-2".into(),
            offset: 5,
        };
        assert_eq!(sel.selected_text().as_deref(), Some("before\n\nafter"));
    }
}
