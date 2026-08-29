use std::sync::Arc;

use gpui::{
    div, img, prelude::*, px, rgb, rgba, svg, Context, Entity, MouseButton, RenderImage,
    SharedString,
};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::scroll::{overlay_scrollbar, ScrollAxis, ScrollbarTone};
use super::theme;
use super::widgets::{
    btn, copy_chip, kbd_chip, missing_image_slot, ocr_meta_bar, section_label, IconKind,
};
use crate::doc::{CopyKind, DocStatus, ImageSlot, OcrMeta};
use crate::state::AppState;

impl MainWindow {
    pub(crate) fn render_detail(
        &mut self,
        capturing: bool,
        copy_pane_w: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
        let can_retry = snap.can_retry;
        let ocr = snap.ocr;
        let orig = self.render_orig_strip(doc_id, full, orig_hover, image_missing, cx);

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
            .child(
                div()
                    .id("preview")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .mx_4()
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
                    )),
            )
            .when(ready, |d| {
                d.child(self.render_copy_rows(doc_id, &copy_rows, copied, ocr, copy_pane_w, cx))
            })
    }

    fn render_orig_strip(
        &self,
        doc_id: Uuid,
        full: Option<Arc<RenderImage>>,
        hovered: bool,
        missing: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if missing {
            return div()
                .px_4()
                .pt_3()
                .child(missing_image_slot(px(720.), px(96.)).w_full())
                .into_any();
        }
        div()
            .px_4()
            .pt_3()
            .child(
                div()
                    .id("orig-thumb")
                    .relative()
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
                    .when_some(full.clone(), |d, img_data| {
                        d.child(
                            img(img_data)
                                .w_full()
                                .h(px(96.))
                                .object_fit(gpui::ObjectFit::Contain),
                        )
                    })
                    .child(
                        div()
                            .id("orig-hit")
                            .absolute()
                            .top(px(0.))
                            .left(px(0.))
                            .w_full()
                            .h(px(96.))
                            .occlude()
                            .when(full.is_some(), |d| {
                                d.cursor_pointer()
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        if this.orig.strip_hover != *hovered {
                                            this.orig.strip_hover = *hovered;
                                            cx.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.zoom_original(doc_id, window, cx);
                                    }))
                            })
                            .when(hovered && full.is_some(), |d| {
                                d.child(
                                    div()
                                        .id("orig-zoom-btn")
                                        .absolute()
                                        .top(px(8.))
                                        .right(px(8.))
                                        .size(px(28.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(theme::hud_pill())
                                        .border_1()
                                        .border_color(rgba(0xffffff47))
                                        .cursor_pointer()
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click({
                                            let entity = cx.entity();
                                            move |_, window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.zoom_original(doc_id, window, cx);
                                                });
                                            }
                                        })
                                        .child(
                                            svg()
                                                .path(IconKind::Corners.asset_path())
                                                .size(px(14.))
                                                .text_color(rgb(0xffffff)),
                                        ),
                                )
                            }),
                    ),
            )
            .into_any()
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
