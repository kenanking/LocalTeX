use gpui::{
    canvas, div, prelude::*, px, rgb, Context, CursorStyle, MouseButton, MouseDownEvent,
    PathBuilder, Pixels, Point,
};

use super::main_window::{MainWindow, View};
use super::theme;
use super::widgets::{btn, ghost_btn, section_label};
use crate::ingest::IngestSource;

pub(crate) struct DrawBoard {
    pub(crate) lines: Vec<Vec<Point<Pixels>>>,
    pub(crate) painting: bool,
}

impl DrawBoard {
    pub(crate) fn new() -> Self {
        Self {
            lines: Vec::new(),
            painting: false,
        }
    }

    pub(crate) fn has_ink(&self) -> bool {
        self.lines.iter().any(|line| line.len() >= 2)
    }
}

impl MainWindow {
    pub(crate) fn render_draw(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.board.lines.clone();
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
                    .child(section_label("Draw a formula or note"))
                    .child(div().flex_1())
                    .child(ghost_btn("draw-clear", "Clear", has_ink, {
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.board.lines.clear();
                                this.board.painting = false;
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
                                let pts: Vec<Vec<(f32, f32)>> = this
                                    .board
                                    .lines
                                    .iter()
                                    .map(|line| {
                                        line.iter()
                                            .map(|p| (f32::from(p.x), f32::from(p.y)))
                                            .collect()
                                    })
                                    .collect();
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
                            this.board.lines.push(vec![ev.position]);
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _, cx| {
                        if !this.board.painting {
                            return;
                        }
                        if let Some(line) = this.board.lines.last_mut() {
                            line.push(ev.position);
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
