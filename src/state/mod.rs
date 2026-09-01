use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use gpui::{App, AppContext, Context, Window, WindowHandle};
use uuid::Uuid;

use crate::desktop::DesktopCmd;
use crate::doc::{Document, ExportFmt};
use crate::identity::APP_SLUG;
use crate::keymap::{self, AssignError, ShortcutId};
use crate::library::Library;
use crate::ocr::Engine;
use crate::prefs::{Prefs, WindowCloseAction};
use crate::store::{Store, StoreWriter};
use crate::ui::MainWindow;

mod capture;
mod ingest;
mod orig_export;
mod search;
mod session;

pub use ingest::IngestSource;

use session::{CaptureSession, IngestPump, SearchFilter};

pub use crate::library::DatePreset;

enum PersistenceState {
    Loading,
    Ready {
        store: Arc<Store>,
        writer: StoreWriter,
    },
    RamOnly,
}

pub struct AppState {
    pub library: Library,
    export_fmt: ExportFmt,
    pub prefs: Prefs,
    engine: Arc<Engine>,
    persistence: PersistenceState,
    search: SearchFilter,
    ingest: IngestPump,
    capture: CaptureSession,
    orig_copy_flash: Option<Uuid>,
    orig_copy_flash_gen: u64,
    pub main_window: Option<WindowHandle<MainWindow>>,
}

impl AppState {
    pub fn new(prefs: Prefs) -> Self {
        Self {
            library: Library::new(),
            export_fmt: prefs.default_fmt,
            prefs,
            engine: Engine::load(),
            persistence: PersistenceState::Loading,
            search: SearchFilter::new(),
            ingest: IngestPump::new(),
            capture: CaptureSession::new(),
            orig_copy_flash: None,
            orig_copy_flash_gen: 0,
            main_window: None,
        }
    }

    pub fn bootstrap_store(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.persistence, PersistenceState::Loading) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_spawn(async move {
                    let store = Arc::new(Store::open_default()?);
                    let items = store.list()?;
                    let writer = StoreWriter::start(store.clone())?;
                    anyhow::Ok((store, writer, items))
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                match opened {
                    Ok((store, writer, items)) => {
                        this.library = Library::from_list(items);
                        this.persistence = PersistenceState::Ready { store, writer };
                        this.boot_selected(cx);
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: snip store unavailable (RAM-only): {err:#}");
                        this.persistence = PersistenceState::RamOnly;
                    }
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
            PersistenceState::Loading | PersistenceState::RamOnly => None,
        }
    }

    fn store_writer(&self) -> Option<StoreWriter> {
        match &self.persistence {
            PersistenceState::Ready { writer, .. } => Some(writer.clone()),
            PersistenceState::Loading | PersistenceState::RamOnly => None,
        }
    }

    pub fn persist_prefs(&mut self) {
        self.prefs.save();
    }

    pub fn update_prefs(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Prefs)) {
        f(&mut self.prefs);
        self.persist_prefs();
        cx.notify();
    }

    pub fn bind_shortcut(
        &mut self,
        id: ShortcutId,
        chord: String,
        cx: &mut Context<Self>,
    ) -> Result<Option<ShortcutId>, AssignError> {
        let stolen = keymap::assign(&mut self.prefs.shortcuts, id, chord)?;
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
    pub fn handle_main_close(action: WindowCloseAction, window: &mut Window, cx: &mut App) -> bool {
        match action {
            WindowCloseAction::Minimize => {
                crate::desktop::iconify_main_window();
                window.minimize_window();
                false
            }
            WindowCloseAction::Quit => {
                cx.spawn(async move |cx| {
                    cx.background_spawn(async {}).await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
                true
            }
        }
    }

    pub fn engine_status(&self) -> crate::ocr::EngineStatus {
        self.engine.status()
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.library.selected()
    }

    pub fn selected_doc(&self) -> Option<&Document> {
        self.library.selected_doc()
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

    pub fn date_preset(&self) -> DatePreset {
        self.library.date_preset()
    }

    pub fn is_empty(&self) -> bool {
        self.library.is_empty()
    }

    pub fn gpu_full_ids(&self) -> Vec<Uuid> {
        self.library.gpu_full_ids()
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
        self.library.select(id);
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
                let active = self
                    .main_window
                    .and_then(|handle| handle.is_active(cx))
                    .unwrap_or(false);
                if active {
                    self.iconify_main(cx);
                } else {
                    self.restore_main(cx);
                }
            }
            DesktopCmd::Quit => cx.quit(),
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

pub fn bind_keys(cx: &mut App, over: &crate::keymap::Overrides) {
    crate::keymap::apply(cx, over);
}
