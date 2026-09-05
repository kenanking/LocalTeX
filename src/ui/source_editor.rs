use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, App, AvailableSpace,
    Bounds, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, Font, GlobalElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, SharedString, Size,
    Style, TextAlign, TextRun, UTF16Selection, Window, WrappedLine,
};

use super::text_buffer::TextBuffer;
use super::theme;
use crate::prefs::ContentFontSize;

actions!(
    source_editor,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Enter,
        Undo,
        Redo,
    ]
);

const UNDO_CAP: usize = 64;

pub struct SourceEditor {
    focus_handle: FocusHandle,
    buf: TextBuffer,
    last_lines: Vec<(usize, WrappedLine)>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    font_px: f32,
    line_px: f32,
}

impl SourceEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let m = ContentFontSize::Medium.metrics();
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            buf: TextBuffer::new(),
            last_lines: Vec::new(),
            last_bounds: None,
            last_line_height: px(m.body_line),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            font_px: m.body,
            line_px: m.body_line,
        }
    }

    pub fn text(&self) -> String {
        self.buf.content.to_string()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn set_content_font(&mut self, size: ContentFontSize, cx: &mut Context<Self>) {
        let m = size.metrics();
        if self.font_px == m.body && self.line_px == m.body_line {
            return;
        }
        self.font_px = m.body;
        self.line_px = m.body_line;
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.buf.content = text.into();
        let len = self.buf.content.len();
        self.buf.selected_range = len..len;
        self.buf.marked_range = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        cx.notify();
    }

    pub fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(prev) = self.undo_stack.pop() else {
            return;
        };
        self.redo_stack.push(self.buf.content.to_string());
        self.buf.content = prev.into();
        self.buf.marked_range = None;
        self.buf.selection_reversed = false;
        let len = self.buf.content.len();
        self.buf.selected_range = len..len;
        cx.notify();
    }

    pub fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(next) = self.redo_stack.pop() else {
            return;
        };
        self.undo_stack.push(self.buf.content.to_string());
        self.buf.content = next.into();
        self.buf.marked_range = None;
        self.buf.selection_reversed = false;
        let len = self.buf.content.len();
        self.buf.selected_range = len..len;
        cx.notify();
    }

    pub fn undo_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.undo(&Undo, window, cx);
    }

    pub fn redo_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redo(&Redo, window, cx);
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(self.buf.content.to_string());
        if self.undo_stack.len() > UNDO_CAP {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.buf.selected_range.is_empty() {
            self.move_to(self.buf.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.buf.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.buf.selected_range.is_empty() {
            self.move_to(self.buf.next_boundary(self.buf.selected_range.end), cx);
        } else {
            self.move_to(self.buf.selected_range.end, cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.offset_on_neighbor_line(self.cursor_offset(), -1), cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.offset_on_neighbor_line(self.cursor_offset(), 1), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buf.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buf.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.buf.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.cursor_offset();
        if let (Some(bounds), Some(pos)) = (self.last_bounds, self.visual_pos(offset)) {
            let y = bounds.top() + pos.y + self.last_line_height * 0.5;
            self.move_to(self.index_for_mouse_position(point(bounds.left(), y)), cx);
        } else {
            self.move_to(self.line_start(offset), cx);
        }
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.cursor_offset();
        if let (Some(bounds), Some(pos)) = (self.last_bounds, self.visual_pos(offset)) {
            let y = bounds.top() + pos.y + self.last_line_height * 0.5;
            self.move_to(self.index_for_mouse_position(point(bounds.right(), y)), cx);
        } else {
            self.move_to(self.line_end(offset), cx);
        }
    }

    fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx);
        cx.stop_propagation();
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.selected_range.is_empty() {
            self.select_to(self.buf.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
        cx.stop_propagation();
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.selected_range.is_empty() {
            self.select_to(self.buf.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
        cx.stop_propagation();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.buf.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx);
        }
        cx.stop_propagation();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.buf.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.buf.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
        cx.stop_propagation();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.buf.write_selection(cx);
        cx.stop_propagation();
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.buf.selected_text().is_some() {
            self.buf.write_selection(cx);
            self.replace_text_in_range(None, "", window, cx);
        }
        cx.stop_propagation();
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.buf.move_to(offset);
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        self.buf.cursor_offset()
    }

    fn line_start(&self, offset: usize) -> usize {
        self.buf.content[..offset]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    fn line_end(&self, offset: usize) -> usize {
        self.buf.content[offset..]
            .find('\n')
            .map(|i| offset + i)
            .unwrap_or(self.buf.content.len())
    }

    fn visual_pos(&self, offset: usize) -> Option<Point<Pixels>> {
        let lh = self.last_line_height;
        let mut y = px(0.);
        for (i, (start, line)) in self.last_lines.iter().enumerate() {
            let end = start + line.len();
            let last = i + 1 == self.last_lines.len();
            if offset <= end || last {
                let p = line.position_for_index(offset.saturating_sub(*start), lh)?;
                return Some(point(p.x, y + p.y));
            }
            y += line.size(lh).height;
        }
        None
    }

    fn offset_on_neighbor_line(&self, offset: usize, dir: i32) -> usize {
        let Some(bounds) = self.last_bounds else {
            return offset;
        };
        let Some(pos) = self.visual_pos(offset) else {
            return offset;
        };
        let next_y = pos.y + self.last_line_height * (dir as f32);
        if next_y < px(0.) {
            return offset;
        }
        self.index_for_mouse_position(point(bounds.left() + pos.x, bounds.top() + next_y))
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        if self.last_lines.is_empty() {
            return 0;
        }
        let local = point(
            (position.x - bounds.left()).max(px(0.)),
            (position.y - bounds.top()).max(px(0.)),
        );
        let lh = self.last_line_height;
        let mut y = px(0.);
        for (i, (start, line)) in self.last_lines.iter().enumerate() {
            let h = line.size(lh).height;
            let last = i + 1 == self.last_lines.len();
            if local.y < y + h || last {
                let col =
                    wrap_index(line.closest_index_for_position(point(local.x, local.y - y), lh));
                return (start + col).min(self.buf.content.len());
            }
            y += h;
        }
        self.buf.content.len()
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.buf.select_to(offset);
        cx.notify();
    }
}

impl EntityInputHandler for SourceEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        self.buf.text_for_utf16(range_utf16, actual_range)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(self.buf.selected_utf16())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.buf
            .marked_range
            .as_ref()
            .map(|range| self.buf.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.buf.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push_undo();
        self.buf.replace_text(range_utf16, new_text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push_undo();
        self.buf
            .replace_and_mark(range_utf16, new_text, new_selected_range_utf16);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.buf.range_from_utf16(&range_utf16);
        let p0 = self.visual_pos(range.start)?;
        let p1 = self.visual_pos(range.end).unwrap_or(p0);
        let lh = self.last_line_height;
        Some(Bounds::from_corners(
            point(bounds.left() + p0.x, bounds.top() + p0.y),
            point(
                bounds.left() + p1.x.max(p0.x + px(2.)),
                bounds.top() + p0.y + lh,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(
            self.buf
                .offset_to_utf16(self.index_for_mouse_position(point)),
        )
    }
}

fn wrap_index(result: Result<usize, usize>) -> usize {
    result.unwrap_or_else(|i| i)
}

fn shape_wrapped(
    window: &mut Window,
    content: SharedString,
    wrap_width: Option<Pixels>,
    font: Font,
    font_size: Pixels,
    line_height: Pixels,
) -> (Vec<(usize, WrappedLine)>, Pixels) {
    let run = TextRun {
        len: content.len(),
        font,
        color: rgb(theme::TEXT).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs = if content.is_empty() {
        Vec::new()
    } else {
        vec![run]
    };
    let shaped = window
        .text_system()
        .shape_text(content, font_size, &runs, wrap_width, None)
        .unwrap_or_default();
    let n = shaped.len();
    let mut lines = Vec::with_capacity(n);
    let mut byte = 0usize;
    let mut height = px(0.);
    for (i, line) in shaped.into_iter().enumerate() {
        height += line.size(line_height).height;
        let len = line.len();
        lines.push((byte, line));
        byte += len;
        if i + 1 < n {
            byte += 1;
        }
    }
    if lines.is_empty() {
        height = line_height;
    }
    (lines, height)
}

struct FieldElement {
    input: Entity<SourceEditor>,
}

struct PrepaintState {
    lines: Vec<(usize, WrappedLine)>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    height: Pixels,
}

impl IntoElement for FieldElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for FieldElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.clone();
        let font = window.text_style().font();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let id = window.request_measured_layout(style, move |known, available, window, cx| {
            let wrap_width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
            });
            let ed = input.read(cx);
            let content = ed.buf.content.clone();
            let font_size = px(ed.font_px);
            let line_height = px(ed.line_px);
            let (_, height) = shape_wrapped(
                window,
                content,
                wrap_width,
                font.clone(),
                font_size,
                line_height,
            );
            let width = wrap_width.unwrap_or(px(0.));
            Size { width, height }
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.buf.content.clone();
        let selected_range = input.buf.selected_range.clone();
        let cursor = input.cursor_offset();
        let font_size = px(input.font_px);
        let line_height = px(input.line_px);
        let font = window.text_style().font();
        let wrap_width = if bounds.size.width > px(0.) {
            Some(bounds.size.width)
        } else {
            None
        };
        let (lines, height) =
            shape_wrapped(window, content, wrap_width, font, font_size, line_height);
        let mut selection = Vec::new();
        let mut cursor_quad = None;
        let mut y = bounds.top();
        for (start, line) in &lines {
            let end = start + line.len();
            let h = line.size(line_height).height;
            let n_rows = ((f32::from(h) / f32::from(line_height).max(1.0)).round() as usize).max(1);
            if selected_range.start < selected_range.end
                && selected_range.start <= end
                && selected_range.end >= *start
            {
                for row in 0..n_rows {
                    let row_y = line_height * (row as f32);
                    let mid_y = row_y + line_height * 0.5;
                    let row_start = wrap_index(
                        line.closest_index_for_position(point(px(0.), mid_y), line_height),
                    );
                    let row_end =
                        wrap_index(line.closest_index_for_position(
                            point(bounds.size.width, mid_y),
                            line_height,
                        ));
                    let a = selected_range.start.max(*start + row_start);
                    let b = selected_range.end.min(*start + row_end);
                    if a < b {
                        let x0 = if a <= *start + row_start {
                            px(0.)
                        } else {
                            line.position_for_index(a - *start, line_height)
                                .map(|p| p.x)
                                .unwrap_or(px(0.))
                        };
                        let x1 = if b >= *start + row_end {
                            bounds.size.width
                        } else {
                            line.position_for_index(b - *start, line_height)
                                .map(|p| p.x)
                                .unwrap_or(bounds.size.width)
                        };
                        selection.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + x0, y + row_y),
                                point(bounds.left() + x1.max(x0 + px(1.)), y + row_y + line_height),
                            ),
                            rgba(0x2563eb30),
                        ));
                    }
                }
            }
            if cursor >= *start && cursor <= end {
                if let Some(p) = line.position_for_index(cursor.saturating_sub(*start), line_height)
                {
                    cursor_quad = Some(fill(
                        Bounds::new(
                            point(bounds.left() + p.x, y + p.y),
                            size(px(2.), line_height),
                        ),
                        rgb(theme::ACCENT),
                    ));
                }
            }
            y += h;
        }
        PrepaintState {
            lines,
            cursor: cursor_quad,
            selection,
            height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        for quad in prepaint.selection.drain(..) {
            window.paint_quad(quad);
        }
        let line_height = px(self.input.read(cx).line_px);
        let mut origin = bounds.origin;
        for (_, line) in prepaint.lines.iter() {
            let _ = line.paint(
                origin,
                line_height,
                TextAlign::Left,
                Some(bounds),
                window,
                cx,
            );
            origin.y += line.size(line_height).height;
        }
        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        let lines = std::mem::take(&mut prepaint.lines);
        let height = prepaint.height;
        self.input.update(cx, |input, _cx| {
            input.last_lines = lines;
            input.last_bounds = Some(bounds);
            input.last_line_height = line_height;
            let _ = height;
        });
    }
}

impl Render for SourceEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = cx.entity();
        div()
            .id("source-editor")
            .accessibility_id("source-editor")
            .role(gpui::Role::MultilineTextInput)
            .aria_label("Source content")
            .aria_value(self.buf.content.clone())
            .on_a11y_action(gpui::AccessibleAction::SetValue, move |data, _, cx| {
                if let Some(gpui::accesskit::ActionData::Value(text)) = data {
                    input.update(cx, |input, cx| {
                        input.push_undo();
                        input.buf.marked_range = None;
                        input.buf.selected_range = 0..input.buf.content.len();
                        input.buf.replace_text(None, text);
                        cx.notify();
                    });
                }
            })
            .flex()
            .flex_col()
            .key_context("SourceEditor")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .min_w_0()
            .pb(px(52.))
            .text_size(px(self.font_px))
            .line_height(px(self.line_px))
            .font_family(theme::SOURCE_FONT)
            .text_color(rgb(theme::TEXT))
            .child(FieldElement { input: cx.entity() })
    }
}

impl Focusable for SourceEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("SourceEditor")),
        KeyBinding::new("delete", Delete, Some("SourceEditor")),
        KeyBinding::new("left", Left, Some("SourceEditor")),
        KeyBinding::new("right", Right, Some("SourceEditor")),
        KeyBinding::new("up", Up, Some("SourceEditor")),
        KeyBinding::new("down", Down, Some("SourceEditor")),
        KeyBinding::new("shift-left", SelectLeft, Some("SourceEditor")),
        KeyBinding::new("shift-right", SelectRight, Some("SourceEditor")),
        KeyBinding::new("ctrl-a", SelectAll, Some("SourceEditor")),
        KeyBinding::new("cmd-a", SelectAll, Some("SourceEditor")),
        KeyBinding::new("ctrl-v", Paste, Some("SourceEditor")),
        KeyBinding::new("cmd-v", Paste, Some("SourceEditor")),
        KeyBinding::new("ctrl-c", Copy, Some("SourceEditor")),
        KeyBinding::new("cmd-c", Copy, Some("SourceEditor")),
        KeyBinding::new("ctrl-x", Cut, Some("SourceEditor")),
        KeyBinding::new("cmd-x", Cut, Some("SourceEditor")),
        KeyBinding::new("home", Home, Some("SourceEditor")),
        KeyBinding::new("end", End, Some("SourceEditor")),
        KeyBinding::new("enter", Enter, Some("SourceEditor")),
        KeyBinding::new("ctrl-z", Undo, Some("SourceEditor")),
        KeyBinding::new("cmd-z", Undo, Some("SourceEditor")),
        KeyBinding::new("ctrl-shift-z", Redo, Some("SourceEditor")),
        KeyBinding::new("cmd-shift-z", Redo, Some("SourceEditor")),
    ]);
}
