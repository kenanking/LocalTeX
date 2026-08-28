use std::sync::Arc;

use gpui::{
    div, img, prelude::*, px, rgb, AnyElement, App, Context, Entity, EntityId, MouseButton,
    RenderImage, SharedString,
};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::scroll::{h_scroll_pane, overlay_scrollbar, ScrollAxis, ScrollChrome};
use super::selectable::{selectable_run, selectable_text};
use super::theme;
use super::widgets::{
    btn, copy_chip, icon_btn, kbd_chip, missing_image_slot, ocr_meta_bar, section_label, IconKind,
};
use crate::doc::{CopyKind, DocStatus, ImageSlot, OcrMeta};
use crate::preview::{segs_lines, Eqno, InlineSeg, PreviewBlock, PreviewLayout, SvgMath};
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
        let orig_hover = self.orig_hover;
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
            state.selected().and_then(|id| self.full(id))
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
                    SharedString::from(format!("copy-{}", row.exporter_id)),
                    row.label,
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

    fn preview_element(
        &mut self,
        doc_id: Uuid,
        status: &DocStatus,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity_id();
        let has_derived = self.derived.as_ref().is_some_and(|d| d.id == doc_id);
        let blocks: Vec<PreviewBlock> = self
            .derived
            .as_ref()
            .filter(|d| d.id == doc_id)
            .map(|d| d.preview.clone())
            .unwrap_or_default();

        if blocks.is_empty() {
            let msg = match status {
                DocStatus::Recognizing => "Recognizing…",
                DocStatus::Ready if has_derived => "No preview yet.",
                DocStatus::Ready => "Loading…",
                DocStatus::Failed(_) => "",
            };
            return div()
                .id("preview-empty")
                .w_full()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child(SharedString::from(msg))
                .into_any_element();
        }

        // Pane-width column: paragraphs wrap here. Wide tables / display
        // math scroll inside their own `h_scroll_pane` instead of stretching
        // this column (which would also stretch wrapped text).
        let mut col = div()
            .id("preview-doc")
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .flex()
            .flex_col()
            .gap_3()
            .flex_none()
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
            });

        let flow: Vec<(String, SharedString)> = blocks
            .iter()
            .enumerate()
            .filter_map(|(i, block)| match block {
                PreviewBlock::Paragraph(segs)
                | PreviewBlock::Heading { segs, .. }
                | PreviewBlock::Caption(segs) => {
                    Some((format!("p-{i}"), concat_inline_segs(segs).0.into()))
                }
                PreviewBlock::Fallback(text) => Some((format!("f-{i}"), text.clone().into())),
                PreviewBlock::Display { .. } | PreviewBlock::Table(_) => None,
            })
            .collect();
        self.preview.sel.borrow_mut().set_flow(flow);

        for (i, block) in blocks.iter().enumerate() {
            col = col.child(self.render_preview_block(i, block, view, cx));
        }
        col.into_any_element()
    }

    fn render_preview_block(
        &mut self,
        i: usize,
        block: &PreviewBlock,
        view: EntityId,
        cx: &mut App,
    ) -> impl IntoElement {
        match block {
            PreviewBlock::Paragraph(segs) => {
                let has_math = segs.iter().any(|s| matches!(s, InlineSeg::Math { .. }));
                div()
                    .id(SharedString::from(format!("p-{i}")))
                    .w_full()
                    .min_w_0()
                    .when(!has_math, |d| {
                        d.text_sm()
                            .line_height(px(22.))
                            .text_color(rgb(theme::TEXT))
                            .whitespace_normal()
                    })
                    .child(self.render_segs(format!("p-{i}"), segs, true, cx))
                    .into_any()
            }
            PreviewBlock::Heading { role, segs } => {
                let doc_title = *role == crate::doc::BlockRole::DocTitle;
                div()
                    .id(SharedString::from(format!("p-{i}")))
                    .w_full()
                    .min_w_0()
                    .text_color(rgb(theme::TEXT))
                    .when(doc_title, |d| {
                        d.pt(px(2.))
                            .pb(px(6.))
                            .text_xl()
                            .line_height(px(28.))
                            .font_weight(gpui::FontWeight::BOLD)
                    })
                    .when(!doc_title, |d| {
                        d.pt(px(12.))
                            .pb(px(4.))
                            .text_lg()
                            .line_height(px(24.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                    })
                    .child(self.render_segs(format!("p-{i}"), segs, false, cx))
                    .into_any()
            }
            PreviewBlock::Caption(segs) => div()
                .id(SharedString::from(format!("p-{i}")))
                .w_full()
                .min_w_0()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(self.render_segs(format!("p-{i}"), segs, false, cx))
                .into_any(),
            PreviewBlock::Display { math, eqno } => {
                self.render_display_math(i, math, eqno.as_ref(), view, cx)
            }
            PreviewBlock::Table(layout) => {
                let handle = self.preview.hscroll_handle(&format!("tbl-{i}"));
                let table_el = self.render_table_preview(i, layout, cx);
                h_scroll_pane(
                    format!("tbl-{i}"),
                    ScrollChrome {
                        handle: &handle,
                        drag: &self.preview.thumb,
                        hover: &self.preview.hscroll_hover,
                        view,
                    },
                    layout.width,
                    layout.height,
                    false,
                    table_el,
                )
                .into_any()
            }
            PreviewBlock::Fallback(text) => div()
                .id(SharedString::from(format!("f-{i}")))
                .w_full()
                .min_w_0()
                .text_sm()
                .whitespace_normal()
                .text_color(rgb(theme::TEXT))
                .child(selectable_text(
                    format!("f-{i}"),
                    text.clone(),
                    self.preview.sel.clone(),
                ))
                .into_any(),
        }
    }

    fn render_display_math(
        &mut self,
        i: usize,
        math: &SvgMath,
        eqno: Option<&Eqno>,
        view: EntityId,
        cx: &mut App,
    ) -> AnyElement {
        let handle = self.preview.hscroll_handle(&format!("d-{i}"));
        let measured: f32 = handle.bounds().size.width.into();
        if measured > 1.0 {
            self.preview.pane_w = measured;
        }
        let view_w = self.preview.pane_w;
        let tag_w = eqno.map(Eqno::width).unwrap_or(0.0);
        let trailing = eqno.is_some() && eqno_should_trail(math.width, tag_w, view_w);

        if trailing {
            let eqno = eqno.expect("trailing requires eqno");
            let content_w = math.width + EQNO_GAP + tag_w.max(1.0);
            let content_h = math.height.max(eqno.height());
            let img = self.math_img(math, cx);
            let number = self.eqno_el(eqno, cx);
            let row = div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(content_h))
                .child(img)
                .child(div().w(px(EQNO_GAP)).h(px(1.)))
                .child(number);
            return div()
                .w_full()
                .min_w_0()
                .py_3()
                .child(h_scroll_pane(
                    format!("d-{i}"),
                    ScrollChrome {
                        handle: &handle,
                        drag: &self.preview.thumb,
                        hover: &self.preview.hscroll_hover,
                        view,
                    },
                    content_w,
                    content_h,
                    false,
                    row,
                ))
                .into_any();
        }

        let pane = {
            let img = self.math_img(math, cx);
            h_scroll_pane(
                format!("d-{i}"),
                ScrollChrome {
                    handle: &handle,
                    drag: &self.preview.thumb,
                    hover: &self.preview.hscroll_hover,
                    view,
                },
                math.width,
                math.height,
                true,
                img,
            )
        };
        let mut inner = div().relative().w_full().min_w_0().child(pane);
        if let Some(eqno) = eqno {
            inner = inner.child(
                div()
                    .absolute()
                    .right(px(0.))
                    .top(px(0.))
                    .h(px(math.height.max(1.0)))
                    .flex()
                    .items_center()
                    .child(self.eqno_el(eqno, cx)),
            );
        }
        div().w_full().min_w_0().py_3().child(inner).into_any()
    }

    fn eqno_el(&mut self, eqno: &Eqno, cx: &mut App) -> AnyElement {
        if let Some(math) = &eqno.math {
            self.math_img(math, cx).into_any_element()
        } else {
            div()
                .text_size(px(18.))
                .line_height(px(22.))
                .text_color(rgb(theme::TEXT))
                .child(SharedString::from(crate::math::format_eqno(&eqno.raw)))
                .into_any()
        }
    }

    fn render_table_preview(
        &mut self,
        i: usize,
        layout: &PreviewLayout,
        cx: &mut App,
    ) -> impl IntoElement {
        let mut wrap = div()
            .id(SharedString::from(format!("tbl-{i}")))
            .relative()
            .flex_shrink_0()
            .w(px(layout.width.max(1.0)))
            .h(px(layout.height.max(1.0)))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded_md()
            .overflow_hidden();
        for cell in &layout.cells {
            let header = cell.header;
            let lines = segs_lines(&cell.segs);
            let stacked = lines.len() > 1;
            let center = !stacked && (cell.numeric || cell.colspan > 1);
            let cell_id = format!("c-{i}-{}-{}", cell.row, cell.col);
            let body = if stacked {
                let mut col = div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_0();
                for (k, line) in lines.iter().enumerate() {
                    col = col.child(self.render_segs(format!("{cell_id}-{k}"), line, false, cx));
                }
                col.into_any()
            } else {
                self.render_segs(cell_id.clone(), &cell.segs, false, cx)
            };
            wrap = wrap.child(
                div()
                    .id(SharedString::from(cell_id))
                    .absolute()
                    .left(px(cell.x))
                    .top(px(cell.y))
                    .w(px(cell.w))
                    .h(px(cell.h))
                    .px_2()
                    .overflow_hidden()
                    .flex()
                    .when(stacked, |d| d.flex_col().items_start().py_1())
                    .when(!stacked, |d| d.items_center())
                    .when(center, |d| d.justify_center())
                    .border_b_1()
                    .border_r_1()
                    .border_color(rgb(theme::BORDER))
                    .when(header, |d| {
                        d.bg(rgb(theme::BG_SUNKEN))
                            .font_weight(gpui::FontWeight::MEDIUM)
                    })
                    .text_xs()
                    .text_color(rgb(theme::TEXT))
                    .child(body),
            );
        }
        wrap
    }

    fn render_segs(
        &mut self,
        id: String,
        segs: &[InlineSeg],
        paragraph: bool,
        cx: &mut App,
    ) -> AnyElement {
        let has_math = segs.iter().any(|s| matches!(s, InlineSeg::Math { .. }));
        if !has_math {
            let text = segs
                .iter()
                .map(|s| match s {
                    InlineSeg::Text(t) => t.as_str(),
                    InlineSeg::Math { .. } => "",
                })
                .collect::<Vec<_>>()
                .join("");
            return div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .child(selectable_text(id, text, self.preview.sel.clone()))
                .into_any();
        }
        let (para_text, ranges) = concat_inline_segs(segs);
        let mut row = div()
            .id(SharedString::from(format!("{id}-row")))
            .w_full()
            .min_w_0()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .when(paragraph, |d| {
                d.text_sm()
                    .line_height(px(22.))
                    .text_color(rgb(theme::TEXT))
            });
        let sel = self.preview.sel.clone();
        for (j, seg) in segs.iter().enumerate() {
            match seg {
                InlineSeg::Text(t) => {
                    for (k, (off, tok)) in wrap_units(t).into_iter().enumerate() {
                        if tok.is_empty() {
                            continue;
                        }
                        let run_id = format!("{id}-t-{j}-{k}");
                        row = row.child(
                            div()
                                .id(SharedString::from(run_id.clone()))
                                .flex_shrink_0()
                                .when(paragraph, |d| {
                                    d.text_sm()
                                        .line_height(px(22.))
                                        .text_color(rgb(theme::TEXT))
                                })
                                .when(!paragraph, |d| d.text_xs().text_color(rgb(theme::TEXT)))
                                .child(selectable_run(
                                    run_id,
                                    id.clone(),
                                    tok,
                                    para_text.clone(),
                                    ranges[j].start + off,
                                    sel.clone(),
                                )),
                        );
                    }
                }
                InlineSeg::Math { svg, .. } => {
                    let math_on = {
                        let state = sel.borrow();
                        state
                            .span_in(&id, para_text.len())
                            .is_some_and(|r| ranges_overlap(&r, &ranges[j]))
                    };
                    row = row.child(
                        div()
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .px(px(1.))
                            .when(math_on, |d| d.bg(theme::accent_soft()))
                            .child(self.math_img(svg, cx)),
                    );
                }
            }
        }
        row.into_any()
    }

    fn math_img(&mut self, math: &SvgMath, cx: &mut App) -> impl IntoElement {
        let image = self.math_image(&math.svg, cx);
        img(image)
            .w(px(math.width))
            .h(px(math.height))
            .flex_shrink_0()
            .object_fit(gpui::ObjectFit::Contain)
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

const EQNO_GAP: f32 = 12.0;

fn eqno_should_trail(math_w: f32, tag_w: f32, pane_w: f32) -> bool {
    pane_w > 1.0 && math_w + tag_w + EQNO_GAP > pane_w
}

fn concat_inline_segs(segs: &[InlineSeg]) -> (String, Vec<std::ops::Range<usize>>) {
    let mut text = String::new();
    let mut ranges = Vec::with_capacity(segs.len());
    for seg in segs {
        let start = text.len();
        match seg {
            InlineSeg::Text(t) => text.push_str(t),
            InlineSeg::Math { tex, .. } => {
                text.push('$');
                text.push_str(tex);
                text.push('$');
            }
        }
        ranges.push(start..text.len());
    }
    (text, ranges)
}

fn ranges_overlap(a: &std::ops::Range<usize>, b: &std::ops::Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Break a text run at wrap opportunities: Latin words and CJK characters.
///
/// Whitespace is glue, not a box. CSS `white-space: normal` discards
/// collapsible spaces at the start of a line; a space that is its own flex
/// item would wrap onto the next row and indent it. Trailing glue stays on
/// the previous word so the gap sits at the end of the line, never the start.
fn wrap_units(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let ch = text[i..]
            .chars()
            .next()
            .expect("byte index at char boundary");
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }
        let start = i;
        if is_cjk(ch) {
            i += ch.len_utf8();
        } else {
            i += ch.len_utf8();
            while i < text.len() {
                let c = text[i..].chars().next().expect("char");
                if c.is_whitespace() || is_cjk(c) {
                    break;
                }
                i += c.len_utf8();
            }
        }
        while i < text.len() {
            let c = text[i..].chars().next().expect("char");
            if !c.is_whitespace() {
                break;
            }
            i += c.len_utf8();
        }
        out.push((start, text[start..i].to_string()));
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

#[cfg(test)]
mod tests {
    use super::wrap_units;

    #[test]
    fn wrap_units_breaks_on_spaces() {
        let parts = wrap_units("hello world");
        assert_eq!(parts, vec![(0, "hello ".into()), (6, "world".into())]);
    }

    #[test]
    fn wrap_units_does_not_emit_leading_space() {
        let parts = wrap_units(" world");
        assert_eq!(parts, vec![(1, "world".into())]);
    }

    #[test]
    fn wrap_units_attaches_double_space_to_previous_word() {
        let parts = wrap_units("hello  world");
        assert_eq!(parts, vec![(0, "hello  ".into()), (7, "world".into())]);
    }

    #[test]
    fn wrap_units_breaks_cjk_per_char() {
        let parts = wrap_units("你好");
        assert_eq!(parts, vec![(0, "你".into()), ("你".len(), "好".into())]);
    }

    #[test]
    fn wrap_units_keeps_latin_word() {
        let parts = wrap_units("E=mc^2");
        assert_eq!(parts, vec![(0, "E=mc^2".into())]);
    }

    #[test]
    fn eqno_mode_ignores_unmeasured_pane() {
        assert!(!super::eqno_should_trail(200.0, 24.0, 0.0));
        assert!(!super::eqno_should_trail(200.0, 24.0, 400.0));
        assert!(super::eqno_should_trail(200.0, 24.0, 220.0));
    }
}
