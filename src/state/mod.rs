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
use crate::store::Store;
use crate::ui::MainWindow;

mod capture;
mod ingest;
mod search;
mod session;

use session::{CaptureSession, IngestPump, SearchFilter};

pub use crate::library::DatePreset;

pub struct AppState {
    pub library: Library,
    export_fmt: ExportFmt,
    pub prefs: Prefs,
    engine: Arc<Engine>,
    store: Option<Arc<Store>>,
    search: SearchFilter,
    ingest: IngestPump,
    capture: CaptureSession,
    pub main_window: Option<WindowHandle<MainWindow>>,
}

impl AppState {
    pub fn new(prefs: Prefs) -> Self {
        let store = match Store::open_default() {
            Ok(store) => Some(Arc::new(store)),
            Err(err) => {
                eprintln!("{APP_SLUG}: snip store unavailable (RAM-only): {err}");
                None
            }
        };
        let library = match store.as_ref() {
            Some(store) => match store.list() {
                Ok(items) => Library::from_list(items),
                Err(err) => {
                    eprintln!("{APP_SLUG}: list snips: {err}");
                    Library::new()
                }
            },
            None => Library::new(),
        };
        Self {
            library,
            export_fmt: prefs.default_fmt,
            prefs,
            engine: Engine::load(),
            store,
            search: SearchFilter::new(),
            ingest: IngestPump::new(),
            capture: CaptureSession::new(),
            main_window: None,
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
        crate::desktop::rebind_capture(
            keymap::effective(&self.prefs.shortcuts, ShortcutId::Capture).as_deref(),
        );
        cx.notify();
    }

    pub fn handle_main_close(action: WindowCloseAction, window: &mut Window, cx: &mut App) -> bool {
        match action {
            WindowCloseAction::Minimize => {
                window.minimize_window();
                false
            }
            WindowCloseAction::Quit => {
                cx.quit();
                true
            }
        }
    }

    pub fn set_format(&mut self, fmt: ExportFmt, cx: &mut Context<Self>) {
        self.export_fmt = fmt;
        cx.notify();
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
            DesktopCmd::Show => self.restore_main(cx),
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
        if state
            .update(cx, |state, cx| state.handle_desktop(cmd, cx))
            .is_err()
        {
            break;
        }
    })
    .detach();
}

pub fn bind_keys(cx: &mut App, over: &crate::keymap::Overrides) {
    crate::keymap::apply(cx, over);
}
