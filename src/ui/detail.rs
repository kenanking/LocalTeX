use std::sync::Arc;

use gpui::{
    div, img, point, prelude::*, px, rgb, Context, Entity, Image, ImageFormat, RenderImage,
    SharedString,
};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::theme;
use super::widgets::{
    btn, copy_row, icon_btn, kbd_chip, missing_image_slot, ocr_meta_bar, overlay_y_scrollbar,
    section_label, IconKind,
};
use crate::doc::{CopyKind, DocStatus, ImageSlot, OcrMeta};
use crate::preview::{self, InlineSeg, PreviewBlock, SvgMath};
use crate::state::AppState;

impl MainWindow {
    pub(crate) fn render_detail(
        &mut self,
        capturing: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (doc, prefs) = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                return empty_state(self.state.clone(), capturing);
            };
            (doc.clone(), state.prefs.clone())
        };

        let failed = matches!(doc.status, DocStatus::Failed(_));
        let ready = matches!(doc.status, DocStatus::Ready);
        let copy_rows = if ready {
            doc.copy_rows(&prefs)
        } else {
            Vec::new()
        };
        let retry_state = self.state.clone();
        let full = self.fulls.get(&doc.id).cloned();
        let preview = self.preview_element(&doc);
        let doc_id = doc.id;
        let copied = self.copied.filter(|(id, _)| *id == doc_id);
        let orig_hover = self.orig_hover;
        let image_missing = matches!(doc.image, ImageSlot::Missing);
        let can_retry = doc.can_retry();
        let ocr = doc.ocr;
        let orig = self.render_orig_strip(doc_id, full, orig_hover, image_missing, cx);

        if self.preview_scroll_doc != Some(doc_id) {
            self.preview_scroll.set_offset(point(px(0.), px(0.)));
            self.preview_scroll_doc = Some(doc_id);
            self.preview_bar_pending = true;
        }
        if ready && self.preview_bar_pending {
            self.preview_bar_pending = false;
            let entity = cx.entity();
            cx.defer(move |cx| {
                entity.update(cx, |_, cx| cx.notify());
            });
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
                        .child(div().flex_1().min_w_0().text_xs().child(doc.first_line()))
                        .child(btn("retry", "Retry", true, can_retry, move |_, cx| {
                            retry_state.update(cx, |s, cx| s.retry_selected(cx));
                        })),
                )
            })
            .when(prefs.show_original, |d| d.child(orig))
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
                    .child(
                        div()
                            .id("preview-scroll")
                            .size_full()
                            .px_4()
                            .py_3()
                            .overflow_y_scroll()
                            .track_scroll(&self.preview_scroll)
                            .on_scroll_wheel({
                                let entity = cx.entity();
                                move |_, _, cx| {
                                    entity.update(cx, |_, cx| cx.notify());
                                }
                            })
                            .child(preview),
                    )
                    .child(overlay_y_scrollbar(&self.preview_scroll)),
            )
            .when(ready && (!copy_rows.is_empty() || ocr.is_some()), |d| {
                d.child(self.render_copy_rows(doc_id, &copy_rows, copied, ocr, cx))
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
                                        if this.orig_hover != *hovered {
                                            this.orig_hover = *hovered;
                                            cx.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.zoom_original(doc_id, cx);
                                    }))
                            })
                            .when(hovered && full.is_some(), |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .top(px(8.))
                                        .right(px(8.))
                                        .rounded_md()
                                        .bg(rgb(theme::BG_RAISED))
                                        .border_1()
                                        .border_color(rgb(theme::BORDER))
                                        .child(icon_btn(
                                            "orig-zoom-btn",
                                            IconKind::Zoom,
                                            "Zoom original",
                                            false,
                                            true,
                                            {
                                                let entity = cx.entity();
                                                move |_, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.zoom_original(doc_id, cx);
                                                    });
                                                }
                                            },
                                        )),
                                )
                            }),
                    ),
            )
            .into_any()
    }

    pub(crate) fn render_orig_zoom(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let full = {
            let state = self.state.read(cx);
            state.selected.and_then(|id| self.fulls.get(&id).cloned())
        };
        div()
            .id("orig-zoom")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.unzoom();
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when_some(full, |d, img_data| {
                        d.child(
                            img(img_data)
                                .max_w_full()
                                .max_h_full()
                                .object_fit(gpui::ObjectFit::Contain),
                        )
                    }),
            )
            .child(
                div()
                    .pb_3()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .flex()
                    .justify_center()
                    .child("Click or Esc to close"),
            )
    }

    fn render_copy_rows(
        &self,
        doc_id: Uuid,
        rows: &[crate::doc::CopyRow],
        copied: Option<(Uuid, CopyKind)>,
        ocr: Option<OcrMeta>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut col = div()
            .px_4()
            .pb_3()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(section_label("Copy"));

        for (i, row) in rows.iter().enumerate() {
            let kind = row.kind;
            let text = row.text.clone();
            let preview = row.text.split_whitespace().collect::<Vec<_>>().join(" ");
            let is_copied = copied.is_some_and(|(_, k)| k == kind);
            let entity = cx.entity();
            col = col.child(copy_row(
                SharedString::from(format!("copy-{i}")),
                kind.label(),
                SharedString::from(preview),
                is_copied,
                !text.is_empty(),
                move |_, cx| {
                    crate::state::AppState::copy_text(text.clone(), cx);
                    entity.update(cx, |this, cx| {
                        this.copied = Some((doc_id, kind));
                        cx.notify();
                    });
                },
            ));
        }
        if let Some(meta) = ocr {
            col = col.child(ocr_meta_bar(meta));
        }
        col
    }

    fn preview_element(&mut self, doc: &crate::doc::Document) -> impl IntoElement {
        let key = preview_key(doc);
        let blocks = if let Some((k, blocks)) = self.previews.get(&doc.id) {
            if k == &key {
                blocks.clone()
            } else {
                let blocks = preview::document_preview(&doc.blocks);
                self.previews.insert(doc.id, (key, blocks.clone()));
                blocks
            }
        } else {
            let blocks = preview::document_preview(&doc.blocks);
            self.previews.insert(doc.id, (key, blocks.clone()));
            blocks
        };

        if blocks.is_empty() {
            let msg = match doc.status {
                DocStatus::Recognizing => "Recognizing…",
                DocStatus::Ready => "No preview yet.",
                DocStatus::Failed(_) => "",
            };
            return div()
                .id("preview-empty")
                .w_full()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child(SharedString::from(msg));
        }

        let mut col = div()
            .id("preview-doc")
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .items_start()
            .gap_3();

        for (i, block) in blocks.iter().enumerate() {
            col = col.child(self.render_preview_block(i, block));
        }
        col
    }

    fn render_preview_block(&mut self, i: usize, block: &PreviewBlock) -> impl IntoElement {
        match block {
            PreviewBlock::Paragraph(segs) => {
                let has_math = segs.iter().any(|s| matches!(s, InlineSeg::Math(_)));
                if !has_math {
                    let text = segs
                        .iter()
                        .map(|s| match s {
                            InlineSeg::Text(t) => t.as_str(),
                            InlineSeg::Math(_) => "",
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    div()
                        .id(SharedString::from(format!("p-{i}")))
                        .w_full()
                        .text_sm()
                        .line_height(px(22.))
                        .text_color(rgb(theme::TEXT))
                        .whitespace_normal()
                        .child(SharedString::from(text))
                        .into_any()
                } else {
                    let mut row = div()
                        .id(SharedString::from(format!("p-{i}")))
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(4.))
                        .text_sm()
                        .line_height(px(22.))
                        .text_color(rgb(theme::TEXT));
                    for (j, seg) in segs.iter().enumerate() {
                        match seg {
                            InlineSeg::Text(t) => {
                                for (k, tok) in wrap_text_tokens(t).into_iter().enumerate() {
                                    row = row.child(
                                        div()
                                            .id(SharedString::from(format!("t-{i}-{j}-{k}")))
                                            .text_sm()
                                            .line_height(px(22.))
                                            .text_color(rgb(theme::TEXT))
                                            .child(SharedString::from(tok)),
                                    );
                                }
                            }
                            InlineSeg::Math(math) => {
                                row = row.child(
                                    div()
                                        .id(SharedString::from(format!("m-{i}-{j}")))
                                        .flex()
                                        .items_center()
                                        .flex_shrink_0()
                                        .child(self.math_img(math)),
                                );
                            }
                        }
                    }
                    row.into_any()
                }
            }
            PreviewBlock::Display(math) => div()
                .id(SharedString::from(format!("d-{i}")))
                .w_full()
                .min_w_0()
                .py_2()
                .overflow_x_scroll()
                .flex()
                .justify_center()
                .child(self.math_img(math))
                .into_any(),
            PreviewBlock::Table(table) => div()
                .id(SharedString::from(format!("tbl-wrap-{i}")))
                .w_full()
                .child(self.render_table_preview(i, table))
                .into_any(),
            PreviewBlock::Fallback(text) => div()
                .id(SharedString::from(format!("f-{i}")))
                .w_full()
                .min_w_0()
                .text_sm()
                .whitespace_normal()
                .text_color(rgb(theme::TEXT))
                .child(SharedString::from(text.clone()))
                .into_any(),
        }
    }

    fn render_table_preview(&mut self, i: usize, table: &crate::table::Table) -> impl IntoElement {
        let grid = table.grid();
        let mut wrap = div()
            .id(SharedString::from(format!("tbl-{i}")))
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded_md()
            .overflow_x_scroll();
        for (r, row) in grid.iter().enumerate() {
            let mut line = div().flex().flex_row();
            for (c, cell) in row.iter().enumerate() {
                line = line.child(
                    div()
                        .id(SharedString::from(format!("c-{i}-{r}-{c}")))
                        .flex_1()
                        .min_w(px(64.))
                        .px_2()
                        .py_1()
                        .border_b_1()
                        .border_r_1()
                        .border_color(rgb(theme::BORDER))
                        .when(r == 0, |d| {
                            d.bg(rgb(theme::BG_SUNKEN))
                                .font_weight(gpui::FontWeight::MEDIUM)
                        })
                        .text_xs()
                        .text_color(rgb(theme::TEXT))
                        .child(SharedString::from(cell.clone())),
                );
            }
            wrap = wrap.child(line);
        }
        wrap
    }

    fn math_img(&mut self, math: &SvgMath) -> impl IntoElement {
        let image = self
            .math_imgs
            .entry(math.svg.clone())
            .or_insert_with(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    math.svg.clone().into_bytes(),
                ))
            })
            .clone();
        img(image)
            .w(px(math.width))
            .h(px(math.height))
            .flex_shrink_0()
            .object_fit(gpui::ObjectFit::Contain)
    }
}

fn preview_key(doc: &crate::doc::Document) -> String {
    doc.blocks
        .iter()
        .map(|b| format!("{:?}:{}", b.kind, b.text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn wrap_text_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in text.split_whitespace() {
        if piece.is_empty() {
            continue;
        }
        let mut buf = String::new();
        for ch in piece.chars() {
            let cjk = is_cjk(ch);
            if cjk {
                if !buf.is_empty() && !buf.chars().any(is_cjk) {
                    out.push(std::mem::take(&mut buf));
                }
                buf.push(ch);
                if buf.chars().count() >= 8 {
                    out.push(std::mem::take(&mut buf));
                }
            } else {
                if buf.chars().any(is_cjk) {
                    out.push(std::mem::take(&mut buf));
                }
                buf.push(ch);
            }
        }
        if !buf.is_empty() {
            out.push(buf);
        }
    }
    if out.is_empty() && !text.trim().is_empty() {
        out.push(text.to_string());
    }
    out
}

fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{303F}'
            | '\u{3040}'..='\u{30FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF00}'..='\u{FFEF}'
    )
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
                .child(btn("empty-snip", "Snip", true, !capturing, move |_, cx| {
                    state.update(cx, |s, cx| s.request_capture(cx));
                }))
                .child(kbd_chip("Ctrl+Shift+S")),
        )
}
