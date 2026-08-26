use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    px, size, App, AppContext, ClipboardItem, Context, Focusable, Timer, TitlebarOptions, Window,
    WindowDecorations, WindowHandle, WindowKind, WindowOptions,
};
use image::RgbaImage;
use uuid::Uuid;

use crate::capture;
use crate::desktop::DesktopCmd;
use crate::doc::{DocStatus, Document, ExportFmt};
use crate::identity::APP_SLUG;
use crate::ocr::Engine;
use crate::prefs::{Prefs, WindowCloseAction};
use crate::ui::{MainWindow, Overlay};

enum Capture {
    Idle,
    Grabbing,
    Overlay,
    Failed(String),
}

pub struct AppState {
    pub documents: Vec<Document>,
    pub selected: Option<Uuid>,
    export_fmt: ExportFmt,
    pub prefs: Prefs,
    engine: Arc<Engine>,
    capture: Capture,
    pub main_window: Option<WindowHandle<MainWindow>>,
    overlay_window: Option<WindowHandle<Overlay>>,
    /// Set when capture minimized the main window; OCR/cancel clears it.
    hidden_for_capture: bool,
    /// Successful snip: leave Settings/Draw when the result is shown.
    reveal_on_main: bool,
}

impl AppState {
    pub fn new() -> Self {
        let prefs = Prefs::load();
        Self {
            documents: Vec::new(),
            selected: None,
            export_fmt: prefs.default_fmt,
            prefs,
            engine: Engine::load(),
            capture: Capture::Idle,
            main_window: None,
            overlay_window: None,
            hidden_for_capture: false,
            reveal_on_main: false,
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

    /// Close-button policy. Ctrl+Q / tray Quit still call `cx.quit()` directly.
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

    pub fn export_fmt(&self) -> ExportFmt {
        self.export_fmt
    }

    pub fn set_format(&mut self, fmt: ExportFmt, cx: &mut Context<Self>) {
        self.export_fmt = fmt;
        cx.notify();
    }

    pub fn is_capturing(&self) -> bool {
        matches!(self.capture, Capture::Grabbing | Capture::Overlay)
    }

    pub fn capture_error(&self) -> Option<&str> {
        match &self.capture {
            Capture::Failed(err) => Some(err.as_str()),
            _ => None,
        }
    }

    pub fn engine_status(&self) -> crate::ocr::EngineStatus {
        self.engine.status()
    }

    pub fn selected_doc(&self) -> Option<&Document> {
        let id = self.selected?;
        self.documents.iter().find(|d| d.id == id)
    }

    pub fn selected_index(&self) -> Option<usize> {
        let id = self.selected?;
        self.documents.iter().position(|d| d.id == id)
    }

    pub fn request_capture(&mut self, cx: &mut Context<Self>) {
        if matches!(self.capture, Capture::Grabbing) {
            return;
        }
        // A stuck overlay otherwise eats the desktop and this early-return
        // (plus the global hotkey) can never recover.
        if matches!(self.capture, Capture::Overlay) {
            self.abort_overlay_session(cx);
        }
        self.capture = Capture::Grabbing;
        cx.notify();
        self.dismiss_main_sheet(cx);

        if self.prefs.hide_on_capture {
            self.hide_main(cx);
        }

        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(400)).await;
            let grabbed = cx.background_spawn(async { capture::grab_primary() }).await;
            if let Err(err) = this.update(cx, |this, cx| match grabbed {
                Ok(grab) => this.open_overlay(grab, cx),
                Err(err) => {
                    this.capture = Capture::Failed(err.to_string());
                    this.restore_after_hide(cx);
                    cx.notify();
                }
            }) {
                eprintln!("{APP_SLUG}: capture task: {err}");
            }
        })
        .detach();
    }

    fn open_overlay(&mut self, grab: capture::Grab, cx: &mut Context<Self>) {
        if self.overlay_is_reusable(cx) {
            self.reuse_overlay(grab, cx);
            return;
        }
        self.overlay_window = None;
        self.create_overlay(grab, cx);
    }

    fn overlay_is_reusable(&self, cx: &mut Context<Self>) -> bool {
        self.overlay_window
            .is_some_and(|handle| handle.update(cx, |_, _, _| ()).is_ok())
    }

    /// Linux keeps one GPUI overlay (unmap/map). Reset it instead of opening
    /// a second X11 window that never receives input.
    fn reuse_overlay(&mut self, grab: capture::Grab, cx: &mut Context<Self>) {
        let Some(handle) = self.overlay_window else {
            return;
        };
        if handle
            .update(cx, |view, window, cx| {
                view.reset(grab, window, cx);
            })
            .is_err()
        {
            self.overlay_window = None;
            self.capture = Capture::Failed("overlay window lost".into());
            self.restore_after_hide(cx);
            cx.notify();
            return;
        }
        self.begin_overlay_session(cx);
    }

    fn create_overlay(&mut self, grab: capture::Grab, cx: &mut Context<Self>) {
        let w = grab.image.width() as f32;
        let h = grab.image.height() as f32;
        let bounds = cx
            .primary_display()
            .map(|d| d.bounds())
            .unwrap_or_else(|| gpui::Bounds {
                origin: gpui::point(px(0.), px(0.)),
                size: size(px(w), px(h)),
            });

        let state = cx.entity();
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(crate::desktop::overlay_window_bounds(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(crate::identity::OVERLAY_TITLE.into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                // Borderless: a server titlebar would shrink the client below
                // the freeze-frame and GNOME would scale the shot to fit.
                window_decorations: Some(WindowDecorations::Client),
                kind: WindowKind::Normal,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                focus: true,
                show: true,
                app_id: Some(crate::identity::APP_ID.into()),
                window_background: gpui::WindowBackgroundAppearance::Opaque,
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Overlay::new(state, grab, window, cx)),
        );
        match result {
            Ok(handle) => {
                self.overlay_window = Some(handle);
                self.begin_overlay_session(cx);
            }
            Err(err) => {
                self.capture = Capture::Failed(format!("overlay: {err}"));
                self.restore_after_hide(cx);
                cx.notify();
            }
        }
    }

    fn begin_overlay_session(&mut self, cx: &mut Context<Self>) {
        self.capture = Capture::Overlay;
        crate::desktop::arm_overlay();
        self.schedule_overlay_raises(cx);
    }

    /// Park (Linux) or destroy (Win/mac) a live overlay so a new snip can start.
    fn abort_overlay_session(&mut self, cx: &mut Context<Self>) {
        if crate::desktop::overlay_keeps_window() {
            crate::desktop::park_overlay();
        } else {
            self.force_close_overlay(cx);
        }
        self.capture = Capture::Idle;
    }

    fn force_close_overlay(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.overlay_window.take() {
            if let Err(err) = handle.update(cx, |_, window, _| {
                window.remove_window();
            }) {
                eprintln!("{APP_SLUG}: close overlay: {err}");
            }
        }
    }

    /// Overlay already called `remove_window` on Win/mac. Linux parks the
    /// same GPUI window (unmap) so the next snip does not create a second
    /// X11 window that never receives input.
    fn end_overlay_session(&mut self, cx: &mut Context<Self>) {
        self.capture = Capture::Idle;
        if crate::desktop::overlay_keeps_window() {
            cx.defer(|_| {
                crate::desktop::park_overlay();
            });
        } else {
            self.overlay_window = None;
        }
    }

    pub fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        self.reveal_on_main = false;
        self.end_overlay_session(cx);
        self.restore_after_hide(cx);
        cx.notify();
    }

    pub fn finish_capture(&mut self, crop: RgbaImage, cx: &mut Context<Self>) {
        self.reveal_on_main = true;
        self.end_overlay_session(cx);
        self.ingest_image(crop, cx);
        // Stay minimized until OCR finishes (or fails). Cancel still
        // restores immediately via cancel_capture.
    }

    pub fn ingest_image(&mut self, image: RgbaImage, cx: &mut Context<Self>) {
        let doc = Document::pending(Arc::new(image));
        let id = doc.id;
        self.documents.insert(0, doc);
        self.selected = Some(id);
        self.start_ocr(id, cx);
        cx.notify();
    }

    pub fn request_upload(&mut self, cx: &mut Context<Self>) {
        if self.is_capturing() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
        else {
            return;
        };
        match image::open(&path) {
            Ok(dynimg) => self.ingest_image(dynimg.to_rgba8(), cx),
            Err(err) => {
                self.capture = Capture::Failed(format!("open image: {err}"));
                cx.notify();
            }
        }
    }

    fn hide_main(&mut self, cx: &mut Context<Self>) {
        self.hidden_for_capture = true;
        let handle = self.main_window;
        // Never `handle.update` the main window here: in-app Snip / Ctrl+Shift+S
        // run while that window is already on GPUI's update stack (`take()`),
        // so a nested update fails with "window not found" and the window
        // stays mapped.
        cx.defer(move |cx| {
            if let Some(handle) = handle {
                if let Err(err) = handle.update(cx, |_, window, _| {
                    window.minimize_window();
                }) {
                    eprintln!("{APP_SLUG}: minimize main: {err}");
                }
            }
        });
    }

    fn schedule_overlay_raises(&self, cx: &mut Context<Self>) {
        let handle = self.overlay_window;
        cx.defer(move |cx| {
            raise_overlay_once(handle, cx);
        });
        cx.spawn(async move |this, cx| {
            for delay in [50u64, 100, 200, 400, 800] {
                Timer::after(Duration::from_millis(delay)).await;
                let keep_going = this.update(cx, |this, _cx| {
                    if !matches!(this.capture, Capture::Overlay) {
                        return false;
                    }
                    // Retries only restack via EWMH. handle.update here races
                    // the X11 client RefCell and can drop overlay input.
                    crate::desktop::raise_overlay();
                    true
                });
                if !matches!(keep_going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn dismiss_main_sheet(&self, cx: &mut Context<Self>) {
        let handle = self.main_window;
        cx.defer(move |cx| {
            if let Some(handle) = handle {
                if let Err(err) = handle.update(cx, |view, _, cx| {
                    view.dismiss_sheet(cx);
                }) {
                    eprintln!("{APP_SLUG}: dismiss sheet: {err}");
                }
            }
        });
    }

    fn restore_after_hide(&mut self, cx: &mut Context<Self>) {
        if self.hidden_for_capture {
            self.restore_main(cx);
        }
    }

    fn restore_main(&mut self, cx: &mut Context<Self>) {
        self.hidden_for_capture = false;
        let handle = self.main_window;
        cx.defer(move |cx| {
            if let Some(handle) = handle {
                activate_window(handle, cx);
            }
        });
    }

    pub fn start_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(doc) = self.documents.iter().find(|d| d.id == id) else {
            return;
        };
        let image = doc.image.clone();
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.recognize(&image) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                    match result {
                        Ok(blocks) => {
                            doc.blocks = blocks;
                            doc.status = DocStatus::Ready;
                        }
                        Err(err) => {
                            doc.status = DocStatus::Failed(err.to_string());
                        }
                    }
                }
                if this.prefs.autocopy
                    && this
                        .selected_doc()
                        .is_some_and(|d| matches!(d.status, DocStatus::Ready))
                {
                    this.copy_selected(cx);
                }
                this.restore_after_hide(cx);
                if this.reveal_on_main {
                    this.reveal_on_main = false;
                    this.dismiss_main_sheet(cx);
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: ocr task: {err}");
            }
        })
        .detach();
    }

    pub fn retry_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected {
            if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                doc.status = DocStatus::Recognizing;
                doc.blocks.clear();
            }
            self.start_ocr(id, cx);
            cx.notify();
        }
    }

    pub fn copy_selected(&self, cx: &mut App) {
        let Some(doc) = self.selected_doc() else {
            return;
        };
        let text = doc.export(self.export_fmt, &self.prefs);
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn select_delta(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.documents.is_empty() {
            return;
        }
        let idx = self.selected_index().unwrap_or(0) as isize;
        let next = (idx + delta).clamp(0, self.documents.len() as isize - 1) as usize;
        self.selected = Some(self.documents[next].id);
        cx.notify();
    }

    pub fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        self.documents.retain(|d| d.id != id);
        self.selected = self.documents.first().map(|d| d.id);
        cx.notify();
    }

    pub fn toggle_format(&mut self, cx: &mut Context<Self>) {
        self.export_fmt = self.export_fmt.toggle();
        cx.notify();
    }

    pub fn select(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.selected = Some(id);
        cx.notify();
    }

    fn handle_desktop(&mut self, cmd: DesktopCmd, cx: &mut Context<Self>) {
        match cmd {
            DesktopCmd::Capture => self.request_capture(cx),
            DesktopCmd::Show => self.restore_main(cx),
            DesktopCmd::Quit => cx.quit(),
        }
    }
}

fn activate_window<V: 'static>(handle: WindowHandle<V>, cx: &mut App) {
    if let Err(err) = handle.update(cx, |_, window, _| {
        window.activate_window();
    }) {
        eprintln!("{APP_SLUG}: activate window: {err}");
    }
}

fn raise_overlay_once(handle: Option<WindowHandle<Overlay>>, cx: &mut App) {
    crate::desktop::raise_overlay();
    let Some(handle) = handle else {
        return;
    };
    if let Err(err) = handle.update(cx, |view, window, cx| {
        window.focus(&view.focus_handle(cx));
        crate::desktop::focus_native_overlay(window);
    }) {
        eprintln!("{APP_SLUG}: raise overlay: {err}");
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

pub fn bind_keys(cx: &mut App) {
    use crate::actions::*;
    use gpui::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-s", Capture, None),
        KeyBinding::new("ctrl-n", Capture, None),
        KeyBinding::new("ctrl-o", UploadImage, None),
        KeyBinding::new("ctrl-d", StartDraw, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("escape", CloseSheet, None),
        KeyBinding::new("ctrl-shift-c", CopyExport, None),
        KeyBinding::new("ctrl-c", CopyExport, None),
        KeyBinding::new("down", SelectNext, None),
        KeyBinding::new("j", SelectNext, None),
        KeyBinding::new("up", SelectPrev, None),
        KeyBinding::new("k", SelectPrev, None),
        KeyBinding::new("delete", DeleteSelected, None),
        KeyBinding::new("backspace", DeleteSelected, None),
        KeyBinding::new("ctrl-l", ToggleFormat, None),
        KeyBinding::new("enter", ConfirmOverlay, Some("Overlay")),
        KeyBinding::new("escape", CancelOverlay, Some("Overlay")),
        KeyBinding::new("ctrl-r", RetryOcr, None),
        KeyBinding::new("ctrl-q", QuitApp, None),
    ]);
}
