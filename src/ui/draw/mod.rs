mod ink;
mod view;

use std::time::Instant;

use gpui::{Pixels, Point};

use ink::{erase_and_split, StrokePt, ERASER_RADIUS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawTool {
    Pen,
    Eraser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawPaper {
    Dots,
    Lines,
    Blank,
}

pub(crate) struct DrawBoard {
    pub(crate) lines: Vec<Vec<StrokePt>>,
    painting: bool,
    t0: Option<Instant>,
    pub(crate) tool: DrawTool,
    pub(crate) paper: DrawPaper,
    undo: Vec<Vec<Vec<StrokePt>>>,
    redo: Vec<Vec<Vec<StrokePt>>>,
    gesture_before: Option<Vec<Vec<StrokePt>>>,
    pub(crate) hover: Option<Point<Pixels>>,
    pad_hovered: bool,
    dock_hovered: bool,
}

impl DrawBoard {
    pub(crate) fn new() -> Self {
        Self {
            lines: Vec::new(),
            painting: false,
            t0: None,
            tool: DrawTool::Pen,
            paper: DrawPaper::Dots,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture_before: None,
            hover: None,
            pad_hovered: false,
            dock_hovered: false,
        }
    }

    pub(crate) fn clear_ink(&mut self) {
        self.lines.clear();
        self.undo.clear();
        self.redo.clear();
        self.gesture_before = None;
        self.painting = false;
        self.t0 = None;
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn set_tool(&mut self, tool: DrawTool) {
        self.tool = tool;
    }

    pub(crate) fn set_paper(&mut self, paper: DrawPaper) {
        self.paper = paper;
    }

    pub(crate) fn set_pad_hovered(&mut self, hovered: bool) {
        self.pad_hovered = hovered;
        if !hovered {
            self.hover = None;
            self.dock_hovered = false;
        }
    }

    pub(crate) fn set_dock_hovered(&mut self, hovered: bool) {
        self.dock_hovered = hovered;
    }

    pub(crate) fn leave_canvas(&mut self) {
        self.set_pad_hovered(false);
    }

    /// Hide the OS pointer so the painted eraser ring is the only cursor.
    pub(crate) fn os_cursor_hidden(&self) -> bool {
        self.tool == DrawTool::Eraser && self.pad_hovered && !self.dock_hovered
    }

    pub(crate) fn eraser_ring(&self) -> Option<Point<Pixels>> {
        if self.tool != DrawTool::Eraser || self.dock_hovered {
            return None;
        }
        self.hover
    }

    pub(crate) fn begin_gesture(&mut self) {
        self.gesture_before = Some(self.lines.clone());
    }

    pub(crate) fn end_gesture(&mut self) {
        self.painting = false;
        let Some(before) = self.gesture_before.take() else {
            return;
        };
        if before != self.lines {
            self.undo.push(before);
            self.redo.clear();
        }
    }

    pub(crate) fn pointer_down(&mut self, pos: Point<Pixels>) {
        self.hover = Some(pos);
        self.pad_hovered = true;
        self.begin_gesture();
        self.painting = true;
        self.apply_tool(pos, true);
    }

    pub(crate) fn pointer_move(&mut self, pos: Point<Pixels>) -> bool {
        self.hover = Some(pos);
        self.pad_hovered = true;
        if self.painting {
            self.apply_tool(pos, false);
            return true;
        }
        self.tool == DrawTool::Eraser
    }

    pub(crate) fn pointer_up(&mut self) {
        self.end_gesture();
    }

    pub(crate) fn is_gesturing(&self) -> bool {
        self.painting || self.gesture_before.is_some()
    }

    fn apply_tool(&mut self, pos: Point<Pixels>, start: bool) {
        match self.tool {
            DrawTool::Pen => {
                let pt = self.stamp(pos);
                if start {
                    self.lines.push(vec![pt]);
                } else if let Some(line) = self.lines.last_mut() {
                    line.push(pt);
                }
            }
            DrawTool::Eraser => {
                let x = f32::from(pos.x);
                let y = f32::from(pos.y);
                erase_and_split(&mut self.lines, x, y, ERASER_RADIUS);
            }
        }
    }

    pub(crate) fn undo(&mut self) {
        let Some(prev) = self.undo.pop() else {
            return;
        };
        self.redo.push(self.lines.clone());
        self.lines = prev;
        self.painting = false;
        self.gesture_before = None;
    }

    pub(crate) fn redo(&mut self) {
        let Some(next) = self.redo.pop() else {
            return;
        };
        self.undo.push(self.lines.clone());
        self.lines = next;
        self.painting = false;
        self.gesture_before = None;
    }

    pub(crate) fn has_ink(&self) -> bool {
        self.lines.iter().any(|line| line.len() >= 2)
    }

    fn stamp(&mut self, pos: Point<Pixels>) -> StrokePt {
        let t0 = *self.t0.get_or_insert_with(Instant::now);
        StrokePt {
            pos,
            t_ms: t0.elapsed().as_secs_f32() * 1000.0,
        }
    }

    pub(crate) fn traces(&self) -> Vec<Vec<[f32; 3]>> {
        let mut traces: Vec<Vec<[f32; 3]>> = self
            .lines
            .iter()
            .filter(|line| line.len() >= 2)
            .map(|line| {
                line.iter()
                    .map(|p| [f32::from(p.pos.x), f32::from(p.pos.y), p.t_ms])
                    .collect()
            })
            .collect();
        crate::ocr::inktex::deburst(&mut traces);
        traces
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    #[test]
    fn undo_redo_and_new_stroke_clears_redo() {
        let mut b = DrawBoard::new();
        b.pointer_down(point(px(0.0), px(0.0)));
        let _ = b.pointer_move(point(px(4.0), px(0.0)));
        b.pointer_up();
        b.pointer_down(point(px(8.0), px(0.0)));
        let _ = b.pointer_move(point(px(12.0), px(0.0)));
        b.pointer_up();
        assert!(b.can_undo());
        b.undo();
        assert_eq!(b.lines.len(), 1);
        assert!(b.can_redo());
        b.redo();
        assert_eq!(b.lines.len(), 2);
        b.undo();
        b.pointer_down(point(px(0.0), px(0.0)));
        let _ = b.pointer_move(point(px(1.0), px(0.0)));
        b.pointer_up();
        assert!(!b.can_redo());
    }

    #[test]
    fn end_gesture_clears_painting_without_snapshot() {
        let mut b = DrawBoard::new();
        b.painting = true;
        b.end_gesture();
        assert!(!b.painting);
        assert!(!b.is_gesturing());
    }

    #[test]
    fn clear_ink_keeps_tool_and_paper() {
        let mut b = DrawBoard::new();
        b.pointer_down(point(px(0.0), px(0.0)));
        let _ = b.pointer_move(point(px(4.0), px(0.0)));
        b.pointer_up();
        b.set_tool(DrawTool::Eraser);
        b.set_paper(DrawPaper::Lines);
        b.clear_ink();
        assert!(b.lines.is_empty());
        assert!(!b.has_ink());
        assert_eq!(b.tool, DrawTool::Eraser);
        assert_eq!(b.paper, DrawPaper::Lines);
    }

    #[test]
    fn eraser_hides_os_cursor_only_on_pad() {
        let mut b = DrawBoard::new();
        assert!(!b.os_cursor_hidden());
        b.set_tool(DrawTool::Eraser);
        assert!(!b.os_cursor_hidden());
        let _ = b.pointer_move(point(px(8.0), px(8.0)));
        assert!(b.os_cursor_hidden());
        assert!(b.eraser_ring().is_some());
        b.set_dock_hovered(true);
        assert!(!b.os_cursor_hidden());
        assert!(b.eraser_ring().is_none());
        b.set_dock_hovered(false);
        assert!(b.os_cursor_hidden());
        b.set_tool(DrawTool::Pen);
        assert!(!b.os_cursor_hidden());
        b.set_tool(DrawTool::Eraser);
        b.leave_canvas();
        assert!(!b.os_cursor_hidden());
        assert!(b.hover.is_none());
    }
}
