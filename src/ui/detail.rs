use gpui::{
    div, img, prelude::*, px, rgb, Context, CursorStyle, Entity, MouseButton, MouseDownEvent,
    SharedString,
};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::orig_view::{
    clamp_strip_h, copy_reserve, max_strip_h, orig_hud_disc, source_done_disc, source_hud_bar,
    source_hud_disc, source_hud_sep, OrigStripFrame,
};
use super::scroll::{
    overlay_chrome_hovered, overlay_pointer_in_pane, overlay_scrollbar, ScrollAxis, ScrollbarTone,
};
use super::theme;
use super::widgets::{btn, copy_chip, kbd_chip, ocr_meta_bar, section_label, IconKind};
use super::window_drag::WindowDrag;
use crate::doc::{DocStatus, ImageSlot, OcrMeta};
use crate::export::CopyKind;
use crate::keymap::{self, ShortcutId};
use crate::preview::DerivedCopyRow;
use crate::state::AppState;

const PREVIEW_BORDER: f32 = 1.0;
const PREVIEW_SCROLL_PAD_X: f32 = 16.0;
const FRAME_GUTTER_X: f32 = 16.0;
const SOURCE_PANEL_MIN_W: f32 = 140.0;
const SOURCE_SPLITTER_W: f32 = 7.0;

impl MainWindow {
    pub(crate) fn render_detail(
        &mut self,
        capturing: bool,
        copy_pane_w: f32,
        workspace_h: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if self.orig_strip.bind_doc(self.state.read(cx).selected())
            && self.window_drag.as_ref().is_some_and(|d| d.is_strip())
        {
            self.window_drag = None;
        }
        let snap = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                let capture_chord = keymap::effective(&state.prefs.shortcuts, ShortcutId::Capture)
                    .map(|c| keymap::chips(&c).join("+"));
                return empty_state(self.state.clone(), capturing, capture_chord);
            };
            DetailSnap {
                id: doc.id,
                status: doc.status.clone(),
                first_line: doc.first_line(),
                can_retry: doc.can_retry(),
                ocr: doc.ocr,
                image_missing: matches!(doc.image, ImageSlot::Missing),
                show_original: state.prefs.show_original,
                orig_strip_h: state.prefs.orig_strip_h,
                reading_cap: state.prefs.reading_width.cap_px(),
                revision: doc.revision,
                ready: matches!(doc.status, DocStatus::Ready),
            }
        };

        let failed = matches!(snap.status, DocStatus::Failed(_));
        let ready = snap.ready;
        let copy_rows = self
            .media
            .derived
            .as_ref()
            .filter(|d| d.id == snap.id && d.revision == snap.revision)
            .map(|d| d.copy_rows.clone())
            .unwrap_or_default();
        let retry_state = self.state.clone();
        let full = self.full(snap.id);
        let doc_id = snap.id;
        self.preview.reset_for(doc_id);
        let source_open = self.source_panel.open;
        let preview_w = preview_column_width(
            copy_pane_w,
            source_open,
            self.source_panel.split,
            snap.reading_cap,
        );
        let preview = self.preview_element(doc_id, &snap.status, preview_w, cx);
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
        let lang = self.source_panel.lang;
        let edited = self
            .state
            .read(cx)
            .selected_doc()
            .is_some_and(|d| d.is_edited());
        let can_undo = self.source_panel.editor.read(cx).can_undo();
        let can_redo = self.source_panel.editor.read(cx).can_redo();
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
        let copy_h = copy_reserve(ready);
        let max_h = max_strip_h(workspace_h, copy_h);
        let strip_h = clamp_strip_h(snap.orig_strip_h, max_h);
        let orig = self.render_orig_strip(
            OrigStripFrame {
                doc_id,
                full,
                hovered: orig_hover,
                copied: orig_copied,
                reveal_enabled,
                missing: image_missing,
                img_w,
                img_h,
                pane_w: copy_pane_w,
                strip_h,
                max_h,
            },
            cx,
        );

        if ready && self.preview.bar_pending && self.preview.hscroll_bounds_ready() {
            self.preview.bar_pending = false;
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
                        .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                            let next = overlay_chrome_hovered(
                                *hovered,
                                overlay_pointer_in_pane(
                                    &this.preview.vscroll,
                                    ScrollAxis::Vertical,
                                    window.mouse_position(),
                                ),
                            );
                            if this.preview.hover != next {
                                this.preview.hover = next;
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
                                .flex()
                                .flex_col()
                                .items_center()
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
                        .when(ready && !source_open && self.preview.hover, |d| {
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
        let (cap, font) = {
            let prefs = &self.state.read(cx).prefs;
            (px(prefs.reading_width.cap_px()), prefs.content_font)
        };
        self.source_panel
            .editor
            .update(cx, |ed, cx| ed.set_content_font(font, cx));
        let src_w = (pane_w * self.source_panel.split).max(SOURCE_PANEL_MIN_W);
        div()
            .id("source-panel")
            .relative()
            .min_h_0()
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
                    .px_4()
                    .pt_3()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(cap)
                            .min_w_0()
                            .child(self.source_panel.editor.clone()),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(6.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .items_center()
                    .gap(px(6.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        source_hud_bar()
                            .child(
                                div()
                                    .px_1()
                                    .text_xs()
                                    .text_color(rgb(theme::MUTED))
                                    .child(lang),
                            )
                            .child(source_hud_sep())
                            .child(div().opacity(if edited { 1. } else { 0.38 }).child(
                                source_hud_disc(
                                    "src-revert",
                                    IconKind::Reset,
                                    "Revert OCR",
                                    edited,
                                    {
                                        let entity = entity.clone();
                                        move |_, window, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.revert_source(window, cx);
                                            });
                                        }
                                    },
                                ),
                            ))
                            .child(source_hud_sep())
                            .child(div().opacity(if can_undo { 1. } else { 0.38 }).child(
                                source_hud_disc("src-undo", IconKind::Undo, "Undo", can_undo, {
                                    let entity = entity.clone();
                                    move |_, window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.source_panel.editor.update(cx, |ed, cx| {
                                                ed.undo_click(window, cx);
                                            });
                                        });
                                    }
                                }),
                            ))
                            .child(div().opacity(if can_redo { 1. } else { 0.38 }).child(
                                source_hud_disc("src-redo", IconKind::Redo, "Redo", can_redo, {
                                    let entity = entity.clone();
                                    move |_, window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.source_panel.editor.update(cx, |ed, cx| {
                                                ed.redo_click(window, cx);
                                            });
                                        });
                                    }
                                }),
                            )),
                    )
                    .child(source_done_disc("src-hide", IconKind::Check, "Done", {
                        let entity = entity.clone();
                        move |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.set_source_open(false, window, cx);
                            });
                        }
                    })),
            )
    }

    fn render_source_split(&self, pane_w: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let start_split = self.source_panel.split;
        div()
            .id("source-split")
            .relative()
            .w(px(SOURCE_SPLITTER_W))
            .min_h_0()
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.window_drag = Some(WindowDrag::Source {
                        start_x: f32::from(ev.position.x),
                        start_pct: start_split,
                        work_w: pane_w.max(1.0),
                    });
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

    fn render_copy_rows(
        &self,
        doc_id: Uuid,
        rows: &[DerivedCopyRow],
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
                let text = row.payload.clone();
                let hint = if copied.is_some_and(|(_, k)| k == kind) {
                    SharedString::from("Copied")
                } else {
                    row.hint.clone()
                };
                let is_copied = copied.is_some_and(|(_, k)| k == kind);
                let entity = cx.entity();
                let state = self.state.clone();
                line = line.child(copy_chip(
                    SharedString::from(format!("copy-{}", kind.id())),
                    kind.label(),
                    kind.symbol(),
                    hint,
                    is_copied,
                    !text.is_empty(),
                    move |_, cx| {
                        state.update(cx, |s, cx| s.copy_chip_payload(kind, text.to_string(), cx));
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
    orig_strip_h: f32,
    reading_cap: f32,
    revision: u64,
    ready: bool,
}

fn preview_column_width(
    copy_pane_w: f32,
    source_open: bool,
    source_split: f32,
    reading_cap: f32,
) -> f32 {
    let frame_w = if source_open {
        let source_w = (copy_pane_w * source_split).max(SOURCE_PANEL_MIN_W);
        copy_pane_w - FRAME_GUTTER_X - source_w - SOURCE_SPLITTER_W - FRAME_GUTTER_X
    } else {
        copy_pane_w - 2.0 * FRAME_GUTTER_X
    };
    let chrome_w = 2.0 * (PREVIEW_BORDER + PREVIEW_SCROLL_PAD_X);
    (frame_w - chrome_w).min(reading_cap).max(1.0)
}

fn empty_state(
    state: Entity<AppState>,
    capturing: bool,
    capture_chord: Option<String>,
) -> gpui::Div {
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
                .children(capture_chord.map(kbd_chip)),
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

#[cfg(test)]
mod tests {
    use super::preview_column_width;

    #[test]
    fn preview_column_uses_current_frame_and_reading_cap() {
        assert_eq!(preview_column_width(500.0, false, 0.5, 720.0), 434.0);
        assert_eq!(preview_column_width(900.0, false, 0.5, 720.0), 720.0);
    }

    #[test]
    fn preview_column_accounts_for_source_split() {
        assert_eq!(preview_column_width(800.0, true, 0.5, 720.0), 327.0);
        assert_eq!(preview_column_width(300.0, true, 0.28, 720.0), 87.0);
    }

    #[test]
    fn preview_column_stays_positive_in_a_narrow_frame() {
        let width = preview_column_width(112.0, true, 0.72, 576.0);
        assert_eq!(width, 1.0);
        assert!(width.is_finite());
    }
}
