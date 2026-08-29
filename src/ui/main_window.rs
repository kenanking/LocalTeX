use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, rgb, App, ClipboardItem, Context, CursorStyle, Entity, FocusHandle,
    Focusable, Image, MouseButton, RenderImage, ScrollHandle, SharedString, Timer, Window,
};
use uuid::Uuid;

use super::draw::DrawBoard;
use super::history::{HistoryPane, SIDEBAR_MAX, SIDEBAR_MIN};
use super::scroll::{ScrollThumbDrag, ThumbDragCatcher};
use super::search_field::SearchField;
use super::selectable::PreviewSel;
use super::settings::SettingsPane;
use super::theme;
use super::widgets::{icon_btn, status_dot, IconKind};
use crate::actions::{
    Capture, CloseSheet, CopyExport, DeleteSelected, OpenDocx, OpenSettings, PasteSnip, QuitApp,
    RetryOcr, SelectNext, SelectPrev, StartDraw, ToggleFormat, UploadImage,
};
use crate::cache::MediaCache;
use crate::doc::{CopyKind, DocStatus};
use crate::ocr::EngineStatus;
use crate::preview::{document_preview_with_dpr, raster_dpr, DocDerived};
use crate::state::AppState;

#[derive(Clone)]
pub(crate) enum View {
    Library,
    Draw,
    Settings,
}

pub(crate) struct PreviewPane {
    pub(crate) vscroll: ScrollHandle,
    hscrolls: HashMap<String, ScrollHandle>,
    pub(crate) sel: Rc<RefCell<PreviewSel>>,
    pub(crate) thumb: Rc<RefCell<Option<ScrollThumbDrag>>>,
    pub(crate) hover: bool,
    pub(crate) hscroll_hover: Rc<RefCell<HashSet<String>>>,
    /// Last measured preview-column width. Hover notify must not treat
    /// a 0-width first layout as a different eqno mode.
    pub(crate) pane_w: f32,
    doc: Option<Uuid>,
    pub(crate) bar_pending: bool,
}

impl PreviewPane {
    fn new() -> Self {
        Self {
            vscroll: ScrollHandle::new(),
            hscrolls: HashMap::new(),
            sel: Rc::new(RefCell::new(PreviewSel::default())),
            thumb: Rc::new(RefCell::new(None)),
            hover: false,
            hscroll_hover: Rc::new(RefCell::new(HashSet::new())),
            pane_w: 0.0,
            doc: None,
            bar_pending: false,
        }
    }

    pub(crate) fn reset_for(&mut self, doc_id: Uuid) {
        if self.doc == Some(doc_id) {
            return;
        }
        self.vscroll.set_offset(point(px(0.), px(0.)));
        self.hscrolls.clear();
        self.thumb.borrow_mut().take();
        self.hover = false;
        self.hscroll_hover.borrow_mut().clear();
        self.pane_w = 0.0;
        self.doc = Some(doc_id);
        self.bar_pending = true;
        self.sel.borrow_mut().clear();
    }

    pub(crate) fn hscroll_handle(&mut self, key: &str) -> ScrollHandle {
        self.hscrolls.entry(key.to_string()).or_default().clone()
    }

    pub(crate) fn hscroll_bounds_ready(&self) -> bool {
        if self.hscrolls.is_empty() {
            return true;
        }
        self.hscrolls
            .values()
            .any(|h| f32::from(h.bounds().size.width) > 1.0)
    }
}

pub struct MainWindow {
    pub(crate) state: Entity<AppState>,
    focus: FocusHandle,
    pub(crate) snip_list_focus: FocusHandle,
    pub(crate) search: Entity<SearchField>,
    pub(crate) media: Rc<RefCell<MediaCache>>,
    pub(crate) derived: Option<DocDerived>,
    derived_busy: bool,
    gc_scheduled: bool,
    pub(crate) thumb_keep: Rc<RefCell<Vec<Uuid>>>,
    pub(crate) thumb_need: Rc<RefCell<Vec<Uuid>>>,
    pub(crate) view: View,
    pub(crate) board: DrawBoard,
    pub(crate) settings: Entity<SettingsPane>,
    pub(crate) copied: Option<(Uuid, CopyKind)>,
    copied_epoch: u64,
    pub(crate) orig_hover: bool,
    orig_zoomed: bool,
    zoom_doc: Option<Uuid>,
    pub(crate) preview: PreviewPane,
    /// (pointer x at drag start, sidebar width at drag start).
    pub(crate) sidebar_drag: Option<(f32, f32)>,
    pub(crate) history: HistoryPane,
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
        let settings = cx.new(|_| SettingsPane::new());
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        let win_w: f32 = window.bounds().size.width.into();
        let pinned = state.read(cx).prefs.sidebar_pinned_collapsed;
        let history = HistoryPane::new(win_w, pinned);
        cx.observe_window_bounds(window, |this, window, cx| {
            let now_w: f32 = window.bounds().size.width.into();
            let pinned = this.state.read(cx).prefs.sidebar_pinned_collapsed;
            if this.history.fold.on_resize(now_w, pinned) {
                cx.notify();
            }
        })
        .detach();
        let this = cx.entity();
        cx.intercept_keystrokes(move |event, _, cx| {
            let settings = this.read(cx).settings.clone();
            let Some(id) = settings.read(cx).listening() else {
                return;
            };
            cx.stop_propagation();
            let key = event.keystroke.key.as_str();
            if key == "escape" {
                settings.update(cx, |pane, cx| {
                    pane.set_listen(None);
                    cx.notify();
                });
                return;
            }
            if matches!(
                key,
                "control" | "shift" | "alt" | "platform" | "fn" | "function"
            ) {
                return;
            }
            let chord = event.keystroke.unparse();
            let state = this.read(cx).state.clone();
            state.update(cx, |s, cx| {
                let _ = s.bind_shortcut(id, chord, cx);
            });
            settings.update(cx, |pane, cx| {
                pane.set_listen(None);
                cx.notify();
            });
        })
        .detach();
        Self {
            state,
            focus,
            snip_list_focus,
            search,
            media: Rc::new(RefCell::new(MediaCache::new())),
            derived: None,
            derived_busy: false,
            gc_scheduled: false,
            thumb_keep: Rc::new(RefCell::new(Vec::new())),
            thumb_need: Rc::new(RefCell::new(Vec::new())),
            view: View::Library,
            board: DrawBoard::new(),
            settings,
            copied: None,
            copied_epoch: 0,
            orig_hover: false,
            orig_zoomed: false,
            zoom_doc: None,
            preview: PreviewPane::new(),
            sidebar_drag: None,
            history,
        }
    }

    fn upload(&mut self, _: &UploadImage, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
        self.state.update(cx, |state, cx| state.request_upload(cx));
    }

    fn paste_snip(&mut self, _: &PasteSnip, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
        self.state.update(cx, |state, cx| state.request_paste(cx));
    }

    fn start_draw(&mut self, _: &StartDraw, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_draw(cx);
    }

    fn toggle_draw(&mut self, cx: &mut Context<Self>) {
        if matches!(self.view, View::Draw) {
            self.dismiss_sheet(cx);
            return;
        }
        self.unzoom();
        self.view = View::Draw;
        cx.notify();
    }

    fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.unzoom();
        self.view = if matches!(self.view, View::Settings) {
            self.settings.update(cx, |pane, _| pane.dismiss_listen());
            View::Library
        } else {
            self.settings.update(cx, |pane, _| pane.reset_scroll());
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
            self.settings.update(cx, |pane, _| pane.dismiss_listen());
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

    pub(crate) fn toggle_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) {
        let pin = self.history.fold.toggle(window.bounds().size.width.into());
        self.state.update(cx, |s, cx| {
            s.update_prefs(cx, |p| p.sidebar_pinned_collapsed = pin);
        });
        cx.notify();
    }

    fn current_sidebar_width(&self, has_docs: bool, cx: &App) -> f32 {
        self.history
            .fold
            .width(has_docs, self.state.read(cx).prefs.sidebar_width)
    }

    pub(crate) fn zoom_original(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.orig_zoomed = true;
        self.orig_hover = false;
        self.zoom_doc = Some(id);
        cx.notify();
    }

    fn copy(&mut self, _: &CopyExport, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.preview.sel.borrow().selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            return;
        }
        self.state.update(cx, |state, cx| state.copy_selected(cx));
    }

    pub(crate) fn flash_copied(&mut self, doc_id: Uuid, kind: CopyKind, cx: &mut Context<Self>) {
        self.copied = Some((doc_id, kind));
        self.copied_epoch = self.copied_epoch.wrapping_add(1);
        let epoch = self.copied_epoch;
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(1200)).await;
            this.update(cx, |this, cx| {
                if this.copied_epoch == epoch {
                    this.copied = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn open_docx(&mut self, _: &OpenDocx, _: &mut Window, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.open_docx_selected(cx));
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

    pub(crate) fn full(&self, id: Uuid) -> Option<Arc<RenderImage>> {
        self.media.borrow().full(id)
    }

    pub(crate) fn math_image(&self, svg: &str, cx: &mut App) -> Arc<Image> {
        self.media.borrow_mut().math_image(svg, cx)
    }

    fn schedule_media_gc(&mut self, cx: &mut Context<Self>) {
        if self.gc_scheduled {
            return;
        }
        self.gc_scheduled = true;
        let entity = cx.entity();
        cx.defer(move |cx| {
            entity.update(cx, |this, cx| {
                this.gc_scheduled = false;
                let (keep_thumbs, keep_fulls, pin) = {
                    let state = this.state.read(cx);
                    let keep_fulls = state.gpu_full_ids();
                    let pin = state.selected();
                    let kept = this.thumb_keep.borrow().clone();
                    let keep_thumbs = if kept.is_empty() {
                        state.visible_ids().to_vec()
                    } else {
                        kept
                    };
                    (keep_thumbs, keep_fulls, pin)
                };
                let mut media = this.media.borrow_mut();
                media.retain_thumbs(keep_thumbs.into_iter(), cx);
                media.retain_fulls(keep_fulls.into_iter(), pin, cx);
                media.trim_math(cx);
            });
        });
    }

    fn ensure_selected_full(&self, cx: &App) {
        let state = self.state.read(cx);
        let Some(doc) = state.selected_doc() else {
            return;
        };
        let Some(pixels) = doc.image.pixels() else {
            return;
        };
        self.media.borrow_mut().ensure_full(doc.id, pixels);
    }

    fn schedule_derived(&mut self, window: &Window, cx: &mut Context<Self>) {
        let dpr = raster_dpr(window.scale_factor());
        let selected = {
            let state = self.state.read(cx);
            state.selected_doc().map(|doc| {
                (
                    doc.id,
                    doc.revision,
                    matches!(doc.status, DocStatus::Ready) && doc.blocks_loaded,
                    state.prefs.clone(),
                )
            })
        };
        let Some((id, revision, ready, prefs)) = selected else {
            self.derived = None;
            return;
        };
        if self
            .derived
            .as_ref()
            .is_some_and(|d| d.id != id || d.revision != revision)
        {
            self.derived = None;
        }
        if !ready {
            return;
        }
        if self
            .derived
            .as_ref()
            .is_some_and(|d| d.matches(id, revision, dpr, &prefs))
        {
            return;
        }
        if self.derived_busy {
            return;
        }
        let Some(blocks) = self
            .state
            .read(cx)
            .library
            .get(id)
            .map(|d| d.blocks.clone())
        else {
            return;
        };
        self.derived_busy = true;
        cx.spawn(async move |this, cx| {
            let built = cx
                .background_spawn(async move {
                    let preview = document_preview_with_dpr(&blocks, dpr);
                    let rows = crate::export::visible_copy_rows(&crate::export::copy_rows(
                        &blocks, &prefs,
                    ));
                    DocDerived {
                        id,
                        revision,
                        dpr,
                        inline_delim: prefs.inline_delim,
                        block_delim: prefs.block_delim,
                        preview,
                        copy_rows: rows,
                    }
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.derived_busy = false;
                this.derived = Some(built);
                cx.notify();
            }) {
                eprintln!("{}: derived preview: {err}", crate::identity::APP_SLUG);
            }
        })
        .detach();
    }
}

impl Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl gpui::Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let need = std::mem::take(&mut *self.thumb_need.borrow_mut());
        for id in need {
            self.state.update(cx, |s, cx| s.request_thumb(id, cx));
        }
        let (status_kind, status_label, capturing, has_docs, n_docs) = {
            self.ensure_selected_full(cx);
            self.schedule_derived(window, cx);
            self.schedule_media_gc(cx);
            let state = self.state.read(cx);
            if self.orig_zoomed && self.zoom_doc != state.selected() {
                self.orig_zoomed = false;
                self.orig_hover = false;
                self.zoom_doc = None;
            }
            let (status_kind, status_label) = chrome(state);
            let status_label = if !matches!(
                status_kind,
                theme::StatusKind::Busy | theme::StatusKind::Error
            ) && self.preview.sel.borrow().selected_text().is_some()
            {
                "Ctrl+C to copy".into()
            } else {
                status_label
            };
            (
                status_kind,
                status_label,
                state.is_capturing(),
                !state.is_empty(),
                state.library.visible_docs().count(),
            )
        };
        let history = self.render_history(n_docs, cx);
        let copy_pane_w = {
            let sidebar = self.current_sidebar_width(has_docs, cx);
            let win_w: f32 = window.bounds().size.width.into();
            (win_w - sidebar - 32.).max(112.)
        };
        let detail = self.render_detail(capturing, copy_pane_w, cx);
        let zoom = self.render_orig_zoom(cx);
        let view = self.view.clone();
        let orig_zoomed = self.orig_zoomed;
        let (has_selected, can_open_docx) = {
            let state = self.state.read(cx);
            let has_selected = state.selected().is_some();
            let can_open_docx = state
                .selected_doc()
                .is_some_and(|doc| matches!(doc.status, DocStatus::Ready) && doc.blocks_loaded);
            (has_selected, can_open_docx)
        };
        div()
            .id("main")
            .track_focus(&self.focus)
            .key_context(crate::identity::APP_SLUG)
            .on_action(cx.listener(Self::capture))
            .on_action(cx.listener(Self::upload))
            .on_action(cx.listener(Self::paste_snip))
            .on_action(cx.listener(Self::start_draw))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::close_sheet))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::open_docx))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::delete_selected))
            .on_action(cx.listener(Self::toggle_format))
            .on_action(cx.listener(Self::retry))
            .on_action(cx.listener(Self::quit))
            .cursor(CursorStyle::Arrow)
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, window, cx| {
                // Sidebar resize drag: started by the "sidebar-resize" strip.
                let Some((start_x, start_w)) = this.sidebar_drag else {
                    return;
                };
                let win_w: f32 = window.bounds().size.width.into();
                let max = (win_w - 360.).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                let w = (start_w + f32::from(ev.position.x) - start_x).clamp(SIDEBAR_MIN, max);
                this.state.update(cx, |s, cx| {
                    if (s.prefs.sidebar_width - w).abs() > 0.5 {
                        s.prefs.sidebar_width = w;
                        cx.notify();
                    }
                });
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.sidebar_drag.take().is_some() {
                        this.state.update(cx, |s, _| s.persist_prefs());
                    }
                }),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::BG))
            .text_color(rgb(theme::TEXT))
            .child(ThumbDragCatcher::new(
                [
                    self.preview.thumb.clone(),
                    self.settings.read(cx).scroll_thumb(),
                    self.history.thumb.clone(),
                ],
                cx.entity_id(),
            ))
            .child(self.render_topbar(capturing, has_selected, can_open_docx, cx))
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
                        let state = self.state.clone();
                        d.flex_col().child(
                            self.settings
                                .update(cx, |pane, cx| pane.view(state, window, cx)),
                        )
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
            .child(self.tool_btn(
                "tool-draw",
                IconKind::Draw,
                "Create snip from drawing  Ctrl+D",
                matches!(view, View::Draw),
                !capturing,
                {
                    let entity = cx.entity();
                    move |_, cx| {
                        entity.update(cx, |this, cx| this.toggle_draw(cx));
                    }
                },
            ))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(rgb(theme::TRACK_OFF)))
            .child(self.tool_btn(
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
                                this.settings.update(cx, |pane, _| pane.dismiss_listen());
                                View::Library
                            } else {
                                this.settings.update(cx, |pane, _| pane.reset_scroll());
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
        EngineStatus::Ready => (theme::StatusKind::Ready, "Ready when you are".into()),
    }
}
