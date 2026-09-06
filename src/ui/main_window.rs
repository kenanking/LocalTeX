use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, ClipboardItem, Context, CursorStyle, DragMoveEvent, Entity, ExternalPaths, FocusHandle,
    Focusable, MouseButton, MouseMoveEvent, ScrollHandle, Window, div, point, prelude::*, px, rgb,
};
use uuid::Uuid;

use super::chrome::{chrome, workspace_height};
use super::draw::DrawBoard;
use super::history::{HistoryPane, SIDEBAR_MAX, SIDEBAR_MIN};
use super::intake::{
    INTAKE_COMPLETE_HOLD, INTAKE_PLATEN_OUT, INTAKE_REJECT_HOLD, IntakeKind, IntakePhase,
    IntakePresentation, intake_feedback_duration, render_intake_overlay, slots_from_batch,
};
use super::media::WindowMedia;
use super::orig_view::{OrigStrip, OrigView, copy_reserve, max_strip_h};
use super::scroll::{ScrollThumbDrag, ThumbDragCatcher};
use super::search_field::SearchField;
use super::selectable::PreviewSel;
use super::settings::SettingsPane;
use super::source_editor::SourceEditor;
use super::source_panel::SourceBinding;
use super::theme;
use super::window_drag::{WindowDrag, WindowDragCatcher};
use crate::actions::{
    Capture, CloseSheet, CloseWindow, CopyExport, DeleteSelected, OpenDocx, OpenSettings,
    PasteSnip, QuitApp, RetryOcr, SelectNext, SelectPrev, StartDraw, ToggleFormat, ToggleSource,
    UploadImage,
};
use crate::doc::DocStatus;
use crate::export::CopyKind;
use crate::state::{AppState, IntakeCounts, classify_image_paths};

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
    pub(crate) media: WindowMedia,
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
    pub(crate) source_panel: SourceBinding,
    drop_hover: Option<IntakeCounts>,
    pub(crate) intake: Option<IntakePresentation>,
}

impl MainWindow {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let snip_list_focus = cx.focus_handle().tab_stop(true);
        let orig_focus = cx.focus_handle();
        let draw_focus = cx.focus_handle().tab_stop(true);
        window.focus(&snip_list_focus, cx);
        #[cfg(target_os = "linux")]
        window.set_client_inset(px(0.));
        let state_for_close = state.clone();
        window.on_window_should_close(cx, move |window, cx| {
            let action = state_for_close.read(cx).prefs.close_action;
            state_for_close.update(cx, |state, cx| state.handle_main_close(action, window, cx))
        });
        cx.observe(&state, |this, _, cx| {
            if !this.state.read(cx).main_window_visible() {
                this.settings.update(cx, |settings, _| settings.hide());
            }
            this.sync_intake(cx);
            if this.intake.is_none() {
                this.ensure_selected_full(cx);
                this.schedule_derived_from_app(cx);
            }
            this.schedule_media_gc(cx);
            if this.orig.open && this.state.read(cx).selected().is_none() {
                this.orig.close();
            }
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
        let settings = cx.new(SettingsPane::new);
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        let win_w: f32 = window.bounds().size.width.into();
        let pinned = state.read(cx).prefs.sidebar_pinned_collapsed;
        let history = HistoryPane::new(win_w, pinned);
        cx.observe_window_bounds(window, |this, window, cx| {
            let scale = window.scale_factor();
            if (scale - this.media.last_scale).abs() > f32::EPSILON {
                this.media.last_scale = scale;
                this.schedule_derived(window, cx);
            }
            let now_w: f32 = window.bounds().size.width.into();
            let pinned = this.state.read(cx).prefs.sidebar_pinned_collapsed;
            if this.history.fold.on_resize(now_w, pinned) {
                cx.notify();
            }
        })
        .detach();
        super::settings::intercept_recording(settings.clone(), state.clone(), cx);
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
            media: WindowMedia::new(window.scale_factor()),
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
            source_panel: SourceBinding::new(source),
            drop_hover: None,
            intake: None,
        };
        this.sync_intake(cx);
        if this.intake.is_none() {
            this.schedule_derived(window, cx);
        }
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

    fn on_file_drag_move(
        &mut self,
        event: &DragMoveEvent<ExternalPaths>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !cx.has_active_drag() {
            if self.drop_hover.take().is_some() {
                cx.notify();
            }
            return;
        }
        let blocked = self.state.read(cx).is_capturing() || self.state.read(cx).is_bootstrapping();
        if blocked {
            if self.drop_hover.take().is_some() {
                cx.notify();
            }
            return;
        }
        let Some(paths) = event.dragged_item().downcast_ref::<ExternalPaths>() else {
            return;
        };
        let counts = classify_image_paths(paths.paths().iter().cloned()).counts();
        if self.drop_hover != Some(counts) {
            self.drop_hover = Some(counts);
            cx.notify();
        }
    }

    fn on_file_drop(&mut self, paths: &ExternalPaths, _: &mut Window, cx: &mut Context<Self>) {
        self.drop_hover = None;
        let blocked = self.state.read(cx).is_capturing() || self.state.read(cx).is_bootstrapping();
        if blocked {
            cx.notify();
            return;
        }
        let paths = paths.paths().to_vec();
        self.state
            .update(cx, |state, cx| state.offer_files(paths, cx));
    }

    fn sync_intake(&mut self, cx: &mut Context<Self>) {
        let batch = self.state.read(cx).intake().cloned();
        let Some(batch) = batch else {
            if self
                .intake
                .as_ref()
                .is_some_and(|intake| intake.phase == IntakePhase::Running)
            {
                self.intake = None;
                self.release_intake_polaroids(cx);
            }
            return;
        };

        let is_new = self
            .intake
            .as_ref()
            .is_none_or(|intake| intake.generation != batch.generation);
        let sync = if is_new {
            let (presentation, sync) = IntakePresentation::new(&batch);
            self.intake = Some(presentation);
            sync
        } else {
            self.intake
                .as_mut()
                .expect("matching intake presentation exists")
                .sync(&batch)
        };

        if is_new && batch.items.is_empty() {
            self.schedule_intake_reject(batch.generation, cx);
        }
        if sync.complete {
            if let Some(intake) = &mut self.intake {
                intake.phase = IntakePhase::Complete;
            }
            self.acknowledge_intake(batch.generation, cx);
            self.schedule_intake_complete(batch.generation, cx);
        } else {
            for feedback in sync.feedback {
                self.schedule_intake_feedback(
                    batch.generation,
                    feedback.key,
                    feedback.succeeded,
                    cx,
                );
            }
        }

        self.refresh_intake_media(cx);
    }

    pub(crate) fn refresh_intake_media(&self, cx: &mut Context<Self>) {
        if let Some(intake) = &self.intake {
            let paper_jobs = intake.paper_jobs();
            self.ensure_intake_polaroids(&paper_jobs, cx);
        }
    }

    fn acknowledge_intake(&self, generation: u64, cx: &mut Context<Self>) {
        let state = self.state.clone();
        cx.defer(move |cx| {
            state.update(cx, |state, cx| state.acknowledge_intake(generation, cx));
        });
    }

    fn schedule_intake_reject(&mut self, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INTAKE_REJECT_HOLD).await;
            let _ = this.update(cx, |this, cx| {
                let Some(intake) = this.intake.as_mut().filter(|intake| {
                    intake.generation == generation && intake.phase == IntakePhase::Running
                }) else {
                    return;
                };
                intake.phase = IntakePhase::Fading;
                this.acknowledge_intake(generation, cx);
                cx.notify();
            });
            cx.background_executor().timer(INTAKE_PLATEN_OUT).await;
            let _ = this.update(cx, |this, cx| {
                if this.intake.as_ref().is_some_and(|intake| {
                    intake.generation == generation && intake.phase == IntakePhase::Fading
                }) {
                    this.intake = None;
                    this.release_intake_polaroids(cx);
                    this.resume_deferred_media(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn schedule_intake_feedback(
        &mut self,
        generation: u64,
        key: u64,
        succeeded: bool,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(intake_feedback_duration(succeeded))
                .await;
            let _ = this.update(cx, |this, cx| {
                let mut complete = false;
                let changed = if let Some(intake) = this.intake.as_mut().filter(|intake| {
                    intake.generation == generation && intake.phase == IntakePhase::Running
                }) {
                    let changed = intake.finish_feedback_and_promote(key);
                    if changed && intake.ready_to_complete() {
                        intake.phase = IntakePhase::Complete;
                        complete = true;
                    }
                    changed
                } else {
                    false
                };
                if changed {
                    if complete {
                        this.acknowledge_intake(generation, cx);
                        this.schedule_intake_complete(generation, cx);
                    } else {
                        this.refresh_intake_media(cx);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn schedule_intake_complete(&mut self, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INTAKE_COMPLETE_HOLD).await;
            let _ = this.update(cx, |this, cx| {
                let Some(intake) = this.intake.as_mut().filter(|intake| {
                    intake.generation == generation && intake.phase == IntakePhase::Complete
                }) else {
                    return;
                };
                intake.phase = IntakePhase::Fading;
                cx.notify();
            });
            cx.background_executor().timer(INTAKE_PLATEN_OUT).await;
            let _ = this.update(cx, |this, cx| {
                if this.intake.as_ref().is_some_and(|intake| {
                    intake.generation == generation && intake.phase == IntakePhase::Fading
                }) {
                    this.intake = None;
                    this.release_intake_polaroids(cx);
                    this.resume_deferred_media(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn resume_deferred_media(&mut self, cx: &mut Context<Self>) {
        self.ensure_selected_full(cx);
        self.schedule_derived_from_app(cx);
        self.schedule_media_gc(cx);
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

    pub(crate) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if matches!(self.view, View::Settings) {
            let focus = self.settings.read(cx).focus.clone();
            window.focus(&focus, cx);
        } else {
            window.focus(&self.snip_list_focus, cx);
        }
        cx.notify();
    }

    fn close_sheet(&mut self, _: &CloseSheet, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.read(cx).wipe_confirmation_open() {
            self.settings
                .update(cx, |pane, cx| pane.dismiss_wipe_confirmation(cx));
            return;
        }
        if self.source_panel.open {
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
        window.focus(&self.snip_list_focus, cx);
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

    fn retry(&mut self, _: &RetryOcr, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.snip_list_focus, cx);
        self.state.update(cx, |state, cx| state.retry_selected(cx));
    }

    fn toggle_source(&mut self, _: &ToggleSource, window: &mut Window, cx: &mut Context<Self>) {
        self.set_source_open(!self.source_panel.open, window, cx);
    }

    fn quit(&mut self, _: &QuitApp, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.request_quit(cx));
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        let action = self.state.read(cx).prefs.close_action;
        self.state.update(cx, |state, cx| {
            state.handle_main_close(action, window, cx);
        });
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
                    self.source_panel.split = pct.clamp(0.28, 0.72);
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
        let (status_kind, status_label, capturing, has_docs, n_docs) = {
            let state = self.state.read(cx);
            let (status_kind, status_label) = self
                .intake
                .as_ref()
                .map(IntakePresentation::status)
                .unwrap_or_else(|| chrome(state));
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
            )
        };
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
        let view = self.view.clone();
        let (has_selected, can_open_docx) = {
            let state = self.state.read(cx);
            let has_selected = state.selected().is_some();
            let can_open_docx = state.selected_doc().is_some_and(|doc| {
                doc.has_ready_blocks() && !doc.source_pending && doc.source_error.is_none()
            });
            (has_selected, can_open_docx)
        };
        let accepting = self
            .state
            .read(cx)
            .intake()
            .is_some_and(crate::state::IntakeBatch::is_accepting);
        let hover = if cx.has_active_drag() && !accepting {
            self.drop_hover
        } else {
            None
        };
        let intake = match (hover, self.intake.as_ref()) {
            (Some(counts), _) => {
                Some(render_intake_overlay(IntakeKind::Hover, counts, None, &[]).into_any_element())
            }
            (None, Some(presentation)) => {
                let slots = slots_from_batch(presentation, |id| self.intake_polaroid(id));
                Some(
                    render_intake_overlay(
                        IntakeKind::Flash,
                        presentation.counts,
                        Some(presentation),
                        &slots,
                    )
                    .into_any_element(),
                )
            }
            (None, None) => None,
        };
        let wipe_confirmation = self.settings.read(cx).wipe_confirmation_open().then(|| {
            super::settings::wipe_confirmation(
                self.state.clone(),
                self.settings.clone(),
                self.state.read(cx).snip_count(),
                self.settings.read(cx).wipe_cancel_focus(),
                window,
            )
        });
        div()
            .id("main")
            .track_focus(&self.focus)
            .key_context(crate::identity::APP_SLUG)
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "tab"
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                    && !event.keystroke.modifiers.platform
                {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                }
            })
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
            .on_drag_move::<ExternalPaths>(cx.listener(Self::on_file_drag_move))
            .on_drop(cx.listener(Self::on_file_drop))
            .can_drop(|value, _, _| value.downcast_ref::<ExternalPaths>().is_some())
            .cursor(CursorStyle::Arrow)
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _, cx| {
                if this.drop_hover.is_some() && !cx.has_active_drag() {
                    this.drop_hover = None;
                    cx.notify();
                }
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
            .relative()
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
            .when_some(intake, |d, overlay| d.child(overlay))
            .children(wipe_confirmation)
            .map(|content| super::chrome::client_frame(content, window))
    }
}
