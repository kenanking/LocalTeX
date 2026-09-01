use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, rgb, App, ClipboardItem, Context, CursorStyle, Entity, FocusHandle,
    Focusable, Image, MouseButton, MouseMoveEvent, RenderImage, ScrollHandle, Window,
};
use uuid::Uuid;

use super::chrome::{chrome, workspace_height};
use super::draw::DrawBoard;
use super::history::{HistoryPane, SIDEBAR_MAX, SIDEBAR_MIN};
use super::orig_view::{copy_reserve, max_strip_h, OrigStrip, OrigView};
use super::scroll::{ScrollThumbDrag, ThumbDragCatcher};
use super::search_field::SearchField;
use super::selectable::PreviewSel;
use super::settings::SettingsPane;
use super::source_editor::SourceEditor;
use super::theme;
use super::window_drag::{WindowDrag, WindowDragCatcher};
use crate::actions::{
    Capture, CloseSheet, CloseWindow, CopyExport, DeleteSelected, OpenDocx, OpenSettings,
    PasteSnip, QuitApp, RetryOcr, SelectNext, SelectPrev, StartDraw, ToggleFormat, ToggleSource,
    UploadImage,
};
use crate::cache::{thumb_retain_ids, MediaCache};
use crate::doc::DocStatus;
use crate::export::CopyKind;
use crate::preview::{
    derived_copy_rows, document_preview_with_dpr, raster_dpr, should_spawn_derived, DocDerived,
};
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
    pub(crate) orig_focus: FocusHandle,
    pub(crate) draw_focus: FocusHandle,
    pub(crate) search: Entity<SearchField>,
    pub(crate) media: Rc<RefCell<MediaCache>>,
    pub(crate) derived: Option<DocDerived>,
    derived_busy: bool,
    gc_scheduled: bool,
    last_scale: f32,
    pub(crate) thumb_keep: Rc<RefCell<Vec<Uuid>>>,
    pub(crate) view: View,
    pub(crate) board: DrawBoard,
    pub(crate) settings: Entity<SettingsPane>,
    pub(crate) copied: Option<(Uuid, CopyKind)>,
    copied_epoch: u64,
    pub(crate) orig: OrigView,
    pub(crate) orig_strip: OrigStrip,
    pub(crate) preview: PreviewPane,
    pub(crate) window_drag: Option<WindowDrag>,
    pub(crate) caption_pending_move: Rc<Cell<bool>>,
    pub(crate) history: HistoryPane,
    pub(crate) source_open: bool,
    pub(crate) source: Entity<SourceEditor>,
    pub(crate) source_bound: Option<Uuid>,
    pub(crate) source_last: String,
    pub(crate) source_split: f32,
    source_flush_task: Option<gpui::Task<()>>,
    pub(crate) source_lang: &'static str,
    thumb_inflight: Rc<RefCell<HashSet<Uuid>>>,
}

impl MainWindow {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let snip_list_focus = cx.focus_handle();
        let orig_focus = cx.focus_handle();
        let draw_focus = cx.focus_handle();
        window.focus(&snip_list_focus, cx);
        #[cfg(target_os = "linux")]
        window.set_client_inset(px(0.));
        let state_for_close = state.clone();
        window.on_window_should_close(cx, move |window, cx| {
            let action = state_for_close.read(cx).prefs.close_action;
            AppState::handle_main_close(action, window, cx)
        });
        cx.observe(&state, |this, _, cx| {
            this.ensure_selected_full(cx);
            this.schedule_derived_from_app(cx);
            this.schedule_media_gc(cx);
            let entity = cx.entity();
            cx.defer(move |cx| {
                entity.update(cx, |this, cx| this.sync_source_for_selection(cx));
            });
            cx.notify();
        })
        .detach();
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
            let scale = window.scale_factor();
            if (scale - this.last_scale).abs() > f32::EPSILON {
                this.last_scale = scale;
                this.schedule_derived(window, cx);
            }
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
            if matches!(key, "backspace" | "delete") {
                let state = this.read(cx).state.clone();
                state.update(cx, |s, cx| {
                    s.unbind_shortcut(id, cx);
                });
                settings.update(cx, |pane, cx| {
                    pane.set_listen(None);
                    cx.notify();
                });
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
        let source = cx.new(SourceEditor::new);
        cx.observe(&source, |this, _, cx| {
            this.on_source_edit(cx);
        })
        .detach();
        let mut this = Self {
            state,
            focus,
            snip_list_focus,
            orig_focus,
            draw_focus,
            search,
            media: Rc::new(RefCell::new(MediaCache::new())),
            derived: None,
            derived_busy: false,
            gc_scheduled: false,
            last_scale: window.scale_factor(),
            thumb_keep: Rc::new(RefCell::new(Vec::new())),
            view: View::Library,
            board: DrawBoard::new(),
            settings,
            copied: None,
            copied_epoch: 0,
            orig: OrigView::new(),
            orig_strip: OrigStrip::new(),
            preview: PreviewPane::new(),
            window_drag: None,
            caption_pending_move: Rc::new(Cell::new(false)),
            history,
            source_open: false,
            source,
            source_bound: None,
            source_last: String::new(),
            source_split: 0.5,
            source_flush_task: None,
            source_lang: "LaTeX",
            thumb_inflight: Rc::new(RefCell::new(HashSet::new())),
        };
        this.schedule_derived(window, cx);
        this
    }

    fn upload(&mut self, _: &UploadImage, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
        self.state.update(cx, |state, cx| state.request_upload(cx));
    }

    fn paste_snip(&mut self, _: &PasteSnip, _: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_sheet(cx);
        self.state.update(cx, |state, cx| state.request_paste(cx));
    }

    fn start_draw(&mut self, _: &StartDraw, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_draw(window, cx);
    }

    pub(crate) fn toggle_draw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.view, View::Draw) {
            self.dismiss_sheet(cx);
            window.focus(&self.snip_list_focus, cx);
            return;
        }
        self.unzoom();
        if matches!(self.view, View::Settings) {
            self.settings.update(cx, |pane, _| pane.hide());
        }
        self.view = View::Draw;
        window.focus(&self.draw_focus, cx);
        cx.notify();
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_settings(window, cx);
    }

    pub(crate) fn toggle_settings(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.view, View::Draw) {
            self.board.leave_canvas();
            crate::desktop::set_os_cursor_visible(true);
        }
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

    fn close_sheet(&mut self, _: &CloseSheet, window: &mut Window, cx: &mut Context<Self>) {
        if self.source_open {
            self.set_source_open(false, window, cx);
            return;
        }
        if self.orig.open {
            self.unzoom();
            window.focus(&self.snip_list_focus, cx);
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
        self.board.leave_canvas();
        crate::desktop::set_os_cursor_visible(true);
        let was_zoom = self.orig.open;
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
        self.orig.close();
    }

    pub(crate) fn toggle_sidebar(&mut self, window: &Window, cx: &mut Context<Self>) {
        let pin = self
            .history
            .fold
            .toggle_pin(window.bounds().size.width.into());
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

    pub(crate) fn zoom_original(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.select(id, cx));
        self.orig.open_view();
        window.focus(&self.orig_focus, cx);
        cx.notify();
    }

    pub(crate) fn orig_pointer_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.orig.is_film_panning() {
            if let Some(id) = self.orig.end_film_pan() {
                self.state.update(cx, |s, cx| s.select(id, cx));
            }
            cx.notify();
            return;
        }
        if self.orig.is_image_panning() && self.orig.end_drag() {
            self.unzoom();
            window.focus(&self.snip_list_focus, cx);
            cx.notify();
        }
    }

    fn copy(&mut self, _: &CopyExport, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.preview.sel.borrow().selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            return;
        }
        let copied = self.state.update(cx, |state, cx| state.copy_selected(cx));
        if let Some((id, kind)) = copied {
            self.flash_copied(id, kind, cx);
        }
    }

    pub(crate) fn flash_copied(&mut self, doc_id: Uuid, kind: CopyKind, cx: &mut Context<Self>) {
        self.copied = Some((doc_id, kind));
        self.copied_epoch = self.copied_epoch.wrapping_add(1);
        let epoch = self.copied_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
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

    fn toggle_source(&mut self, _: &ToggleSource, window: &mut Window, cx: &mut Context<Self>) {
        self.set_source_open(!self.source_open, window, cx);
    }

    pub(crate) fn set_source_open(
        &mut self,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ready = self
            .state
            .read(cx)
            .selected_doc()
            .is_some_and(|d| matches!(d.status, DocStatus::Ready) && d.blocks_loaded);
        if on && !ready {
            return;
        }
        if on == self.source_open {
            if on {
                self.bind_source(cx);
                window.focus(&self.source.focus_handle(cx), cx);
            }
            return;
        }
        if on {
            self.source_open = true;
            self.bind_source(cx);
            window.focus(&self.source.focus_handle(cx), cx);
        } else {
            self.close_source(cx);
            window.focus(&self.snip_list_focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn close_source(&mut self, cx: &mut Context<Self>) {
        self.source_flush_task.take();
        if self.source_open {
            self.flush_source(cx);
        }
        self.source_open = false;
        self.source_bound = None;
    }

    pub(crate) fn bind_source(&mut self, cx: &mut Context<Self>) {
        let id = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                return;
            };
            doc.id
        };
        if self.source_bound == Some(id) {
            return;
        }
        if self.source_bound.is_some() {
            self.flush_source(cx);
        }
        let (prefs, blocks) = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                return;
            };
            (state.prefs.clone(), doc.blocks.clone())
        };
        let text = crate::source::blocks_to_source(&blocks, &prefs);
        self.source_lang = crate::source::editor_lang_label(crate::doc::snip_kind(&blocks), &text);
        self.source_last = text.clone();
        self.source_bound = Some(id);
        self.source.update(cx, |ed, cx| ed.set_text(text, cx));
    }

    fn sync_source_for_selection(&mut self, cx: &mut Context<Self>) {
        if !self.source_open {
            return;
        }
        let ready = self
            .state
            .read(cx)
            .selected_doc()
            .is_some_and(|d| matches!(d.status, DocStatus::Ready) && d.blocks_loaded);
        if !ready {
            self.close_source(cx);
            cx.notify();
            return;
        }
        self.bind_source(cx);
    }

    fn on_source_edit(&mut self, cx: &mut Context<Self>) {
        if !self.source_open {
            return;
        }
        self.source_flush_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(280))
                .await;
            this.update(cx, |this, cx| {
                this.flush_source(cx);
            })
            .ok();
        }));
    }

    fn flush_source(&mut self, cx: &mut Context<Self>) {
        if !self.source_open {
            return;
        }
        let text = self.source.read(cx).text();
        if text == self.source_last {
            return;
        }
        let prefs = self.state.read(cx).prefs.clone();
        let Ok(blocks) = crate::source::parse_source(&text, &prefs) else {
            return;
        };
        let Some(id) = self.source_bound else {
            return;
        };
        self.source_lang = crate::source::editor_lang_label(crate::doc::snip_kind(&blocks), &text);
        self.source_last = text;
        self.state
            .update(cx, |state, cx| state.apply_parsed_source(id, blocks, cx));
    }

    pub(crate) fn revert_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.source_bound else {
            return;
        };
        self.state.update(cx, |state, cx| state.revert_ocr(id, cx));
        self.source_bound = None;
        self.bind_source(cx);
        window.focus(&self.source.focus_handle(cx), cx);
        cx.notify();
    }

    fn quit(&mut self, _: &QuitApp, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        let action = self.state.read(cx).prefs.close_action;
        AppState::handle_main_close(action, window, cx);
    }

    pub(crate) fn full(&self, id: Uuid) -> Option<Arc<RenderImage>> {
        self.media.borrow().full(id)
    }

    pub(crate) fn math_image(&self, svg: &str, cx: &mut App) -> Arc<Image> {
        self.media.borrow_mut().math_image(svg, cx)
    }

    pub(crate) fn schedule_media_gc(&mut self, cx: &mut Context<Self>) {
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
                    let keep_thumbs = thumb_retain_ids(this.orig.open, &kept, state.visible_ids());
                    (keep_thumbs, keep_fulls, pin)
                };
                let mut media = this.media.borrow_mut();
                media.retain_thumbs(keep_thumbs.into_iter(), cx);
                media.retain_fulls(keep_fulls.into_iter(), pin, cx);
                media.trim_math(cx);
            });
        });
    }

    pub(crate) fn ensure_thumbs(&self, ids: &[Uuid], cx: &mut Context<Self>) {
        let mut request = Vec::new();
        let mut jobs = Vec::new();
        {
            let state = self.state.read(cx);
            let media = self.media.borrow();
            let mut inflight = self.thumb_inflight.borrow_mut();
            for &id in ids {
                if media.thumb(id).is_some() || inflight.contains(&id) {
                    continue;
                }
                let Some(doc) = state.library.get(id) else {
                    continue;
                };
                if matches!(doc.image, crate::doc::ImageSlot::Missing) {
                    continue;
                }
                if !doc.thumb_jpeg.is_empty() {
                    inflight.insert(id);
                    jobs.push((id, doc.thumb_jpeg.clone(), None));
                } else if let Some(px) = doc.image.pixels() {
                    inflight.insert(id);
                    jobs.push((id, Vec::new(), Some(px.clone())));
                } else {
                    request.push(id);
                }
            }
        }
        if !request.is_empty() {
            let entity = cx.entity();
            cx.defer(move |cx| {
                entity.update(cx, |this, cx| {
                    for id in request {
                        this.state.update(cx, |s, cx| s.request_thumb(id, cx));
                    }
                });
            });
        }
        for (id, jpeg, pixels) in jobs {
            cx.spawn(async move |this, cx| {
                let render = cx
                    .background_spawn(async move {
                        crate::cache::MediaCache::decode_thumb(&jpeg, pixels.as_deref())
                    })
                    .await;
                if let Err(err) = this.update(cx, |this, cx| {
                    this.thumb_inflight.borrow_mut().remove(&id);
                    if let Some(render) = render {
                        this.media.borrow_mut().put_thumb(id, render);
                        cx.notify();
                    }
                }) {
                    eprintln!("thumb decode: {err}");
                }
            })
            .detach();
        }
    }

    pub(crate) fn ensure_selected_full(&self, cx: &App) {
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
        self.last_scale = window.scale_factor();
        self.schedule_derived_from_app(cx);
    }

    fn schedule_derived_from_app(&mut self, cx: &mut Context<Self>) {
        let dpr = raster_dpr(self.last_scale);
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
        if !should_spawn_derived(
            ready,
            self.derived
                .as_ref()
                .is_some_and(|d| d.matches(id, revision, dpr, &prefs)),
            self.derived_busy,
        ) {
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
                    let rows = derived_copy_rows(&blocks, &prefs);
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
                let selected = this.state.read(cx).selected();
                this.derived = built.keep_if_selected(selected);
                this.schedule_derived_from_app(cx);
                cx.notify();
            }) {
                eprintln!("{}: derived preview: {err}", crate::identity::APP_SLUG);
            }
        })
        .detach();
    }

    pub(crate) fn apply_window_drag(
        &mut self,
        ev: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.window_drag {
            Some(WindowDrag::Strip { .. }) => {
                let workspace_h = workspace_height(window.viewport_size().height.into());
                let ready = self
                    .state
                    .read(cx)
                    .selected_doc()
                    .is_some_and(|d| matches!(d.status, DocStatus::Ready));
                let max_h = max_strip_h(workspace_h, copy_reserve(ready));
                let Some(drag) = self.window_drag.as_ref() else {
                    return;
                };
                let Some(next) = drag.strip_h_for(f32::from(ev.position.y), max_h) else {
                    return;
                };
                self.state.update(cx, |s, cx| {
                    if (s.prefs.orig_strip_h - next).abs() > 0.5 {
                        s.prefs.orig_strip_h = next;
                        cx.notify();
                    }
                });
            }
            Some(WindowDrag::Source {
                start_x,
                start_pct,
                work_w,
            }) => {
                if work_w > 1.0 {
                    let pct = start_pct + (f32::from(ev.position.x) - start_x) / work_w;
                    self.source_split = pct.clamp(0.28, 0.72);
                    cx.notify();
                }
            }
            Some(WindowDrag::Sidebar { start_x, start_w }) => {
                let win_w: f32 = window.bounds().size.width.into();
                let max = (win_w - 360.).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                let w = (start_w + f32::from(ev.position.x) - start_x).clamp(SIDEBAR_MIN, max);
                self.state.update(cx, |s, cx| {
                    if (s.prefs.sidebar_width - w).abs() > 0.5 {
                        s.prefs.sidebar_width = w;
                        cx.notify();
                    }
                });
            }
            None => {}
        }
    }

    pub(crate) fn end_window_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.window_drag.take() else {
            return;
        };
        if drag.is_strip() {
            cx.notify();
        }
        if drag.persist_on_end() {
            self.state.update(cx, |s, _| s.persist_prefs());
        }
    }
}

impl Focusable for MainWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl gpui::Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status_kind, status_label, capturing, has_docs, n_docs, close_orig) = {
            let state = self.state.read(cx);
            let close_orig = self.orig.open && state.selected().is_none();
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
                state.is_capturing() || state.is_bootstrapping(),
                !state.is_empty(),
                state.visible_len(),
                close_orig,
            )
        };
        if close_orig {
            let entity = cx.entity();
            cx.defer(move |cx| {
                entity.update(cx, |this, cx| {
                    if this.orig.open && this.state.read(cx).selected().is_none() {
                        this.orig.close();
                        cx.notify();
                    }
                });
            });
        }
        let history = self.render_history(n_docs, cx);
        let viewport = window.viewport_size();
        let copy_pane_w = {
            let sidebar = self.current_sidebar_width(has_docs, cx);
            let win_w: f32 = viewport.width.into();
            (win_w - sidebar - 32.).max(112.)
        };
        let workspace_h = workspace_height(viewport.height.into());
        let detail = self.render_detail(capturing, copy_pane_w, workspace_h, cx);
        let orig_open = self.orig.open;
        if orig_open && !self.orig_focus.is_focused(window) {
            cx.on_next_frame(window, |this, window, cx| {
                if this.orig.open && !this.orig_focus.is_focused(window) {
                    window.focus(&this.orig_focus, cx);
                }
            });
        }
        if matches!(self.view, View::Draw) && !self.draw_focus.is_focused(window) {
            cx.on_next_frame(window, |this, window, cx| {
                if matches!(this.view, View::Draw) && !this.draw_focus.is_focused(window) {
                    window.focus(&this.draw_focus, cx);
                }
            });
        }
        let view = self.view.clone();
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
            .on_action(cx.listener(Self::toggle_source))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::close_window))
            .cursor(CursorStyle::Arrow)
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _, cx| {
                if this.orig.has_pointer() && !ev.dragging() {
                    this.orig.pointer_move(
                        f32::from(ev.position.x),
                        f32::from(ev.position.y),
                        false,
                    );
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.orig_pointer_up(window, cx);
                    if this.board.is_gesturing() {
                        this.board.pointer_up();
                        cx.notify();
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
                    self.orig.film_thumb.clone(),
                ],
                cx.entity_id(),
            ))
            .child(WindowDragCatcher { view: cx.entity() })
            .child(self.render_topbar(capturing, has_selected, can_open_docx, window, cx))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .when(matches!(view, View::Library), |d| {
                        d.when(has_docs, |d| d.child(history))
                            .child(detail)
                            .when(orig_open, |d| {
                                d.child(self.render_orig_overlay(window, workspace_h, cx))
                            })
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
            .map(|content| super::chrome::client_frame(content, window))
    }
}
