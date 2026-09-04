use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, AppContext, Context, Window, WindowHandle};
use uuid::Uuid;

use crate::desktop::DesktopCmd;
use crate::doc::{Document, ExportFmt};
use crate::identity::APP_SLUG;
use crate::keymap::{self, AssignError, ShortcutId};
use crate::library::Library;
use crate::ocr::Engine;
use crate::ocr_queue::OcrQueue;
use crate::prefs::{Prefs, PrefsWriter, WindowCloseAction};
use crate::store::{Store, StoreWriter};
use crate::ui::MainWindow;

mod capture;
mod document_actions;
mod ingest;
mod intake;
mod library_io;
mod orig_export;
mod runtime;
mod search;
mod session;
mod toast;

pub use intake::{classify_image_paths, IntakeBatch, IntakeCounts, IntakeWork};

use runtime::{DocumentRuntime, FileIntakeSession};
use session::{CaptureSession, SearchFilter};
use toast::ToastState;

pub use crate::library::DatePreset;

const OCR_IDLE_RELEASE: Duration = Duration::from_secs(15 * 60);
const HIDDEN_MEDIA_RELEASE: Duration = Duration::from_secs(5 * 60);

enum PersistenceState {
    Loading,
    Ready {
        store: Arc<Store>,
        writer: StoreWriter,
    },
    RamOnly,
    ShuttingDown,
}

enum MainWindowState {
    Closed,
    Opening,
    Open {
        handle: WindowHandle<MainWindow>,
        visible: bool,
    },
}

impl MainWindowState {
    fn handle(&self) -> Option<WindowHandle<MainWindow>> {
        match self {
            Self::Open { handle, .. } => Some(*handle),
            Self::Closed | Self::Opening => None,
        }
    }

    fn is_visible(&self) -> bool {
        matches!(self, Self::Open { visible: true, .. })
    }

    fn is_opening(&self) -> bool {
        matches!(self, Self::Opening)
    }

    fn set_visible(&mut self, visible: bool) {
        if let Self::Open { visible: slot, .. } = self {
            *slot = visible;
        }
    }
}

pub struct AppState {
    library: Library,
    export_fmt: ExportFmt,
    pub prefs: Prefs,
    prefs_writer: Option<PrefsWriter>,
    autostart_pending: bool,
    capture_after_bootstrap: bool,
    engine: Arc<Engine>,
    engine_release_task: Option<gpui::Task<()>>,
    hidden_media_release_task: Option<gpui::Task<()>>,
    persistence: PersistenceState,
    search: SearchFilter,
    ocr: OcrQueue,
    file_intake: FileIntakeSession,
    documents: DocumentRuntime,
    capture: CaptureSession,
    toast: ToastState,
    orig_copy_flash: Option<Uuid>,
    orig_copy_flash_gen: u64,
    intake: Option<IntakeBatch>,
    intake_gen: u64,
    main_window: MainWindowState,
}

impl AppState {
    pub fn new(prefs: Prefs) -> Self {
        let prefs_writer = PrefsWriter::start()
            .map_err(|err| eprintln!("{APP_SLUG}: {err:#}"))
            .ok();
        Self {
            library: Library::new(),
            export_fmt: prefs.default_fmt,
            prefs,
            prefs_writer,
            autostart_pending: false,
            capture_after_bootstrap: false,
            engine: Engine::load(),
            engine_release_task: None,
            hidden_media_release_task: None,
            persistence: PersistenceState::Loading,
            search: SearchFilter::new(),
            ocr: OcrQueue::new(),
            file_intake: FileIntakeSession::new(),
            documents: DocumentRuntime::new(),
            capture: CaptureSession::new(),
            toast: ToastState::new(),
            orig_copy_flash: None,
            orig_copy_flash_gen: 0,
            intake: None,
            intake_gen: 0,
            main_window: MainWindowState::Closed,
        }
    }

    pub fn bootstrap_store(&mut self, hydrate_selected: bool, cx: &mut Context<Self>) {
        if !matches!(self.persistence, PersistenceState::Loading) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_spawn(async move {
                    let store = Arc::new(Store::open_default()?);
                    let items = store.list()?;
                    let (writer, events) = StoreWriter::start(store.clone())?;
                    anyhow::Ok((store, writer, events, items))
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                match opened {
                    Ok((store, writer, events, items)) => {
                        this.library = Library::from_list(items);
                        this.persistence = PersistenceState::Ready { store, writer };
                        this.pump_store_events(events, cx);
                        if hydrate_selected || this.main_window.is_visible() {
                            this.boot_selected(cx);
                        }
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: snip store unavailable (RAM-only): {err:#}");
                        this.persistence = PersistenceState::RamOnly;
                    }
                }
                if std::mem::take(&mut this.capture_after_bootstrap) {
                    this.request_capture(cx);
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: store bootstrap task: {err}");
            }
        })
        .detach();
    }

    pub fn is_bootstrapping(&self) -> bool {
        matches!(self.persistence, PersistenceState::Loading)
    }

    fn store(&self) -> Option<Arc<Store>> {
        match &self.persistence {
            PersistenceState::Ready { store, .. } => Some(store.clone()),
            PersistenceState::Loading
            | PersistenceState::RamOnly
            | PersistenceState::ShuttingDown => None,
        }
    }

    fn store_writer(&self) -> Option<StoreWriter> {
        match &self.persistence {
            PersistenceState::Ready { writer, .. } => Some(writer.clone()),
            PersistenceState::Loading
            | PersistenceState::RamOnly
            | PersistenceState::ShuttingDown => None,
        }
    }

    pub fn persist_prefs(&mut self) {
        if let Some(writer) = &self.prefs_writer {
            if let Err(err) = writer.save(self.prefs.clone()) {
                eprintln!("{APP_SLUG}: queue prefs: {err:#}");
            }
        } else if let Err(err) = self.prefs.save() {
            eprintln!("{APP_SLUG}: save prefs: {err:#}");
        }
    }

    pub fn update_prefs(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Prefs)) {
        f(&mut self.prefs);
        self.persist_prefs();
        cx.notify();
    }

    pub fn set_launch_at_startup(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.autostart_pending || self.prefs.launch_at_startup == enabled {
            return;
        }
        self.autostart_pending = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { crate::autostart::apply(enabled) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.autostart_pending = false;
                match result {
                    Ok(()) => {
                        this.prefs.launch_at_startup = enabled;
                        this.persist_prefs();
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: autostart: {err:#}");
                        this.flash_error("Couldn't change launch at startup", cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn bind_shortcut(
        &mut self,
        id: ShortcutId,
        chord: String,
        cx: &mut Context<Self>,
    ) -> Result<Option<ShortcutId>, AssignError> {
        let stolen = keymap::assign(&mut self.prefs.shortcuts, id, chord).map_err(|err| {
            if keymap::spec(id).global() && err == AssignError::Invalid {
                self.flash_error("Global shortcuts need a supported key and modifier", cx);
            }
            err
        })?;
        self.commit_shortcuts(cx);
        Ok(stolen)
    }

    pub fn restore_shortcut(
        &mut self,
        id: ShortcutId,
        cx: &mut Context<Self>,
    ) -> Result<Option<ShortcutId>, AssignError> {
        let stolen = keymap::restore(&mut self.prefs.shortcuts, id)?;
        self.commit_shortcuts(cx);
        Ok(stolen)
    }

    pub fn unbind_shortcut(&mut self, id: ShortcutId, cx: &mut Context<Self>) {
        keymap::unbind(&mut self.prefs.shortcuts, id);
        self.commit_shortcuts(cx);
    }

    pub fn reset_shortcuts(&mut self, cx: &mut Context<Self>) {
        keymap::reset(&mut self.prefs.shortcuts);
        self.commit_shortcuts(cx);
    }

    fn commit_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.persist_prefs();
        keymap::apply(cx, &self.prefs.shortcuts);
        crate::desktop::rebind_globals(&self.prefs.shortcuts);
        cx.notify();
    }

    /// Title-bar / WM close. On Linux this runs while GPUI's X11 client still
    /// holds its `RefCell`; calling [`App::quit`] here re-enters `with_common`
    /// and aborts (`RefCell already borrowed`). Close the window now, then stop
    /// the loop after this event handler returns.
    pub fn handle_main_close(
        &mut self,
        action: WindowCloseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match action {
            WindowCloseAction::Minimize => {
                self.minimize_main(window, cx);
                false
            }
            WindowCloseAction::Quit => {
                self.request_quit(cx);
                false
            }
        }
    }

    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        if matches!(self.persistence, PersistenceState::ShuttingDown) {
            return;
        }
        let writer = self.store_writer();
        let prefs_writer = self.prefs_writer.take();
        self.persistence = PersistenceState::ShuttingDown;
        cx.spawn(async move |_, cx| {
            let result = match writer {
                Some(writer) => cx.background_spawn(async move { writer.shutdown() }).await,
                None => {
                    // Leave the current window callback before stopping GPUI.
                    cx.background_spawn(async {}).await;
                    Ok(())
                }
            };
            if let Err(err) = result {
                eprintln!("{APP_SLUG}: store shutdown: {err:#}");
            }
            if let Some(writer) = prefs_writer {
                if let Err(err) = cx.background_spawn(async move { writer.shutdown() }).await {
                    eprintln!("{APP_SLUG}: prefs shutdown: {err:#}");
                }
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    }

    pub fn engine_status(&self) -> crate::ocr::EngineStatus {
        self.engine.status()
    }

    pub fn model_info(&self) -> crate::ocr::ModelInfo {
        self.engine.model_info()
    }

    pub fn start_model_inspect(&self, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move {
                engine.inspect_file_metadata();
            })
            .await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn schedule_engine_release(&mut self, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        let generation = engine.usage_generation();
        self.engine_release_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(OCR_IDLE_RELEASE).await;
            let released = cx
                .background_spawn(async move { engine.release_if_idle(generation) })
                .await;
            if released {
                eprintln!("{APP_SLUG}: released idle OCR sessions");
            }
        }));
    }

    pub(super) fn schedule_hidden_media_release(&mut self, cx: &mut Context<Self>) {
        self.hidden_media_release_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HIDDEN_MEDIA_RELEASE).await;
            let _ = this.update(cx, |this, cx| {
                if this.main_window.is_visible() {
                    return;
                }
                this.library.release_persisted_images();
                let handle = this.main_window.handle();
                cx.defer(move |cx| {
                    if let Some(handle) = handle {
                        let _ = handle.update(cx, |window, _, cx| {
                            window.release_hidden_media(cx);
                        });
                    }
                });
                cx.notify();
            });
        }));
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.library.selected()
    }

    pub fn selected_doc(&self) -> Option<&Document> {
        self.library.selected_doc()
    }

    pub fn doc(&self, id: Uuid) -> Option<&Document> {
        self.library.get(id)
    }

    pub fn visible_ids(&self) -> &[Uuid] {
        self.library.visible_ids()
    }

    pub fn visible_snapshot(&self) -> Arc<[Uuid]> {
        self.library.visible_snapshot()
    }

    pub fn visible_len(&self) -> usize {
        self.library.visible_len()
    }

    pub fn snip_count(&self) -> usize {
        self.library.len()
    }

    pub fn date_preset(&self) -> DatePreset {
        self.library.date_preset()
    }

    pub fn is_empty(&self) -> bool {
        self.library.is_empty()
    }

    pub fn gpu_full_ids(&self) -> Vec<Uuid> {
        self.library.gpu_full_ids()
    }

    pub(crate) fn has_main_window(&self) -> bool {
        self.main_window.handle().is_some()
    }

    pub(crate) fn open_main(&mut self, handle: WindowHandle<MainWindow>) {
        self.main_window = MainWindowState::Open {
            handle,
            visible: true,
        };
    }

    pub fn select_delta(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.visible_ids().is_empty() {
            return;
        }
        let idx = self
            .selected()
            .and_then(|id| self.visible_ids().iter().position(|x| *x == id))
            .unwrap_or(0) as isize;
        let next = (idx + delta).clamp(0, self.visible_ids().len() as isize - 1) as usize;
        let id = self.visible_ids()[next];
        self.select(id, cx);
    }

    pub fn toggle_format(&mut self, cx: &mut Context<Self>) {
        self.export_fmt = self.export_fmt.toggle();
        cx.notify();
    }

    pub fn select(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.library.select(id) {
            return;
        }
        self.ensure_detail(id, cx);
        cx.notify();
    }

    pub fn boot_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.library.selected() {
            self.ensure_detail(id, cx);
        }
    }

    fn handle_desktop(&mut self, cmd: DesktopCmd, cx: &mut Context<Self>) {
        match cmd {
            DesktopCmd::Capture => self.request_capture(cx),
            DesktopCmd::Show => {
                if self.main_window.is_visible() {
                    self.hide_to_tray(cx);
                } else {
                    self.restore_main(cx);
                }
            }
            DesktopCmd::Reveal => self.restore_main(cx),
            DesktopCmd::Quit => self.request_quit(cx),
        }
    }
}

pub fn pump_desktop_events(state: gpui::Entity<AppState>, rx: Receiver<DesktopCmd>, cx: &mut App) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx| loop {
        let rx = rx.clone();
        let cmd = cx
            .background_spawn(async move { rx.lock().ok()?.recv().ok() })
            .await;
        let Some(cmd) = cmd else {
            break;
        };
        state.update(cx, |state, cx| state.handle_desktop(cmd, cx));
    })
    .detach();
}
