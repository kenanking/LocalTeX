use std::time::Instant;

use gpui::{
    canvas, div, prelude::*, px, rgb, Context, CursorStyle, MouseButton, MouseDownEvent,
    PathBuilder, Pixels, Point,
};

use super::main_window::{MainWindow, View};
use super::theme;
use super::widgets::{btn, ghost_btn, section_label};
use crate::ingest::IngestSource;

struct StrokePt {
    pos: Point<Pixels>,
    t_ms: f32,
}

pub(crate) struct DrawBoard {
    lines: Vec<Vec<StrokePt>>,
    painting: bool,
    t0: Option<Instant>,
}

impl DrawBoard {
    pub(crate) fn new() -> Self {
        Self {
            lines: Vec::new(),
            painting: false,
            t0: None,
        }
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

    fn traces(&self) -> Vec<Vec<[f32; 3]>> {
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

impl MainWindow {
    pub(crate) fn render_draw(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let lines: Vec<Vec<Point<Pixels>>> = self
            .board
            .lines
            .iter()
            .map(|line| line.iter().map(|p| p.pos).collect())
            .collect();
        let has_ink = self.board.has_ink();

        div()
            .id("draw")
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
                    .child(section_label("Draw a formula"))
                    .child(div().flex_1())
                    .child(ghost_btn("draw-clear", "Clear", has_ink, {
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.board = DrawBoard::new();
                                cx.notify();
                            });
                        }
                    }))
                    .child(ghost_btn("draw-cancel", "Cancel", true, {
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.dismiss_sheet(cx);
                            });
                        }
                    }))
                    .child(btn("draw-go", "Recognize", true, has_ink, {
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                let pts = this.board.traces();
                                this.board = DrawBoard::new();
                                this.view = View::Library;
                                this.state.update(cx, |s, cx| {
                                    s.ingest(IngestSource::Strokes(pts), cx);
                                });
                                cx.notify();
                            });
                        }
                    })),
            )
            .child(
                div()
                    .id("draw-canvas")
                    .flex_1()
                    .min_h_0()
                    .m_4()
                    .rounded_md()
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .cursor(CursorStyle::Arrow)
                    .child(
                        canvas(
                            move |_, _, _| {},
                            move |_, _, window, _| {
                                for points in &lines {
                                    if points.len() < 2 {
                                        continue;
                                    }
                                    let mut builder = PathBuilder::stroke(px(2.4));
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
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.board.painting = true;
                            let pt = this.board.stamp(ev.position);
                            this.board.lines.push(vec![pt]);
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _, cx| {
                        if !this.board.painting {
                            return;
                        }
                        let pt = this.board.stamp(ev.position);
                        if let Some(line) = this.board.lines.last_mut() {
                            line.push(pt);
                        }
                        cx.notify();
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.board.painting = false;
                            cx.notify();
                        }),
                    ),
            )
    }
}
