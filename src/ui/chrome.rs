use gpui::{div, prelude::*, px, rgb, Context, SharedString};

use super::main_window::{MainWindow, View};
use super::theme;
use super::widgets::{icon_btn, status_dot, IconKind};
use crate::doc::DocStatus;
use crate::ocr::EngineStatus;
use crate::state::AppState;

impl MainWindow {
    pub(crate) fn render_topbar(
        &self,
        capturing: bool,
        has_selected: bool,
        can_open_docx: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.clone();
        let view = self.view.clone();

        div()
            .flex()
            .items_center()
            .h(px(44.))
            .px_3()
            .gap_1()
            .bg(rgb(theme::BG_RAISED))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child({
                let entity = cx.entity();
                div()
                    .id("topbar-home")
                    .px_2()
                    .mr_1()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .text_sm()
                    .text_color(rgb(theme::MUTED))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.dismiss_sheet(cx));
                    })
                    .child(crate::identity::APP_NAME)
            })
            .child(icon_btn(
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
            ))
            .child(icon_btn(
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
            ))
            .child(icon_btn(
                "tool-paste",
                IconKind::Paste,
                "Paste image or path from clipboard  Ctrl+V",
                false,
                !capturing,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.dismiss_sheet(cx);
                            this.state.update(cx, |s, cx| s.request_paste(cx));
                        });
                    }
                },
            ))
            .child(icon_btn(
                "tool-draw",
                IconKind::Draw,
                "Create snip from drawing  Ctrl+D",
                matches!(view, View::Draw),
                !capturing,
                {
                    let entity = cx.entity();
                    move |window, cx| {
                        entity.update(cx, |this, cx| this.toggle_draw(window, cx));
                    }
                },
            ))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(rgb(theme::TRACK_OFF)))
            .child(icon_btn(
                "tool-word",
                IconKind::Word,
                "Open as Word document",
                false,
                can_open_docx,
                {
                    let state = state.clone();
                    move |_, cx| {
                        state.update(cx, |s, cx| s.open_docx_selected(cx));
                    }
                },
            ))
            .child(div().flex_1())
            .child(icon_btn(
                "tool-delete",
                IconKind::Delete,
                "Delete snip  Delete",
                false,
                has_selected && matches!(view, View::Library),
                {
                    let state = state.clone();
                    move |_, cx| {
                        state.update(cx, |s, cx| s.delete_selected(cx));
                    }
                },
            ))
            .child(icon_btn(
                "tool-settings",
                IconKind::Settings,
                "Settings  Ctrl+,",
                matches!(view, View::Settings),
                true,
                {
                    let entity = cx.entity();
                    move |window, cx| {
                        entity.update(cx, |this, cx| this.toggle_settings(window, cx));
                    }
                },
            ))
    }

    pub(crate) fn render_footer(
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
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_ellipsis()
                    .text_color(rgb(theme::MUTED))
                    .child(SharedString::from(status_label)),
            )
    }
}

pub(crate) fn chrome(state: &AppState) -> (theme::StatusKind, String) {
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
        EngineStatus::Ready => (theme::StatusKind::Ready, "Ready when you are".into()),
    }
}
