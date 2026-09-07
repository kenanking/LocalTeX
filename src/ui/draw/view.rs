use gpui::{
    Bounds, Context, CursorStyle, MouseButton, MouseDownEvent, PathBuilder, Pixels, Point, canvas,
    div, fill, point, prelude::*, px, rgb,
};

use super::{DrawPaper, DrawTool};
use crate::actions::{DrawEraser, DrawPen, DrawRedo, DrawUndo};
use crate::i18n::t;
use crate::ui::main_window::{MainWindow, View};
use crate::ui::theme;
use crate::ui::widgets::{
    IconKind, btn, ghost_btn, icon_btn_kbd, section_label, seg_item, segmented,
};

use super::ink::ERASER_RADIUS;

const DOCK_W: f32 = 314.0;
const PAPER_PITCH: f32 = 20.0;
const PAPER_RULE_GAP: f32 = 28.0;
const INK_WIDTH: f32 = 2.4;

impl MainWindow {
    pub(crate) fn render_draw(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let lines: Vec<Vec<Point<Pixels>>> = self
            .board
            .lines
            .iter()
            .map(|line| line.iter().map(|p| p.pos).collect())
            .collect();
        let paper = self.board.paper;
        let tool = self.board.tool;
        let can_undo = self.board.can_undo();
        let can_redo = self.board.can_redo();
        let has_ink = self.board.has_ink();
        // Eraser has no GPUI hidden-cursor style (None was dropped). Arrow is
        // the fallback if the OS hide fails; the pad hides the pointer and
        // paints the ring as the cursor.
        let cursor = match tool {
            DrawTool::Pen => CursorStyle::Crosshair,
            DrawTool::Eraser => CursorStyle::Arrow,
        };
        let eraser_at = self.board.eraser_ring();
        let draw_focus = self.draw_focus.clone();

        div()
            .id("draw")
            .track_focus(&draw_focus)
            .key_context("DrawBoard")
            .on_action(cx.listener(|this, _: &DrawPen, window, cx| {
                this.board.set_tool(DrawTool::Pen);
                sync_eraser_os_cursor(&this.board, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DrawEraser, window, cx| {
                this.board.set_tool(DrawTool::Eraser);
                sync_eraser_os_cursor(&this.board, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DrawUndo, _, cx| {
                this.board.undo();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DrawRedo, _, cx| {
                this.board.redo();
                cx.notify();
            }))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG_RAISED))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(40.))
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER))
                    .child(section_label(t("draw.title")))
                    .child(div().flex_1())
                    .child(ghost_btn("draw-clear", t("draw.clear"), has_ink, {
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.board.clear_ink();
                                cx.notify();
                            });
                        }
                    }))
                    .child(ghost_btn("draw-cancel", t("draw.cancel"), true, {
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.dismiss_sheet(cx);
                            });
                        }
                    }))
                    .child(btn("draw-go", t("draw.recognize"), true, has_ink, {
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                let pts = this.board.traces();
                                this.board.clear_ink();
                                this.board.leave_canvas();
                                crate::desktop::set_os_cursor_visible(true);
                                this.view = View::Library;
                                this.state.update(cx, |s, cx| {
                                    s.ingest_strokes(pts, cx);
                                });
                                cx.notify();
                            });
                        }
                    })),
            )
            .child(
                div()
                    .id("draw-canvas")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .m_4()
                    .child(
                        div()
                            .id("draw-pad")
                            .size_full()
                            .relative()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(theme::BORDER))
                            .overflow_hidden()
                            .cursor(cursor)
                            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                                this.board.set_pad_hovered(*hovered);
                                sync_eraser_os_cursor(&this.board, window, cx);
                                cx.notify();
                            }))
                            .child(
                                canvas(
                                    move |_, _, _| {},
                                    move |bounds, _, window, _| {
                                        paint_paper(bounds, paper, window);
                                        for points in &lines {
                                            if points.len() < 2 {
                                                continue;
                                            }
                                            let mut builder = PathBuilder::stroke(px(INK_WIDTH));
                                            for (i, p) in points.iter().enumerate() {
                                                if i == 0 {
                                                    builder.move_to(*p);
                                                } else {
                                                    builder.line_to(*p);
                                                }
                                            }
                                            if let Ok(path) = builder.build() {
                                                window.paint_path(path, rgb(theme::TEXT));
                                            }
                                        }
                                        if let Some(pos) = eraser_at {
                                            paint_eraser_ring(pos, window);
                                        }
                                    },
                                )
                                .size_full(),
                            )
                            .child(draw_dock(tool, paper, can_undo, can_redo, cx))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                    this.board.pointer_down(ev.position);
                                    sync_eraser_os_cursor(&this.board, window, cx);
                                    cx.notify();
                                }),
                            )
                            .on_mouse_move(cx.listener(
                                |this, ev: &gpui::MouseMoveEvent, window, cx| {
                                    if this.board.pointer_move(ev.position) {
                                        sync_eraser_os_cursor(&this.board, window, cx);
                                        cx.notify();
                                    }
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.board.pointer_up();
                                    cx.notify();
                                }),
                            ),
                    ),
            )
    }
}

fn paint_paper(bounds: Bounds<Pixels>, paper: DrawPaper, window: &mut gpui::Window) {
    window.paint_quad(fill(bounds, rgb(theme::PAPER)));
    let ox: f32 = bounds.origin.x.into();
    let oy: f32 = bounds.origin.y.into();
    let right: f32 = ox + f32::from(bounds.size.width);
    let bottom: f32 = oy + f32::from(bounds.size.height);
    match paper {
        DrawPaper::Dots => {
            let mut x = ox + 10.0;
            while x < right {
                let mut y = oy + 10.0;
                while y < bottom {
                    window.paint_quad(fill(
                        Bounds::from_corners(point(px(x), px(y)), point(px(x + 2.0), px(y + 2.0))),
                        rgb(theme::PAPER_DOT),
                    ));
                    y += PAPER_PITCH;
                }
                x += PAPER_PITCH;
            }
        }
        DrawPaper::Lines => {
            let mut y = oy + 27.0;
            while y < bottom {
                window.paint_quad(fill(
                    Bounds::from_corners(point(px(ox), px(y)), point(px(right), px(y + 1.0))),
                    rgb(theme::PAPER_RULE),
                ));
                y += PAPER_RULE_GAP;
            }
        }
        DrawPaper::Blank => {}
    }
}

fn sync_eraser_os_cursor(
    board: &crate::ui::draw::DrawBoard,
    window: &mut gpui::Window,
    cx: &mut Context<MainWindow>,
) {
    crate::desktop::set_os_cursor_visible(!board.os_cursor_hidden());
    if board.os_cursor_hidden() {
        cx.on_next_frame(window, |_, _, _| {
            crate::desktop::reassert_hidden_os_cursor();
        });
    }
}

fn paint_eraser_ring(pos: Point<Pixels>, window: &mut gpui::Window) {
    let r = px(ERASER_RADIUS);
    let radii = point(r, r);
    let left = point(pos.x - r, pos.y);
    let right = point(pos.x + r, pos.y);
    let mut builder = PathBuilder::stroke(px(1.6));
    builder.move_to(left);
    builder.arc_to(radii, px(0.), false, true, right);
    builder.arc_to(radii, px(0.), false, true, left);
    if let Ok(path) = builder.build() {
        window.paint_path(path, rgb(theme::TEXT));
    }
}

fn draw_dock(
    tool: DrawTool,
    paper: DrawPaper,
    can_undo: bool,
    can_redo: bool,
    cx: &mut Context<MainWindow>,
) -> impl IntoElement + use<> {
    div()
        .id("draw-dock")
        .absolute()
        .bottom(px(12.))
        .left(gpui::relative(0.5))
        .ml(px(-DOCK_W / 2.0))
        .w(px(DOCK_W))
        .h(px(40.))
        .px(px(4.))
        .flex()
        .items_center()
        .gap(px(2.))
        .rounded_md()
        .bg(rgb(theme::BG_RAISED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
            this.board.set_dock_hovered(*hovered);
            sync_eraser_os_cursor(&this.board, window, cx);
            cx.notify();
        }))
        .child(icon_btn_kbd(
            "draw-pen",
            IconKind::Draw,
            t("draw.pen"),
            '1',
            tool == DrawTool::Pen,
            true,
            {
                let entity = cx.entity();
                move |window, cx| {
                    entity.update(cx, |this, cx| {
                        this.board.set_tool(DrawTool::Pen);
                        sync_eraser_os_cursor(&this.board, window, cx);
                        cx.notify();
                    });
                }
            },
        ))
        .child(icon_btn_kbd(
            "draw-eraser",
            IconKind::Eraser,
            t("draw.eraser"),
            '2',
            tool == DrawTool::Eraser,
            true,
            {
                let entity = cx.entity();
                move |window, cx| {
                    entity.update(cx, |this, cx| {
                        this.board.set_tool(DrawTool::Eraser);
                        sync_eraser_os_cursor(&this.board, window, cx);
                        cx.notify();
                    });
                }
            },
        ))
        .child(div().w(px(1.)).h(px(16.)).mx_1().bg(rgb(theme::TRACK_OFF)))
        .child(icon_btn_kbd(
            "draw-undo",
            IconKind::Undo,
            t("draw.undo"),
            '3',
            false,
            can_undo,
            {
                let entity = cx.entity();
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.board.undo();
                        cx.notify();
                    });
                }
            },
        ))
        .child(icon_btn_kbd(
            "draw-redo",
            IconKind::Redo,
            t("draw.redo"),
            '4',
            false,
            can_redo,
            {
                let entity = cx.entity();
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.board.redo();
                        cx.notify();
                    });
                }
            },
        ))
        .child(div().w(px(148.)).ml(px(4.)).child(segmented([
            seg_item(
                "draw-paper-dots",
                t("draw.dots"),
                paper == DrawPaper::Dots,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.board.set_paper(DrawPaper::Dots);
                            cx.notify();
                        });
                    }
                },
            ),
            seg_item(
                "draw-paper-lines",
                t("draw.lines"),
                paper == DrawPaper::Lines,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.board.set_paper(DrawPaper::Lines);
                            cx.notify();
                        });
                    }
                },
            ),
            seg_item(
                "draw-paper-blank",
                t("draw.blank"),
                paper == DrawPaper::Blank,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.board.set_paper(DrawPaper::Blank);
                            cx.notify();
                        });
                    }
                },
            ),
        ])))
}
