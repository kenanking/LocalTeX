use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, App, Context, CursorStyle, Entity, FocusHandle, Focusable, Image,
    RenderImage, ScrollHandle, SharedString, Window,
};
use uuid::Uuid;

use super::draw::DrawBoard;
use super::search_field::SearchField;
use super::settings::SettingsTab;
use super::theme;
use super::widgets::{icon_btn, status_dot, IconKind};
use crate::actions::{
    Capture, CloseSheet, CopyExport, DeleteSelected, OpenSettings, QuitApp, RetryOcr, SelectNext,
    SelectPrev, StartDraw, ToggleFormat, UploadImage,
};
use crate::doc::{CopyKind, DocStatus};
use crate::imgutil;
use crate::ocr::EngineStatus;
use crate::preview::PreviewBlock;
use crate::state::AppState;

#[derive(Clone)]
pub(crate) enum View {
    Library,
    Draw,
    Settings,
}

pub struct MainWindow {
    pub(crate) state: Entity<AppState>,
    focus: FocusHandle,
    pub(crate) snip_list_focus: FocusHandle,
    pub(crate) search: Entity<SearchField>,
    pub(crate) thumbs: HashMap<Uuid, Arc<RenderImage>>,
    pub(crate) fulls: HashMap<Uuid, Arc<RenderImage>>,
    pub(crate) previews: HashMap<Uuid, (String, Vec<PreviewBlock>)>,
    pub(crate) math_imgs: HashMap<String, Arc<Image>>,
    pub(crate) view: View,
    pub(crate) board: DrawBoard,
    settings_tab: SettingsTab,
    pub(crate) copied: Option<(Uuid, CopyKind)>,
    pub(crate) orig_hover: bool,
    orig_zoomed: bool,
    zoom_doc: Option<Uuid>,
    pub(crate) preview_scroll: ScrollHandle,
    pub(crate) preview_scroll_doc: Option<Uuid>,
    pub(crate) preview_bar_pending: bool,
}

impl MainWindow {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let snip_list_focus = cx.focus_handle();
        window.focus(&snip_list_focus);
        let state_for_close = state.clone();
        window.on_window_should_close(cx, move |window, cx| {
            let action = state_for_close.read(cx).prefs.close_action;
            AppState::handle_main_close(action, window, cx)
        });
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let search = cx.new(|cx| SearchField::new("Search text or LaTeX", cx));
        cx.observe(&search, |this, field, cx| {
            let query = field.read(cx).text();
            this.state
                .update(cx, |state, cx| state.set_search_query(query, cx));
        })
        .detach();
        state.update(cx, |state, cx| state.boot_selected(cx));
        Self {
            state,
            focus,
            snip_list_focus,
            search,
            thumbs: HashMap::new(),
            fulls: HashMap::new(),
            previews: HashMap::new(),
            math_imgs: HashMap::new(),
            view: View::Library,
            board: DrawBoard::new(),
            settings_tab: SettingsTab::General,
            copied: None,
            orig_hover: false,
            orig_zoomed: false,
            zoom_doc: None,
            preview_scroll: ScrollHandle::new(),
            preview_scroll_doc: None,
            preview_bar_pending: false,
        }
    }

    fn upload(&mut self, _: &UploadImage, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
        self.state.update(cx, |state, cx| state.request_upload(cx));
    }

    fn start_draw(&mut self, _: &StartDraw, _: &mut Window, cx: &mut Context<Self>) {
        self.unzoom();
        self.view = View::Draw;
        self.board = DrawBoard::new();
        cx.notify();
    }

    fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.unzoom();
        self.view = if matches!(self.view, View::Settings) {
            View::Library
        } else {
            View::Settings
        };
        cx.notify();
    }

    fn close_sheet(&mut self, _: &CloseSheet, _: &mut Window, cx: &mut Context<Self>) {
        if self.orig_zoomed {
            self.unzoom();
            cx.notify();
            return;
        }
        self.dismiss_sheet(cx);
    }

    fn capture(&mut self, _: &Capture, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
        self.state.update(cx, |state, cx| state.request_capture(cx));
    }

    pub fn dismiss_sheet(&mut self, cx: &mut Context<Self>) {
        let was_zoom = self.orig_zoomed;
        self.unzoom();
        if matches!(self.view, View::Draw | View::Settings) {
            self.view = View::Library;
            cx.notify();
        } else if was_zoom {
            cx.notify();
        }
    }

    pub(crate) fn unzoom(&mut self) {
        self.orig_zoomed = false;
        self.orig_hover = false;
        self.zoom_doc = None;
    }

    pub(crate) fn zoom_original(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.orig_zoomed = true;
        self.orig_hover = false;
        self.zoom_doc = Some(id);
        cx.notify();
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
        self.previews.retain(|id, _| live.contains(id));
        let keep_full: HashSet<Uuid> = state.gpu_full_ids().into_iter().collect();
        self.fulls
            .retain(|id, _| live.contains(id) && keep_full.contains(id));

        for doc in state.visible_docs() {
            if self.thumbs.contains_key(&doc.id) {
                continue;
            }
            if let Some(render) = imgutil::jpeg_to_render(&doc.thumb_jpeg) {
                self.thumbs.insert(doc.id, render);
            } else if let Some(pixels) = doc.image.pixels() {
                let thumb = imgutil::thumbnail(pixels, 56, 40);
                self.thumbs.insert(doc.id, imgutil::rgba_to_render(&thumb));
            }
        }

        let Some(sel) = state.selected else {
            return;
        };
        if self.fulls.contains_key(&sel) {
            return;
        }
        if let Some(doc) = state.selected_doc() {
            if let Some(pixels) = doc.image.pixels() {
                self.fulls.insert(sel, imgutil::gpu_display_image(pixels));
            }
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
            self.ensure_images(state);
            if self.orig_zoomed && self.zoom_doc != state.selected {
                self.orig_zoomed = false;
                self.orig_hover = false;
                self.zoom_doc = None;
            }
            let (status_kind, status_label) = chrome(state);
            (
                status_kind,
                status_label,
                state.is_capturing(),
                !state.documents.is_empty(),
                state.visible_ids.len(),
            )
        };
        let history = self.render_history(n_docs, cx);
        let detail = self.render_detail(capturing, cx);
        let zoom = self.render_orig_zoom(cx);
        let view = self.view.clone();
        let orig_zoomed = self.orig_zoomed;
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
                    .w_full()
                    .when(matches!(view, View::Library) && orig_zoomed, |d| {
                        d.child(zoom)
                    })
                    .when(matches!(view, View::Library) && !orig_zoomed, |d| {
                        d.when(has_docs, |d| d.child(history)).child(detail)
                    })
                    .when(matches!(view, View::Draw), |d| {
                        d.child(self.render_draw(cx))
                    })
                    .when(matches!(view, View::Settings), |d| {
                        let tab = self.settings_tab;
                        let entity = cx.entity();
                        d.flex_col().child(super::settings::page(
                            self.state.clone(),
                            tab,
                            move |tab, cx| {
                                entity.update(cx, |this, cx| {
                                    this.settings_tab = tab;
                                    cx.notify();
                                });
                            },
                            cx,
                        ))
                    }),
            )
            .child(self.render_footer(status_kind, status_label))
            .into_any_element()
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
            ))
            .child(self.tool_btn(
                "tool-draw",
                IconKind::Draw,
                "Create snip from drawing  Ctrl+D",
                matches!(view, View::Draw),
                !capturing,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.unzoom();
                            this.view = View::Draw;
                            this.board = DrawBoard::new();
                            cx.notify();
                        });
                    }
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_3()
                    .text_sm()
                    .text_ellipsis()
                    .text_color(rgb(theme::MUTED))
                    .child(crate::identity::APP_NAME),
            )
            .child(self.tool_btn(
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
            .child(self.tool_btn(
                "tool-settings",
                IconKind::Settings,
                "Settings  Ctrl+,",
                matches!(view, View::Settings),
                true,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.unzoom();
                            this.view = if matches!(this.view, View::Settings) {
                                View::Library
                            } else {
                                View::Settings
                            };
                            cx.notify();
                        });
                    }
                },
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
    ) -> impl IntoElement {
        icon_btn(id, kind, hint, active, enabled, on_click)
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
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_ellipsis()
                    .text_color(rgb(theme::MUTED))
                    .child(SharedString::from(status_label)),
            )
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
