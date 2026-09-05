use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    div, img, prelude::*, px, rgb, AnyElement, App, Context, EntityId, MouseButton, SharedString,
};

use super::main_window::MainWindow;
use super::scroll::{h_scroll_pane, hscroll_should_center, ScrollChrome};
use super::selectable::{selectable_run, selectable_text, PreviewSel};
use super::theme;
use crate::doc::DocStatus;
use crate::math::ScriptKind;
use crate::prefs::ContentFontSize;
use crate::preview::{
    segs_lines, Eqno, InlineSeg, PreviewBlock, PreviewLayout, ScriptGlyph, SvgMath,
};
use uuid::Uuid;

struct TextRunSpec {
    run_id: String,
    block_id: String,
    tok: String,
    para_text: String,
    start: usize,
    paragraph: bool,
}

#[derive(Clone, Copy)]
struct PreviewRenderContext {
    view: EntityId,
    pane_w: f32,
    font: ContentFontSize,
}

impl MainWindow {
    pub(crate) fn preview_element(
        &mut self,
        doc_id: Uuid,
        status: &DocStatus,
        pane_w: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        if let Some(doc) = self.state.read(cx).doc(doc_id) {
            if let Some(error) = doc
                .source_error
                .as_ref()
                .filter(|_| doc.source_feedback_ready())
            {
                let source = doc.raw_text.clone().unwrap_or_default();
                let error = error.clone();
                let font = self.state.read(cx).prefs.content_font;
                self.preview
                    .sel
                    .borrow_mut()
                    .set_flow(vec![("source-error".into(), source.clone().into())]);
                return self.render_source_failure(
                    "source-error".into(),
                    source,
                    Some(error),
                    font,
                    cx,
                );
            }
            if doc.raw_text.as_ref().is_some_and(|s| s.trim().is_empty()) {
                return div()
                    .text_sm()
                    .text_color(rgb(theme::MUTED))
                    .child("No content")
                    .into_any_element();
            }
        }
        let view = cx.entity_id();
        let has_derived = self.media.derived.as_ref().is_some_and(|d| d.id == doc_id);
        let blocks = self
            .media
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

        // Reading-width column: paragraphs wrap here. Wide tables / display
        // math scroll inside their own `h_scroll_pane` instead of stretching
        // this column (which would also stretch wrapped text).
        let (cap, font) = {
            let prefs = &self.state.read(cx).prefs;
            (px(prefs.reading_width.cap_px()), prefs.content_font)
        };
        let mut col = div()
            .id("preview-doc")
            .w_full()
            .max_w(cap)
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
                PreviewBlock::Fallback(text) | PreviewBlock::Error { source: text, .. } => {
                    Some((format!("f-{i}"), text.clone().into()))
                }
                PreviewBlock::Display { .. } | PreviewBlock::Table(_) => None,
            })
            .collect();
        self.preview.sel.borrow_mut().set_flow(flow);

        for (i, block) in blocks.iter().enumerate() {
            col = col.child(self.render_preview_block(i, block, view, pane_w, font, cx));
        }
        col.into_any_element()
    }

    fn render_preview_block(
        &mut self,
        i: usize,
        block: &PreviewBlock,
        view: EntityId,
        pane_w: f32,
        font: ContentFontSize,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let m = font.metrics();
        match block {
            PreviewBlock::Paragraph(segs) => {
                let has_math = segs_have_glyphs(segs);
                div()
                    .id(SharedString::from(format!("p-{i}")))
                    .w_full()
                    .min_w_0()
                    .when(!has_math, |d| {
                        d.text_size(px(m.body))
                            .line_height(px(m.body_line))
                            .text_color(rgb(theme::INK))
                            .whitespace_normal()
                    })
                    .child(self.render_segs(format!("p-{i}"), segs, true, font, cx))
                    .into_any()
            }
            PreviewBlock::Heading { role, segs } => {
                let doc_title = *role == crate::doc::BlockRole::DocTitle;
                div()
                    .id(SharedString::from(format!("p-{i}")))
                    .w_full()
                    .min_w_0()
                    .text_color(rgb(theme::INK))
                    .when(doc_title, |d| {
                        d.pt(px(2.))
                            .pb(px(6.))
                            .text_size(px(m.title))
                            .line_height(px(m.title_line))
                            .font_weight(gpui::FontWeight::BOLD)
                    })
                    .when(!doc_title, |d| {
                        d.pt(px(12.))
                            .pb(px(4.))
                            .text_size(px(m.heading))
                            .line_height(px(m.heading_line))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                    })
                    .child(self.render_segs(format!("p-{i}"), segs, false, font, cx))
                    .into_any()
            }
            PreviewBlock::Caption(segs) => div()
                .id(SharedString::from(format!("p-{i}")))
                .w_full()
                .min_w_0()
                .text_size(px(m.caption))
                .text_color(rgb(theme::INK))
                .child(self.render_segs(format!("p-{i}"), segs, false, font, cx))
                .into_any(),
            PreviewBlock::Display { math, eqno } => self.render_display_math(
                i,
                math,
                eqno.as_ref(),
                PreviewRenderContext { view, pane_w, font },
                cx,
            ),
            PreviewBlock::Table(layout) => {
                let handle = self.preview.hscroll_handle(&format!("tbl-{i}"));
                let table_el = self.render_table_preview(i, layout, font, cx);
                let table = h_scroll_pane(
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
                );
                let mut column = div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(table);
                for cell in &layout.cells {
                    for (j, seg) in cell.segs.iter().enumerate() {
                        if let InlineSeg::Error { source, error } = seg {
                            column = column.child(self.render_source_failure(
                                format!("table-error-{i}-{}-{}-{j}", cell.row, cell.col),
                                source.clone(),
                                Some(error.clone()),
                                font,
                                cx,
                            ));
                        }
                    }
                }
                column.into_any()
            }
            PreviewBlock::Fallback(text) => {
                self.render_source_failure(format!("f-{i}"), text.clone(), None, font, cx)
            }
            PreviewBlock::Error { source, error } => self.render_source_failure(
                format!("f-{i}"),
                source.clone(),
                Some(error.clone()),
                font,
                cx,
            ),
        }
    }

    fn render_source_failure(
        &self,
        id: String,
        source: String,
        error: Option<String>,
        font: ContentFontSize,
        cx: &App,
    ) -> AnyElement {
        let source = if id.starts_with("f-")
            && self
                .media
                .derived
                .as_ref()
                .is_some_and(|d| d.preview.len() == 1)
        {
            self.state
                .read(cx)
                .selected_doc()
                .and_then(|doc| doc.raw_text.clone())
                .unwrap_or(source)
        } else {
            source
        };
        let current = self.state.read(cx).selected_doc().is_some_and(|doc| {
            !doc.source_pending
                && doc.source_feedback_ready()
                && (doc.source_error.is_some()
                    || self
                        .media
                        .derived
                        .as_ref()
                        .is_some_and(|d| d.id == doc.id && d.revision == doc.revision))
        });
        let m = font.metrics();
        let copy = source.clone();
        div()
            .w_full()
            .min_w_0()
            .flex_none()
            .flex()
            .flex_col()
            .gap_3()
            .when(current, |d| {
                d.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .size(px(6.))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(rgb(theme::WARN)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(m.caption))
                                .line_height(px(m.table_line))
                                .text_color(rgb(theme::TEXT))
                                .child("Preview unavailable"),
                        )
                        .child(div().flex_shrink_0().child(super::widgets::icon_btn_sized(
                            format!("{id}-copy"),
                            super::widgets::IconKind::Copy,
                            "Copy source",
                            false,
                            true,
                            super::widgets::IconBtnSize {
                                hit: px(28.),
                                glyph: px(15.),
                                kbd: None,
                            },
                            move |_, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    copy.clone(),
                                ));
                            },
                        ))),
                )
            })
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(theme::BG))
                    .font_family(theme::SOURCE_FONT)
                    .text_size(px(m.body))
                    .line_height(px(m.body_line))
                    .text_color(rgb(theme::INK))
                    .child(selectable_text(
                        id.clone(),
                        source,
                        self.preview.sel.clone(),
                    )),
            )
            .when(current, |d| {
                d.children(error.map(|error| {
                    div()
                        .w_full()
                        .min_w_0()
                        .pl_3()
                        .border_l_1()
                        .border_color(rgb(theme::BORDER))
                        .child(
                            div()
                                .id(SharedString::from(format!("{id}-error-scroll")))
                                .w_full()
                                .min_w_0()
                                .max_h(px(108.))
                                .overflow_y_scroll()
                                .font_family(theme::SOURCE_FONT)
                                .text_size(px(m.caption))
                                .line_height(px(m.table_line))
                                .text_color(rgb(theme::MUTED))
                                .child(selectable_text(
                                    format!("{id}-error"),
                                    error,
                                    self.preview.sel.clone(),
                                )),
                        )
                }))
            })
            .into_any_element()
    }

    fn render_display_math(
        &mut self,
        i: usize,
        math: &SvgMath,
        eqno: Option<&Eqno>,
        render: PreviewRenderContext,
        cx: &mut App,
    ) -> AnyElement {
        let PreviewRenderContext { view, pane_w, font } = render;
        let handle = self.preview.hscroll_handle(&format!("d-{i}"));
        let tag_w = eqno.map(Eqno::width).unwrap_or(0.0);
        let trailing = eqno.is_some() && eqno_should_trail(math.width, tag_w, pane_w);

        if trailing {
            let eqno = eqno.expect("trailing requires eqno");
            let content_w = math.width + EQNO_GAP + tag_w.max(1.0);
            let content_h = math.height.max(eqno.height());
            let img = self.math_img(math, cx);
            let number = self.eqno_el(eqno, font, cx);
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
                hscroll_should_center(math.width, pane_w),
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
                    .child(self.eqno_el(eqno, font, cx)),
            );
        }
        div().w_full().min_w_0().py_3().child(inner).into_any()
    }

    fn eqno_el(&mut self, eqno: &Eqno, font: ContentFontSize, cx: &mut App) -> AnyElement {
        if let Some(math) = &eqno.math {
            self.math_img(math, cx).into_any_element()
        } else {
            let m = font.metrics();
            div()
                .text_size(px(m.display_math as f32))
                .line_height(px(m.display_math as f32 + 4.0))
                .text_color(rgb(theme::INK))
                .child(SharedString::from(crate::math::format_eqno(&eqno.raw)))
                .into_any()
        }
    }

    fn render_table_preview(
        &mut self,
        i: usize,
        layout: &PreviewLayout,
        font: ContentFontSize,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
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
                    col = col.child(self.render_segs(
                        format!("{cell_id}-{k}"),
                        line,
                        false,
                        font,
                        cx,
                    ));
                }
                col.into_any()
            } else {
                self.render_segs(cell_id.clone(), &cell.segs, false, font, cx)
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
                    .text_size(px(font.metrics().caption))
                    .text_color(rgb(theme::INK))
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
        font: ContentFontSize,
        cx: &mut App,
    ) -> AnyElement {
        if !segs_have_glyphs(segs) {
            let text = segs
                .iter()
                .map(|s| match s {
                    InlineSeg::Text(t) => t.as_str(),
                    _ => "",
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
        let m = font.metrics();
        let mut row = div()
            .id(SharedString::from(format!("{id}-row")))
            .w_full()
            .min_w_0()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_end()
            .when(paragraph, |d| {
                d.text_size(px(m.body))
                    .line_height(px(m.inline_line))
                    .text_color(rgb(theme::INK))
            });
        let sel = self.preview.sel.clone();
        let line_h = if paragraph {
            m.inline_line
        } else {
            m.table_line
        };
        for (j, seg) in segs.iter().enumerate() {
            let glue = next_text_starts_with_space(segs.get(j + 1));
            let math_on = {
                let state = sel.borrow();
                state
                    .span_in(&id, para_text.len())
                    .is_some_and(|r| ranges_overlap(&r, &ranges[j]))
            };
            match seg {
                InlineSeg::Text(t) => {
                    for (k, (off, tok)) in wrap_units(t).into_iter().enumerate() {
                        if tok.is_empty() {
                            continue;
                        }
                        row = row.child(self.text_run_el(
                            TextRunSpec {
                                run_id: format!("{id}-t-{j}-{k}"),
                                block_id: id.clone(),
                                tok,
                                para_text: para_text.clone(),
                                start: ranges[j].start + off,
                                paragraph,
                            },
                            sel.clone(),
                            font,
                        ));
                    }
                }
                InlineSeg::TextScript { nucleus, glyph } => {
                    let units = wrap_units(nucleus);
                    let last = units.iter().rposition(|(_, tok)| !tok.is_empty());
                    if last.is_none() {
                        row = row.child(
                            div()
                                .flex()
                                .flex_none()
                                .items_end()
                                .flex_shrink_0()
                                .when(math_on, |d| d.bg(theme::accent_soft()))
                                .child(self.script_el(glyph, line_h, cx))
                                .when(glue, |d| d.child(glue_el(paragraph, font))),
                        );
                        continue;
                    }
                    for (k, (off, tok)) in units.into_iter().enumerate() {
                        if tok.is_empty() {
                            continue;
                        }
                        let run = self.text_run_el(
                            TextRunSpec {
                                run_id: format!("{id}-t-{j}-{k}"),
                                block_id: id.clone(),
                                tok,
                                para_text: para_text.clone(),
                                start: ranges[j].start + off,
                                paragraph,
                            },
                            sel.clone(),
                            font,
                        );
                        if Some(k) == last {
                            row = row.child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_none()
                                    .items_end()
                                    .flex_shrink_0()
                                    .when(math_on, |d| d.bg(theme::accent_soft()))
                                    .child(run)
                                    .child(self.script_el(glyph, line_h, cx))
                                    .when(glue, |d| d.child(glue_el(paragraph, font))),
                            );
                        } else {
                            row = row.child(run);
                        }
                    }
                }
                InlineSeg::Error { source, error } => {
                    if id.starts_with("c-") {
                        row = row.child(div().font_family(theme::SOURCE_FONT).child(
                            selectable_text(
                                format!("{id}-error-{j}"),
                                source.clone(),
                                self.preview.sel.clone(),
                            ),
                        ));
                    } else {
                        row = row.child(self.render_source_failure(
                            format!("{id}-error-{j}"),
                            source.clone(),
                            Some(error.clone()),
                            font,
                            cx,
                        ));
                    }
                }
                InlineSeg::Math { svg, .. } => {
                    let img = self.math_img(svg, cx);
                    let glyph = if svg.height <= line_h + 2.0 {
                        div()
                            .flex_none()
                            .h(px(line_h))
                            .flex()
                            .items_center()
                            .child(img)
                            .into_any()
                    } else {
                        img.into_any_element()
                    };
                    row = row.child(
                        div()
                            .flex()
                            .flex_none()
                            .items_end()
                            .flex_shrink_0()
                            .px(px(1.))
                            .when(math_on, |d| d.bg(theme::accent_soft()))
                            .child(glyph)
                            .when(glue, |d| d.child(glue_el(paragraph, font))),
                    );
                }
            }
        }
        row.into_any()
    }

    fn text_run_el(
        &self,
        spec: TextRunSpec,
        sel: Rc<RefCell<PreviewSel>>,
        font: ContentFontSize,
    ) -> impl IntoElement + use<> {
        let m = font.metrics();
        div()
            .id(SharedString::from(spec.run_id.clone()))
            .flex_shrink_0()
            .when(spec.paragraph, |d| {
                d.text_size(px(m.body))
                    .line_height(px(m.inline_line))
                    .text_color(rgb(theme::INK))
            })
            .when(!spec.paragraph, |d| {
                d.text_size(px(m.caption)).text_color(rgb(theme::INK))
            })
            .child(selectable_run(
                spec.run_id,
                spec.block_id,
                spec.tok,
                spec.para_text,
                spec.start,
                sel,
            ))
    }

    fn script_el(&mut self, glyph: &ScriptGlyph, line_h: f32, cx: &mut App) -> AnyElement {
        let img = self.math_img(&glyph.svg, cx);
        match glyph.kind {
            ScriptKind::Sub => img.into_any_element(),
            ScriptKind::Super => {
                let rise = (line_h - glyph.svg.height).max(0.0);
                div().flex_none().pb(px(rise)).child(img).into_any()
            }
        }
    }

    fn math_img(&mut self, math: &SvgMath, cx: &mut App) -> impl IntoElement + use<> {
        let image = self.math_image(&math.svg, cx);
        img(image)
            .w(px(math.width))
            .h(px(math.height))
            .flex_shrink_0()
            .object_fit(gpui::ObjectFit::Contain)
    }
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
            InlineSeg::Error { source, .. } => text.push_str(source),
            InlineSeg::TextScript { nucleus, glyph } => {
                text.push_str(nucleus);
                text.push('$');
                text.push_str(&glyph.tex);
                text.push('$');
            }
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

fn segs_have_glyphs(segs: &[InlineSeg]) -> bool {
    segs.iter().any(|s| !matches!(s, InlineSeg::Text(_)))
}

fn next_text_starts_with_space(next: Option<&InlineSeg>) -> bool {
    let text = match next {
        Some(InlineSeg::Text(t)) => t.as_str(),
        Some(InlineSeg::TextScript { nucleus, .. }) => nucleus.as_str(),
        _ => return false,
    };
    text.chars().next().is_some_and(char::is_whitespace)
}

fn glue_el(paragraph: bool, font: ContentFontSize) -> gpui::Div {
    let m = font.metrics();
    // A flex item whose only text is U+0020 collapses to zero width.
    div()
        .flex_shrink_0()
        .when(paragraph, |d| {
            d.text_size(px(m.body)).line_height(px(m.inline_line))
        })
        .when(!paragraph, |d| d.text_size(px(m.caption)))
        .child("\u{00A0}")
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

#[cfg(test)]
mod tests {
    use super::{next_text_starts_with_space, wrap_units};
    use crate::preview::InlineSeg;

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
    fn glue_follows_atom_when_next_text_has_space() {
        assert!(next_text_starts_with_space(Some(&InlineSeg::Text(
            " by 2.65%".into()
        ))));
        let next_script = InlineSeg::TextScript {
            nucleus: " by 2.65%, and AP".into(),
            glyph: crate::preview::ScriptGlyph {
                svg: crate::preview::SvgMath {
                    svg: String::new(),
                    width: 1.0,
                    height: 1.0,
                },
                tex: "{}_{75}".into(),
                kind: crate::math::ScriptKind::Sub,
            },
        };
        assert!(next_text_starts_with_space(Some(&next_script)));
        assert!(!next_text_starts_with_space(Some(&InlineSeg::Text(
            ", and".into()
        ))));
        assert!(!next_text_starts_with_space(None));
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
