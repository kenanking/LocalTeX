use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    canvas, div, img, prelude::*, px, rgb, App, Context, CursorStyle, Entity, FocusHandle,
    Focusable, Image, ImageFormat, MouseButton, MouseDownEvent, PathBuilder, Pixels, Point,
    RenderImage, SharedString, Window,
};
use uuid::Uuid;

use super::theme;
use super::widgets::{
    app_icon_image, btn, ghost_btn, icon_btn, kbd_chip, section_label, segmented, status_dot,
    IconKind,
};
use crate::actions::{
    Capture, CloseSheet, CopyExport, DeleteSelected, OpenSettings, QuitApp, RetryOcr, SelectNext,
    SelectPrev, StartDraw, ToggleFormat, UploadImage,
};
use crate::doc::{BlockKind, DocStatus, ExportFmt};
use crate::imgutil;
use crate::ocr::EngineStatus;
use crate::preview;
use crate::state::AppState;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sheet {
    None,
    Draw,
    Settings,
}

struct DrawBoard {
    lines: Vec<Vec<Point<Pixels>>>,
    painting: bool,
}

impl DrawBoard {
    fn has_ink(&self) -> bool {
        self.lines.iter().any(|line| line.len() >= 2)
    }
}

pub struct MainWindow {
    state: Entity<AppState>,
    focus: FocusHandle,
    thumbs: HashMap<Uuid, Arc<RenderImage>>,
    fulls: HashMap<Uuid, Arc<RenderImage>>,
    previews: HashMap<Uuid, (String, Option<Arc<Image>>)>,
    sheet: Sheet,
    board: DrawBoard,
    toolbar_hint: Option<SharedString>,
}

impl MainWindow {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let state_for_close = state.clone();
        window.on_window_should_close(cx, move |window, cx| {
            let action = state_for_close.read(cx).prefs.close_action;
            AppState::handle_main_close(action, window, cx)
        });
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self {
            state,
            focus,
            thumbs: HashMap::new(),
            fulls: HashMap::new(),
            previews: HashMap::new(),
            sheet: Sheet::None,
            board: DrawBoard {
                lines: Vec::new(),
                painting: false,
            },
            toolbar_hint: None,
        }
    }

    fn upload(&mut self, _: &UploadImage, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
        self.state.update(cx, |state, cx| state.request_upload(cx));
    }

    fn start_draw(&mut self, _: &StartDraw, _: &mut Window, cx: &mut Context<Self>) {
        self.sheet = Sheet::Draw;
        self.board = DrawBoard {
            lines: Vec::new(),
            painting: false,
        };
        cx.notify();
    }

    fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.sheet = if self.sheet == Sheet::Settings {
            Sheet::None
        } else {
            Sheet::Settings
        };
        cx.notify();
    }

    fn close_sheet(&mut self, _: &CloseSheet, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
    }

    fn capture(&mut self, _: &Capture, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
        self.state.update(cx, |state, cx| state.request_capture(cx));
    }

    pub fn dismiss_sheet(&mut self, cx: &mut Context<Self>) {
        if self.sheet != Sheet::None {
            self.sheet = Sheet::None;
            cx.notify();
        }
    }

    fn copy(&mut self, _: &CopyExport, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.copy_selected(cx));
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.select_delta(1, cx));
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.select_delta(-1, cx));
    }

    fn delete_selected(&mut self, _: &DeleteSelected, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.delete_selected(cx));
    }

    fn toggle_format(&mut self, _: &ToggleFormat, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.toggle_format(cx));
    }

    fn retry(&mut self, _: &RetryOcr, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.retry_selected(cx));
    }

    fn quit(&mut self, _: &QuitApp, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn ensure_images(&mut self, state: &AppState) {
        let live: HashSet<Uuid> = state.documents.iter().map(|d| d.id).collect();
        self.thumbs.retain(|id, _| live.contains(id));
        self.fulls.retain(|id, _| live.contains(id));
        self.previews.retain(|id, _| live.contains(id));
        for doc in &state.documents {
            self.thumbs.entry(doc.id).or_insert_with(|| {
                let thumb = imgutil::thumbnail(&doc.image, 56, 40);
                imgutil::rgba_to_render(&thumb)
            });
            self.fulls
                .entry(doc.id)
                .or_insert_with(|| imgutil::rgba_to_render(&doc.image));
        }
    }
}

impl Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl gpui::Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status_kind, status_label, capturing, has_docs, n_docs) = {
            let state = self.state.read(cx);
            self.ensure_images(&state);
            let (status_kind, status_label) = chrome(&state);
            (
                status_kind,
                status_label,
                state.is_capturing(),
                !state.documents.is_empty(),
                state.documents.len(),
            )
        };
        let history = self.render_history(n_docs, cx);
        let detail = self.render_detail(capturing, cx);
        let sheet = self.sheet;
        let has_selected = {
            let state = self.state.read(cx);
            state.selected.is_some()
        };

        div()
            .id("main")
            .track_focus(&self.focus)
            .key_context(crate::identity::APP_SLUG)
            .on_action(cx.listener(Self::capture))
            .on_action(cx.listener(Self::upload))
            .on_action(cx.listener(Self::start_draw))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::close_sheet))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::delete_selected))
            .on_action(cx.listener(Self::toggle_format))
            .on_action(cx.listener(Self::retry))
            .on_action(cx.listener(Self::quit))
            .cursor(CursorStyle::Arrow)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::BG))
            .text_color(rgb(theme::TEXT))
            .child(self.render_topbar(capturing, has_selected, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(sheet == Sheet::None, |d| {
                        d.when(has_docs, |d| d.child(history)).child(detail)
                    })
                    .when(sheet == Sheet::Draw, |d| d.child(self.render_draw(cx)))
                    .when(sheet == Sheet::Settings, |d| {
                        d.child(super::settings::page(self.state.clone(), cx))
                    }),
            )
            .child(self.render_footer(status_kind, status_label))
    }
}

impl MainWindow {
    fn render_topbar(
        &self,
        capturing: bool,
        has_selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.clone();
        let hint = self
            .toolbar_hint
            .clone()
            .unwrap_or_else(|| crate::identity::APP_NAME.into());
        let sheet = self.sheet;

        div()
            .flex()
            .items_center()
            .h(px(44.))
            .px_3()
            .gap_1()
            .bg(rgb(theme::BG_RAISED))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(self.tool_btn(
                "tool-snip",
                IconKind::Snip,
                "Create snip from screenshot  Ctrl+Shift+S",
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
                cx,
            ))
            .child(self.tool_btn(
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
                cx,
            ))
            .child(self.tool_btn(
                "tool-draw",
                IconKind::Draw,
                "Create snip from drawing  Ctrl+D",
                sheet == Sheet::Draw,
                !capturing,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.sheet = Sheet::Draw;
                            this.board = DrawBoard {
                                lines: Vec::new(),
                                painting: false,
                            };
                            cx.notify();
                        });
                    }
                },
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .px_3()
                    .text_sm()
                    .text_color(rgb(theme::MUTED))
                    .child(hint),
            )
            .child(self.tool_btn(
                "tool-delete",
                IconKind::Delete,
                "Delete snip  Delete",
                false,
                has_selected && sheet == Sheet::None,
                {
                    let state = state.clone();
                    move |_, cx| {
                        state.update(cx, |s, cx| s.delete_selected(cx));
                    }
                },
                cx,
            ))
            .child(self.tool_btn(
                "tool-settings",
                IconKind::Settings,
                "Settings  Ctrl+,",
                sheet == Sheet::Settings,
                true,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.sheet = if this.sheet == Sheet::Settings {
                                Sheet::None
                            } else {
                                Sheet::Settings
                            };
                            cx.notify();
                        });
                    }
                },
                cx,
            ))
    }

    fn tool_btn(
        &self,
        id: &'static str,
        kind: IconKind,
        hint: &'static str,
        active: bool,
        enabled: bool,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        icon_btn(
            id,
            kind,
            active,
            enabled,
            on_click,
            move |hovered, _, cx| {
                entity.update(cx, |this, cx| {
                    this.toolbar_hint = hovered.then(|| SharedString::from(hint));
                    cx.notify();
                });
            },
        )
    }

    fn render_footer(
        &self,
        status_kind: theme::StatusKind,
        status_label: String,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .h(px(28.))
            .px_4()
            .gap_2()
            .bg(rgb(theme::BG))
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .child(status_dot(status_kind))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child(SharedString::from(status_label)),
            )
    }

    fn render_draw(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
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
                                this.sheet = Sheet::None;
                                cx.notify();
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
                                if let Some(img) = imgutil::rasterize_strokes(&pts, 3) {
                                    this.sheet = Sheet::None;
                                    this.state.update(cx, |s, cx| s.ingest_image(img, cx));
                                }
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

    fn render_history(&self, n_docs: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let mut list = div()
            .id("history")
            .w(px(196.))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG))
            .border_r_1()
            .border_color(rgb(theme::BORDER));

        list = list.child(
            div()
                .flex()
                .items_center()
                .h(px(36.))
                .px_3()
                .child(section_label("Snips"))
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child(SharedString::from(n_docs.to_string())),
                ),
        );

        let mut rows = div()
            .id("history-rows")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .pb_2()
            .overflow_y_scroll();

        for doc in &state.documents {
            let id = doc.id;
            let selected = state.selected == Some(id);
            let thumb = self.thumbs.get(&id).cloned();
            let title = doc.first_line();
            let age = doc.age_label();
            let state_ent = self.state.clone();

            rows = rows.child(
                div()
                    .id(SharedString::from(id.to_string()))
                    .flex()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme::accent_soft()))
                    .when(!selected, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        state_ent.update(cx, |s, cx| s.select(id, cx));
                    })
                    .child(
                        div()
                            .w(px(56.))
                            .h(px(40.))
                            .rounded_sm()
                            .bg(rgb(theme::BG_SUNKEN))
                            .border_1()
                            .border_color(rgb(theme::BORDER))
                            .overflow_hidden()
                            .when_some(thumb, |d, img_data| {
                                d.child(
                                    img(img_data)
                                        .w(px(56.))
                                        .h(px(40.))
                                        .object_fit(gpui::ObjectFit::Cover),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_ellipsis()
                                    .text_color(rgb(theme::TEXT))
                                    .child(SharedString::from(title)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme::MUTED))
                                    .child(SharedString::from(age)),
                            ),
                    ),
            );
        }
        list.child(rows)
    }

    fn render_detail(&mut self, capturing: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let (doc, fmt, prefs) = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                return empty_state(self.state.clone(), capturing);
            };
            (doc.clone(), state.export_fmt(), state.prefs.clone())
        };

        let source = doc.export(fmt, &prefs);
        let full = self.fulls.get(&doc.id).cloned();
        let failed = matches!(doc.status, DocStatus::Failed(_));
        let recognizing = matches!(doc.status, DocStatus::Recognizing);
        let can_copy = matches!(doc.status, DocStatus::Ready) && !source.is_empty();
        let show_orig = prefs.show_original || recognizing;
        let orig_label = if show_orig {
            "Hide original"
        } else {
            "Show original"
        };
        let retry_state = self.state.clone();
        let fmt_state = self.state.clone();
        let copy_state = self.state.clone();
        let age = doc.age_label();
        let preview = self.preview_element(&doc);
        let source_bar = if source.is_empty() {
            match doc.status {
                DocStatus::Recognizing => "Recognizing…".into(),
                _ => String::new(),
            }
        } else {
            source.split_whitespace().collect::<Vec<_>>().join(" ")
        };

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG_RAISED))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(36.))
                    .px_4()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::MUTED))
                            .child(SharedString::from(age)),
                    )
                    .child(div().flex_1())
                    .child(ghost_btn("orig", orig_label, !recognizing, {
                        let persist = self.state.clone();
                        move |_, cx| {
                            persist.update(cx, |s, cx| {
                                s.update_prefs(cx, |p| p.show_original = !p.show_original);
                            });
                        }
                    })),
            )
            .when(show_orig, |d| {
                d.child(
                    div().px_4().child(
                        div()
                            .w_full()
                            .h(px(96.))
                            .rounded_md()
                            .bg(rgb(theme::BG_SUNKEN))
                            .border_1()
                            .border_color(rgb(theme::BORDER))
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(full, |d, img_data| {
                                d.child(
                                    img(img_data)
                                        .w_full()
                                        .h(px(96.))
                                        .object_fit(gpui::ObjectFit::Contain),
                                )
                            }),
                    ),
                )
            })
            .when(failed, |d| {
                d.child(
                    div()
                        .mx_4()
                        .mt_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(theme::danger_soft())
                        .text_color(rgb(theme::DANGER))
                        .child(div().flex_1().text_xs().child(doc.first_line()))
                        .child(btn("retry", "Retry", true, true, move |_, cx| {
                            retry_state.update(cx, |s, cx| s.retry_selected(cx));
                        })),
                )
            })
            .child(
                div()
                    .id("preview")
                    .flex_1()
                    .min_h_0()
                    .m_4()
                    .p_4()
                    .rounded_md()
                    .bg(rgb(theme::BG))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(preview),
            )
            .child(
                div()
                    .px_4()
                    .pb_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(segmented(
                                "fmt-md",
                                ExportFmt::Markdown.label(),
                                fmt == ExportFmt::Markdown,
                                {
                                    let state = fmt_state.clone();
                                    move |_, cx| {
                                        state.update(cx, |s, cx| {
                                            s.set_format(ExportFmt::Markdown, cx);
                                        });
                                    }
                                },
                                "fmt-tex",
                                ExportFmt::Latex.label(),
                                fmt == ExportFmt::Latex,
                                {
                                    move |_, cx| {
                                        fmt_state.update(cx, |s, cx| {
                                            s.set_format(ExportFmt::Latex, cx);
                                        });
                                    }
                                },
                            ))
                            .child(div().flex_1()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(36.))
                            .pl_3()
                            .pr_1()
                            .rounded_md()
                            .bg(theme::accent_soft())
                            .child(
                                div()
                                    .id("copy-bar")
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .font_family("monospace")
                                    .text_sm()
                                    .text_ellipsis()
                                    .text_color(rgb(theme::TEXT))
                                    .child(SharedString::from(source_bar)),
                            )
                            .child(btn("copy", "Copy", true, can_copy, move |_, cx| {
                                copy_state.update(cx, |s, cx| s.copy_selected(cx));
                            })),
                    ),
            )
    }

    fn preview_element(&mut self, doc: &crate::doc::Document) -> impl IntoElement {
        let key = preview_key(doc);
        let cached = self
            .previews
            .get(&doc.id)
            .filter(|(k, _)| k == &key)
            .map(|(_, image)| image.clone());
        let image = if let Some(cached) = cached {
            cached
        } else {
            let image = match preview::document_preview_svg(&doc.blocks) {
                Ok(Some(svg)) => Some(Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    svg.into_bytes(),
                ))),
                Ok(None) => None,
                Err(err) => {
                    eprintln!(
                        "{}: formula preview failed ({err})",
                        crate::identity::APP_SLUG
                    );
                    None
                }
            };
            self.previews.insert(doc.id, (key, image.clone()));
            image
        };
        if let Some(image) = image {
            div()
                .id("preview-svg")
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(image)
                        .max_w_full()
                        .max_h_full()
                        .object_fit(gpui::ObjectFit::Contain),
                )
        } else {
            let plain = doc
                .blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            div()
                .id("preview-plain")
                .size_full()
                .text_sm()
                .text_color(rgb(theme::TEXT))
                .overflow_y_scroll()
                .child(SharedString::from(if plain.is_empty() {
                    match doc.status {
                        DocStatus::Recognizing => "Recognizing…".into(),
                        DocStatus::Ready => "No preview yet.".into(),
                        DocStatus::Failed(_) => String::new(),
                    }
                } else {
                    plain
                }))
        }
    }
}

fn chrome(state: &AppState) -> (theme::StatusKind, String) {
    if state.is_capturing() {
        return (theme::StatusKind::Busy, "Capturing…".into());
    }
    if let Some(err) = state.capture_error() {
        return (theme::StatusKind::Error, err.to_string());
    }
    if let Some(doc) = state.selected_doc() {
        if let DocStatus::Failed(err) = &doc.status {
            return (theme::StatusKind::Error, err.clone());
        }
    }
    match state.engine_status() {
        status @ EngineStatus::MissingModels { .. } => (theme::StatusKind::Idle, status.label()),
        EngineStatus::Ready => (theme::StatusKind::Ready, "Ready".into()),
    }
}

fn preview_key(doc: &crate::doc::Document) -> String {
    doc.blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Formula)
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn empty_state(state: Entity<AppState>, capturing: bool) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .bg(rgb(theme::BG_RAISED))
        .child(
            img(app_icon_image())
                .size(px(72.))
                .rounded_xl()
                .object_fit(gpui::ObjectFit::Contain),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(theme::TEXT))
                .child("Snip the screen"),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child("Capture text or formulas, then copy as Markdown or LaTeX."),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .mt_2()
                .child(btn("empty-snip", "Snip", true, !capturing, move |_, cx| {
                    state.update(cx, |s, cx| s.request_capture(cx));
                }))
                .child(kbd_chip("Ctrl+Shift+S")),
        )
}
