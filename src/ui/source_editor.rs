use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, App, Bounds, Context,
    CursorStyle, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    Focusable, GlobalElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString, Style, TextRun,
    UTF16Selection, Window,
};

use super::text_buffer::TextBuffer;
use super::theme;

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
    last_lines: Vec<(usize, ShapedLine)>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    skip_undo: bool,
}

impl SourceEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            buf: TextBuffer::new(),
            last_lines: Vec::new(),
            last_bounds: None,
            last_line_height: px(16.),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            skip_undo: false,
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

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.buf.content = text.into();
        let len = self.buf.content.len();
        self.buf.selected_range = len..len;
        self.buf.marked_range = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.skip_undo = true;
        cx.notify();
    }

    pub fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(prev) = self.undo_stack.pop() else {
            return;
        };
        self.redo_stack.push(self.buf.content.to_string());
        self.buf.content = prev.into();
        let len = self.buf.content.len();
        self.buf.selected_range = len..len;
        self.skip_undo = true;
        cx.notify();
    }

    pub fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(next) = self.redo_stack.pop() else {
            return;
        };
        self.undo_stack.push(self.buf.content.to_string());
        self.buf.content = next.into();
        let len = self.buf.content.len();
        self.buf.selected_range = len..len;
        self.skip_undo = true;
        cx.notify();
    }

    pub fn undo_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.undo(&Undo, window, cx);
    }

    pub fn redo_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redo(&Redo, window, cx);
    }

    fn push_undo(&mut self) {
        if self.skip_undo {
            self.skip_undo = false;
            return;
        }
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
        self.move_to(self.line_start(self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_end(self.cursor_offset()), cx);
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

    fn line_ranges(&self) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        let mut start = 0usize;
        for (i, ch) in self.buf.content.char_indices() {
            if ch == '\n' {
                ranges.push(start..i);
                start = i + 1;
            }
        }
        ranges.push(start..self.buf.content.len());
        ranges
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

    fn offset_on_neighbor_line(&self, offset: usize, dir: i32) -> usize {
        let ranges = self.line_ranges();
        let Some(ix) = ranges
            .iter()
            .position(|r| offset >= r.start && offset <= r.end)
        else {
            return offset;
        };
        let col = offset - ranges[ix].start;
        let next = if dir < 0 {
            ix.checked_sub(1)
        } else {
            Some(ix + 1).filter(|&i| i < ranges.len())
        };
        let Some(next) = next else {
            return offset;
        };
        let r = &ranges[next];
        (r.start + col).min(r.end)
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        if self.last_lines.is_empty() {
            return 0;
        }
        let y = f32::from(position.y - bounds.top()).max(0.0);
        let lh = f32::from(self.last_line_height).max(1.0);
        let line_ix = (y / lh).floor() as usize;
        let line_ix = line_ix.min(self.last_lines.len().saturating_sub(1));
        let (start, line) = &self.last_lines[line_ix];
        let col = line.closest_index_for_x(position.x - bounds.left());
        (start + col).min(self.buf.content.len())
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
        let lh = self.last_line_height;
        let (line_ix, col_start) = self.line_col(range.start);
        let (_, col_end) = self.line_col(range.end);
        let y = bounds.top() + lh * (line_ix as f32);
        let x0 = self
            .last_lines
            .get(line_ix)
            .map(|(_, line)| line.x_for_index(col_start))
            .unwrap_or(px(0.));
        let x1 = self
            .last_lines
            .get(line_ix)
            .map(|(_, line)| line.x_for_index(col_end))
            .unwrap_or(px(2.));
        Some(Bounds::from_corners(
            point(bounds.left() + x0, y),
            point(bounds.left() + x1.max(x0 + px(2.)), y + lh),
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

impl SourceEditor {
    fn line_col(&self, offset: usize) -> (usize, usize) {
        for (i, (start, line)) in self.last_lines.iter().enumerate() {
            let end = start + line.len();
            if offset <= end || i + 1 == self.last_lines.len() {
                return (i, offset.saturating_sub(*start));
            }
        }
        (0, offset)
    }
}

struct FieldElement {
    input: Entity<SourceEditor>,
}

struct PrepaintState {
    lines: Vec<(usize, ShapedLine)>,
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
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let n = self
            .input
            .read(cx)
            .buf
            .content
            .chars()
            .filter(|c| *c == '\n')
            .count()
            + 1;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * n as f32).into();
        (window.request_layout(style, [], cx), ())
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
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let run = TextRun {
            len: 0,
            font: style.font(),
            color: rgb(theme::TEXT).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut lines = Vec::new();
        let mut byte = 0usize;
        let text = content.to_string();
        let parts: Vec<String> = text.split('\n').map(str::to_string).collect();
        for (i, part) in parts.iter().enumerate() {
            let mut line_run = run.clone();
            line_run.len = part.len();
            let runs = if part.is_empty() {
                Vec::new()
            } else {
                vec![line_run]
            };
            let display = SharedString::from(part.clone());
            let shaped = window
                .text_system()
                .shape_line(display, font_size, &runs, None);
            lines.push((byte, shaped));
            byte += part.len();
            if i + 1 < parts.len() {
                byte += 1;
            }
        }
        let mut selection = Vec::new();
        let mut cursor_quad = None;
        for (i, (start, line)) in lines.iter().enumerate() {
            let end = start + line.len();
            let y = bounds.top() + line_height * (i as f32);
            let line_sel_start = selected_range.start.max(*start);
            let line_sel_end = selected_range.end.min(end);
            if line_sel_start < line_sel_end
                || (selected_range.start < *start && selected_range.end > end)
            {
                let a = selected_range.start.max(*start);
                let b = selected_range.end.min(end);
                if a < b || (selected_range.start <= *start && selected_range.end > end) {
                    let x0 = line.x_for_index(a.saturating_sub(*start));
                    let x1 = if selected_range.end > end {
                        bounds.size.width
                    } else {
                        line.x_for_index(b.saturating_sub(*start))
                    };
                    selection.push(fill(
                        Bounds::from_corners(
                            point(bounds.left() + x0, y),
                            point(bounds.left() + x1.max(x0 + px(1.)), y + line_height),
                        ),
                        rgba(0x2563eb30),
                    ));
                }
            }
            if cursor >= *start && cursor <= end {
                let x = line.x_for_index(cursor.saturating_sub(*start));
                cursor_quad = Some(fill(
                    Bounds::new(point(bounds.left() + x, y), size(px(2.), line_height)),
                    rgb(theme::ACCENT),
                ));
            }
        }
        PrepaintState {
            lines,
            cursor: cursor_quad,
            selection,
            height: line_height * (parts.len().max(1) as f32),
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
        let line_height = window.line_height();
        for (i, (_, line)) in prepaint.lines.iter().enumerate() {
            let origin = point(bounds.origin.x, bounds.origin.y + line_height * (i as f32));
            let _ = line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx);
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
        div()
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
            .px_2()
            .pt_2()
            .pb(px(52.))
            .text_xs()
            .font_family("monospace")
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
