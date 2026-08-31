use std::sync::Arc;

use gpui::{
    div, img, prelude::*, px, rgb, rgba, Context, CursorStyle, Entity, MouseButton, MouseDownEvent,
    RenderImage, SharedString,
};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::orig_view::{fit_scale, max_strip_h, orig_action_capsule, orig_hud_disc};
use super::scroll::{overlay_scrollbar, ScrollAxis, ScrollbarTone};
use super::theme;
use super::widgets::{
    btn, copy_chip, kbd_chip, missing_image_slot, ocr_meta_bar, section_label, IconKind,
};
use crate::doc::{CopyKind, DocStatus, ImageSlot, OcrMeta, SnipKind};
use crate::state::AppState;

impl MainWindow {
    pub(crate) fn render_detail(
        &mut self,
        capturing: bool,
        copy_pane_w: f32,
        win_h: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.orig_strip.bind_doc(self.state.read(cx).selected());
        let snap = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                return empty_state(self.state.clone(), capturing);
            };
            DetailSnap {
                id: doc.id,
                status: doc.status.clone(),
                first_line: doc.first_line(),
                can_retry: doc.can_retry(),
                ocr: doc.ocr,
                image_missing: matches!(doc.image, ImageSlot::Missing),
                show_original: state.prefs.show_original,
                revision: doc.revision,
                ready: matches!(doc.status, DocStatus::Ready),
            }
        };

        let failed = matches!(snap.status, DocStatus::Failed(_));
        let ready = snap.ready;
        let copy_rows = self
            .derived
            .as_ref()
            .filter(|d| d.id == snap.id && d.revision == snap.revision)
            .map(|d| d.copy_rows.clone())
            .unwrap_or_default();
        let retry_state = self.state.clone();
        let full = self.full(snap.id);
        let doc_id = snap.id;
        self.preview.reset_for(doc_id);
        let preview = self.preview_element(doc_id, &snap.status, cx);
        let copied = self.copied.filter(|(id, _)| *id == doc_id);
        let orig_hover = self.orig.strip_hover;
        let image_missing = snap.image_missing;
        let can_retry = snap.can_retry
            && !self
                .state
                .read(cx)
                .selected_doc()
                .is_some_and(|d| d.is_edited());
        let ocr = snap.ocr;
        if self.source_open && !ready {
            self.close_source(cx);
        }
        if self.source_open {
            self.bind_source(cx);
        }
        let source_open = self.source_open;
        let edited = self
            .state
            .read(cx)
            .selected_doc()
            .is_some_and(|d| d.is_edited());
        let can_undo = self.source.read(cx).can_undo();
        let can_redo = self.source.read(cx).can_redo();
        let lang = {
            let prefs = self.state.read(cx).prefs.clone();
            let src = self.source.read(cx).text();
            let kind = crate::source::parse_source(&src, &prefs)
                .map(|b| crate::doc::snip_kind(&b))
                .unwrap_or_else(|_| {
                    self.state
                        .read(cx)
                        .selected_doc()
                        .map(|d| d.snip_kind())
                        .unwrap_or(SnipKind::Formula)
                });
            match kind {
                SnipKind::Formula | SnipKind::Table => "LaTeX",
                SnipKind::Mixed if src.contains("\\begin{tabular}") => "Markdown + tabular",
                SnipKind::Mixed => "Markdown",
            }
        };
        let (orig_copied, reveal_enabled) = {
            let state = self.state.read(cx);
            (
                state.orig_copy_flashed(doc_id),
                state.can_reveal_original(doc_id),
            )
        };
        let (img_w, img_h) = full
            .as_ref()
            .map(|im| {
                let s = im.size(0);
                (u32::from(s.width) as f32, u32::from(s.height) as f32)
            })
            .unwrap_or((0.0, 0.0));
        let copy_h = if ready { 72.0 } else { 0.0 };
        let max_h = max_strip_h(win_h, copy_h);
        let strip_h = self
            .orig_strip
            .displayed_h(img_w, img_h, copy_pane_w, max_h);
        let orig = self.render_orig_strip(
            doc_id,
            full,
            orig_hover,
            orig_copied,
            reveal_enabled,
            image_missing,
            img_w,
            img_h,
            copy_pane_w,
            strip_h,
            cx,
        );

        if ready && self.preview.bar_pending {
            if self.preview.hscroll_bounds_ready() {
                self.preview.bar_pending = false;
            } else {
                let entity = cx.entity();
                cx.defer(move |cx| {
                    entity.update(cx, |_, cx| cx.notify());
                });
            }
        }

        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG_RAISED))
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
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .child(snap.first_line.clone()),
                        )
                        .child(btn("retry", "Retry", true, can_retry, move |_, cx| {
                            retry_state.update(cx, |s, cx| s.retry_selected(cx));
                        })),
                )
            })
            .when(snap.show_original, |d| d.child(orig))
            .child({
                let entity = cx.entity();
                let preview_pane =
                    div()
                        .id("preview")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .when(!source_open, |d| d.mx_4())
                        .when(source_open, |d| d.mr_4())
                        .mt_3()
                        .mb_3()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .bg(rgb(theme::BG))
                        .overflow_hidden()
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if this.preview.hover != *hovered {
                                this.preview.hover = *hovered;
                                cx.notify();
                            }
                        }))
                        .child(
                            div()
                                .id("preview-scroll")
                                .size_full()
                                .px_4()
                                .py_3()
                                .overflow_y_scroll()
                                .track_scroll(&self.preview.vscroll)
                                .capture_any_mouse_down({
                                    let sel = self.preview.sel.clone();
                                    let view = cx.entity_id();
                                    move |event, _, cx| {
                                        if event.button != MouseButton::Left {
                                            return;
                                        }
                                        let mut state = sel.borrow_mut();
                                        if state.anchor.key.is_empty() {
                                            return;
                                        }
                                        state.clear();
                                        cx.notify(view);
                                    }
                                })
                                .on_scroll_wheel({
                                    let view = cx.entity_id();
                                    move |_, _, cx| {
                                        cx.notify(view);
                                    }
                                })
                                .child(preview),
                        )
                        .child(overlay_scrollbar(
                            "y-scroll-thumb",
                            ScrollAxis::Vertical,
                            &self.preview.vscroll,
                            &self.preview.thumb,
                            self.preview.hover
                                || !self.preview.hscroll_hover.borrow().is_empty()
                                || self
                                    .preview
                                    .thumb
                                    .borrow()
                                    .as_ref()
                                    .is_some_and(|d| d.vertical),
                            ScrollbarTone::Subtle,
                        ))
                        .when(ready && !source_open, |d| {
                            d.child(div().absolute().top(px(8.)).right(px(8.)).child(
                                orig_hud_disc("edit-source", IconKind::Draw, "Edit source", {
                                    let entity = entity.clone();
                                    move |_, window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.set_source_open(true, window, cx);
                                        });
                                    }
                                }),
                            ))
                        });
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when(source_open, |d| {
                        d.child(self.render_source_panel(
                            lang,
                            edited,
                            can_undo,
                            can_redo,
                            copy_pane_w,
                            cx,
                        ))
                        .child(self.render_source_split(copy_pane_w, cx))
                    })
                    .child(preview_pane)
            })
            .when(ready, |d| {
                d.child(self.render_copy_rows(doc_id, &copy_rows, copied, ocr, copy_pane_w, cx))
            })
    }

    fn render_source_panel(
        &self,
        lang: &'static str,
        edited: bool,
        can_undo: bool,
        can_redo: bool,
        pane_w: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let src_w = (pane_w * self.source_split).max(140.0);
        div()
            .id("source-panel")
            .relative()
            .h_full()
            .w(px(src_w))
            .min_w_0()
            .flex_none()
            .ml_4()
            .mt_3()
            .mb_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::BG))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("source-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .child(self.source.clone()),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(8.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .h(px(36.))
                            .px(px(8.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .bg(theme::hud_pill())
                            .border_1()
                            .border_color(rgba(0xffffff47))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(div().px_1().text_xs().text_color(rgb(0xc8c8d0)).child(lang))
                            .child(div().w(px(1.)).h(px(12.)).mx_1().bg(rgba(0xffffff38)))
                            .child(orig_hud_disc("src-hide", IconKind::Collapse, "Collapse", {
                                let entity = entity.clone();
                                move |_, window, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.set_source_open(false, window, cx);
                                    });
                                }
                            }))
                            .child(div().opacity(if edited { 1. } else { 0.38 }).child(
                                orig_hud_disc("src-revert", IconKind::Reset, "Revert OCR", {
                                    let entity = entity.clone();
                                    move |_, window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.revert_source(window, cx);
                                        });
                                    }
                                }),
                            ))
                            .child(div().w(px(1.)).h(px(12.)).mx_1().bg(rgba(0xffffff38)))
                            .child(div().opacity(if can_undo { 1. } else { 0.38 }).child(
                                orig_hud_disc("src-undo", IconKind::Undo, "Undo", {
                                    let entity = entity.clone();
                                    move |_, window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.source.update(cx, |ed, cx| {
                                                ed.undo_click(window, cx);
                                            });
                                        });
                                    }
                                }),
                            ))
                            .child(div().opacity(if can_redo { 1. } else { 0.38 }).child(
                                orig_hud_disc("src-redo", IconKind::Redo, "Redo", {
                                    let entity = entity.clone();
                                    move |_, window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.source.update(cx, |ed, cx| {
                                                ed.redo_click(window, cx);
                                            });
                                        });
                                    }
                                }),
                            )),
                    ),
            )
    }

    fn render_source_split(&self, pane_w: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let start_split = self.source_split;
        div()
            .id("source-split")
            .w(px(7.))
            .h_full()
            .mt_3()
            .mb_3()
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.source_split_drag =
                        Some((f32::from(ev.position.x), start_split, pane_w.max(1.0)));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(2.))
                    .top(px(12.))
                    .bottom(px(12.))
                    .w(px(3.))
                    .rounded(px(2.))
                    .bg(rgb(theme::TRACK_OFF)),
            )
    }

    fn render_orig_strip(
        &self,
        doc_id: Uuid,
        full: Option<Arc<RenderImage>>,
        hovered: bool,
        copied: bool,
        reveal_enabled: bool,
        missing: bool,
        img_w: f32,
        img_h: f32,
        pane_w: f32,
        strip_h: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let split_on = self.orig_strip.split_hover || self.orig_strip.drag.is_some();
        let card = if missing {
            missing_image_slot(px(720.), px(strip_h))
                .id("orig-thumb")
                .absolute()
                .inset_0()
                .w_full()
                .when(split_on, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .right(px(8.))
                            .bottom_0()
                            .h(px(2.))
                            .rounded(px(2.))
                            .bg(rgb(theme::ACCENT)),
                    )
                })
                .child(self.orig_strip_handle(strip_h, cx))
                .into_any()
        } else {
            let scale = fit_scale(img_w, img_h, pane_w, strip_h);
            let draw_w = (img_w * scale).max(1.0);
            let draw_h = (img_h * scale).max(1.0);
            let entity = cx.entity();
            div()
                .id("orig-thumb")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(rgb(theme::BG_SUNKEN))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .when_some(full.clone(), |d, img_data| {
                    d.child(
                        img(img_data)
                            .w(px(draw_w))
                            .h(px(draw_h))
                            .object_fit(gpui::ObjectFit::Fill),
                    )
                })
                .when(full.is_some(), |d| {
                    d.child(
                        div()
                            .id("orig-hit")
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom(px(12.))
                            .occlude()
                            .cursor_pointer()
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                if this.orig.strip_hover != *hovered {
                                    this.orig.strip_hover = *hovered;
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.zoom_original(doc_id, window, cx);
                            }))
                            .flex()
                            .items_start()
                            .justify_end()
                            .pt(px(8.))
                            .pr(px(8.))
                            .when(hovered, |d| {
                                d.child(
                                    div()
                                        .id("strip-actions")
                                        .h(px(28.))
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .child(orig_action_capsule(
                                            doc_id,
                                            copied,
                                            reveal_enabled,
                                            "strip",
                                            cx,
                                        ))
                                        .child(orig_hud_disc(
                                            "strip-zoom",
                                            IconKind::Corners,
                                            "Open original",
                                            {
                                                let entity = entity.clone();
                                                move |_, window, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.zoom_original(doc_id, window, cx);
                                                    });
                                                }
                                            },
                                        )),
                                )
                            }),
                    )
                })
                .when(split_on, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .right(px(8.))
                            .bottom_0()
                            .h(px(2.))
                            .rounded(px(2.))
                            .bg(rgb(theme::ACCENT)),
                    )
                })
                .child(self.orig_strip_handle(strip_h, cx))
                .into_any()
        };

        div().px_4().pt_3().flex_shrink_0().child(
            div()
                .id("orig-wrap")
                .relative()
                .w_full()
                .h(px(strip_h))
                .child(card),
        )
    }

    fn orig_strip_handle(&self, strip_h: f32, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("orig-strip-resize")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(12.))
            .occlude()
            .cursor(CursorStyle::ResizeUpDown)
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.orig_strip.split_hover != *hovered {
                    this.orig_strip.split_hover = *hovered;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    if ev.click_count >= 2 {
                        this.orig_strip.reset_auto();
                    } else {
                        this.orig_strip
                            .begin_drag(f32::from(ev.position.y), strip_h);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
    }

    fn render_copy_rows(
        &self,
        doc_id: Uuid,
        rows: &[crate::doc::CopyRow],
        copied: Option<(Uuid, CopyKind)>,
        ocr: Option<OcrMeta>,
        pane_w: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut col = div()
            .w_full()
            .px_4()
            .pb_3()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(section_label("Copy"));

        // GPUI will not grow an `.id()` node, so each wrap-line is a `w_full` flex
        // row and the chip *wrapper* is `flex_1` (fills the same inset as the preview).
        const CHIP_MIN: f32 = 112.;
        const CHIP_GAP: f32 = 6.;
        let n = rows.len();
        let cols = ((pane_w + CHIP_GAP) / (CHIP_MIN + CHIP_GAP))
            .floor()
            .clamp(1., n.max(1) as f32) as usize;
        let mut lines = div().w_full().flex().flex_col().gap(px(CHIP_GAP));
        for chunk in rows.chunks(cols.max(1)) {
            let mut line = div().flex().w_full().gap(px(CHIP_GAP));
            for row in chunk {
                let kind = row.kind;
                let text = row.text.clone();
                let hint = if copied.is_some_and(|(_, k)| k == kind) {
                    SharedString::from("Copied")
                } else if kind == CopyKind::MsWord {
                    SharedString::from("Paste as Word equation")
                } else {
                    SharedString::from(row.text.split_whitespace().collect::<Vec<_>>().join(" "))
                };
                let is_copied = copied.is_some_and(|(_, k)| k == kind);
                let entity = cx.entity();
                line = line.child(copy_chip(
                    SharedString::from(format!("copy-{}", kind.id())),
                    kind.label(),
                    kind.symbol(),
                    hint,
                    is_copied,
                    !text.is_empty(),
                    move |_, cx| {
                        crate::state::AppState::copy_text(text.clone(), cx);
                        entity.update(cx, |this, cx| {
                            this.flash_copied(doc_id, kind, cx);
                        });
                    },
                ));
            }
            lines = lines.child(line);
        }
        col = col.child(lines);
        if let Some(meta) = ocr {
            col = col.child(ocr_meta_bar(meta));
        }
        col
    }
}

struct DetailSnap {
    id: Uuid,
    status: DocStatus,
    first_line: String,
    can_retry: bool,
    ocr: Option<OcrMeta>,
    image_missing: bool,
    show_original: bool,
    revision: u64,
    ready: bool,
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
            img(crate::icon::app_tile_image())
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
                .child("Capture text, formulas, or tables — copy as Markdown or LaTeX."),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .mt_2()
                .child(btn("empty-snip", "Snip", true, !capturing, {
                    let state = state.clone();
                    move |_, cx| {
                        state.update(cx, |s, cx| s.request_capture(cx));
                    }
                }))
                .child(kbd_chip("Ctrl+Shift+S")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(btn(
                    "empty-paste",
                    "Paste",
                    false,
                    !capturing,
                    move |_, cx| {
                        state.update(cx, |s, cx| s.request_paste(cx));
                    },
                ))
                .child(kbd_chip("Ctrl+V")),
        )
}
