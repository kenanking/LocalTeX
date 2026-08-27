use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, AppContext, ClipboardItem, Context, Timer, Window, WindowHandle};
use image::RgbaImage;
use uuid::Uuid;

use crate::capture;
use crate::desktop::DesktopCmd;
use crate::doc::{DocStatus, Document, ExportFmt, ImageSlot};
use crate::identity::APP_SLUG;
use crate::ocr::Engine;
use crate::prefs::{Prefs, WindowCloseAction};
use crate::store::{self, CivilDate, DateRange, Store};
use crate::ui::MainWindow;

enum Capture {
    Idle,
    Grabbing,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatePreset {
    All,
    Today,
    Last7Days,
    Last30Days,
}

impl DatePreset {
    pub fn to_range(self, today: CivilDate) -> DateRange {
        match self {
            Self::All => DateRange::default(),
            Self::Today => DateRange::last_n_days(today, 1),
            Self::Last7Days => DateRange::last_n_days(today, 7),
            Self::Last30Days => DateRange::last_n_days(today, 30),
        }
    }
}

pub struct AppState {
    pub documents: Vec<Document>,
    pub selected: Option<Uuid>,
    /// Sidebar order after search + date filter (in-flight snips first).
    pub visible_ids: Vec<Uuid>,
    pub date_preset: DatePreset,
    export_fmt: ExportFmt,
    pub prefs: Prefs,
    engine: Arc<Engine>,
    store: Option<Arc<Store>>,
    search_query: String,
    search_gen: u64,
    /// Most-recent-last ids with `ImageSlot::Loaded` (max 3, including selected).
    loaded_lru: Vec<Uuid>,
    capture: Capture,
    pub main_window: Option<WindowHandle<MainWindow>>,
    /// Nested hide count: one increment per capture request, one decrement
    /// on that capture's cancel/error or its OCR finish. Tray Show zeros it.
    hide_depth: u32,
    /// Successful snip: leave Settings/Draw when the result is shown.
    reveal_on_main: bool,
}

impl AppState {
    pub fn new() -> Self {
        let prefs = Prefs::load();
        let store = match Store::open_default() {
            Ok(store) => Some(Arc::new(store)),
            Err(err) => {
                eprintln!("{APP_SLUG}: snip store unavailable (RAM-only): {err}");
                None
            }
        };
        let documents = match store.as_ref() {
            Some(store) => match store.list() {
                Ok(items) => items.into_iter().map(Document::from_list_item).collect(),
                Err(err) => {
                    eprintln!("{APP_SLUG}: list snips: {err}");
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        let selected = documents.first().map(|d| d.id);
        let visible_ids: Vec<Uuid> = documents.iter().map(|d| d.id).collect();
        Self {
            documents,
            selected,
            visible_ids,
            date_preset: DatePreset::All,
            export_fmt: prefs.default_fmt,
            prefs,
            engine: Engine::load(),
            store,
            search_query: String::new(),
            search_gen: 0,
            loaded_lru: Vec::new(),
            capture: Capture::Idle,
            main_window: None,
            hide_depth: 0,
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

    pub fn set_format(&mut self, fmt: ExportFmt, cx: &mut Context<Self>) {
        self.export_fmt = fmt;
        cx.notify();
    }

    pub fn is_capturing(&self) -> bool {
        matches!(self.capture, Capture::Grabbing)
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

    pub fn request_capture(&mut self, cx: &mut Context<Self>) {
        if matches!(self.capture, Capture::Grabbing) {
            return;
        }
        self.capture = Capture::Grabbing;
        cx.notify();
        self.dismiss_main_sheet(cx);

        let hide = self.prefs.hide_on_capture;
        if hide {
            self.hide_main(cx);
        }

        cx.spawn(async move |this, cx| {
            if hide {
                cx.background_spawn(async { crate::desktop::wait_until_iconified() })
                    .await;
            } else {
                // Sheet dismiss is deferred; a short settle keeps it out of the freeze.
                cx.background_spawn(async {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                })
                .await;
            }
            let picked = cx
                .background_spawn(async {
                    let shot = capture::grab_desktop()?;
                    crate::desktop::select_region(&shot)
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| match picked {
                Ok(Some(crop)) => this.finish_capture(crop, cx),
                Ok(None) => {
                    this.capture = Capture::Idle;
                    this.reveal_on_main = false;
                    this.restore_after_hide(cx);
                    cx.notify();
                }
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

    pub fn finish_capture(&mut self, crop: RgbaImage, cx: &mut Context<Self>) {
        self.reveal_on_main = true;
        self.capture = Capture::Idle;
        self.ingest_image(crop, cx);
    }

    pub fn ingest_image(&mut self, image: RgbaImage, cx: &mut Context<Self>) {
        let (w0, h0) = image.dimensions();
        let image = crate::imgutil::cap_megapixels(image);
        if image.dimensions() != (w0, h0) {
            eprintln!(
                "{APP_SLUG}: snip {w0}×{h0} downscaled to {}×{} (24 MP cap)",
                image.width(),
                image.height()
            );
        }
        let doc = Document::pending(Arc::new(image));
        let id = doc.id;
        self.documents.insert(0, doc);
        self.selected = Some(id);
        if !self.visible_ids.contains(&id) {
            self.visible_ids.insert(0, id);
        }
        self.touch_lru(id);
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
        self.hide_depth = self.hide_depth.saturating_add(1);
        // EWMH HIDDEN does not nest `handle.update` (in-app Snip runs while the
        // main window is already on GPUI's update stack).
        crate::desktop::iconify_main_window();
        let handle = self.main_window;
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
        if self.hide_depth == 0 {
            return;
        }
        self.hide_depth -= 1;
        if self.hide_depth == 0 {
            self.restore_main(cx);
        }
    }

    fn restore_main(&mut self, cx: &mut Context<Self>) {
        self.hide_depth = 0;
        crate::desktop::deiconify_main_window();
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
        let Some(image) = doc.image.pixels().cloned() else {
            return;
        };
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.recognize(image.as_ref()) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                    match result {
                        Ok(out) => {
                            doc.blocks = out.blocks;
                            doc.blocks_loaded = true;
                            doc.ocr = out.meta;
                            doc.status = DocStatus::Ready;
                            doc.refresh_first_line();
                        }
                        Err(err) => {
                            doc.status = DocStatus::Failed(err.to_string());
                        }
                    }
                }
                let ready = this
                    .documents
                    .iter()
                    .any(|d| d.id == id && matches!(d.status, DocStatus::Ready));
                if ready {
                    this.persist_ready(id, cx);
                    if this.prefs.autocopy && this.selected == Some(id) {
                        this.copy_selected(cx);
                    }
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
        let Some(id) = self.selected else {
            return;
        };
        let slot = self
            .documents
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.image.clone());
        match slot {
            None | Some(ImageSlot::Missing) => (),
            Some(ImageSlot::Loaded(_)) => {
                if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                    doc.status = DocStatus::Recognizing;
                    doc.blocks.clear();
                }
                self.start_ocr(id, cx);
                cx.notify();
            }
            Some(ImageSlot::OnDisk) => self.load_png_then_retry(id, cx),
        }
    }

    pub fn copy_selected(&self, cx: &mut App) {
        let Some(doc) = self.selected_doc() else {
            return;
        };
        let text = doc.primary_copy(self.export_fmt, &self.prefs);
        Self::write_clipboard(text, cx);
    }

    pub fn copy_text(text: String, cx: &mut App) {
        Self::write_clipboard(text, cx);
    }

    fn write_clipboard(text: String, cx: &mut App) {
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn select_delta(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.visible_ids.is_empty() {
            return;
        }
        let idx = self
            .selected
            .and_then(|id| self.visible_ids.iter().position(|x| *x == id))
            .unwrap_or(0) as isize;
        let next = (idx + delta).clamp(0, self.visible_ids.len() as isize - 1) as usize;
        self.select(self.visible_ids[next], cx);
    }

    pub fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        self.documents.retain(|d| d.id != id);
        self.visible_ids.retain(|x| *x != id);
        self.loaded_lru.retain(|x| *x != id);
        self.selected = self
            .visible_ids
            .first()
            .copied()
            .or_else(|| self.documents.first().map(|d| d.id));
        if let Some(store) = self.store.clone() {
            cx.background_spawn(async move {
                if let Err(err) = store.delete(id) {
                    eprintln!("{APP_SLUG}: delete snip: {err}");
                }
            })
            .detach();
        }
        if let Some(id) = self.selected {
            self.ensure_detail(id, cx);
        }
        cx.notify();
    }

    pub fn toggle_format(&mut self, cx: &mut Context<Self>) {
        self.export_fmt = self.export_fmt.toggle();
        cx.notify();
    }

    pub fn select(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.touch_lru(id);
        self.ensure_detail(id, cx);
        cx.notify();
    }

    pub fn set_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.search_query == query {
            return;
        }
        self.search_query = query;
        self.schedule_filter(cx);
    }

    pub fn set_date_preset(&mut self, preset: DatePreset, cx: &mut Context<Self>) {
        if self.date_preset == preset {
            return;
        }
        self.date_preset = preset;
        self.schedule_filter(cx);
    }

    pub fn gpu_full_ids(&self) -> Vec<Uuid> {
        let mut ids = self.loaded_lru.clone();
        if let Some(sel) = self.selected {
            if !ids.contains(&sel) {
                ids.push(sel);
            }
        }
        ids
    }

    pub fn boot_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected {
            self.ensure_detail(id, cx);
        }
    }

    pub fn visible_docs(&self) -> Vec<&Document> {
        self.visible_ids
            .iter()
            .filter_map(|id| self.documents.iter().find(|d| d.id == *id))
            .collect()
    }

    fn persist_ready(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let Some(doc) = self.documents.iter().find(|d| d.id == id).cloned() else {
            return;
        };
        if !matches!(doc.status, DocStatus::Ready) || doc.image.pixels().is_none() {
            return;
        }
        let already = doc.persisted;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if already {
                        store.update_ocr(&doc)?;
                        Ok::<Option<Vec<u8>>, anyhow::Error>(None)
                    } else {
                        Ok(Some(store.insert_ready(&doc)?))
                    }
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                match result {
                    Ok(Some(thumb)) => {
                        if let Some(d) = this.documents.iter_mut().find(|d| d.id == id) {
                            d.persisted = true;
                            d.thumb_jpeg = thumb;
                        }
                        this.schedule_filter(cx);
                    }
                    Ok(None) => {}
                    Err(err) => eprintln!("{APP_SLUG}: persist snip: {err}"),
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: persist task: {err}");
            }
        })
        .detach();
    }

    fn schedule_filter(&mut self, cx: &mut Context<Self>) {
        self.search_gen = self.search_gen.wrapping_add(1);
        let gen = self.search_gen;
        let query = self.search_query.clone();
        let range = self.date_preset.to_range(CivilDate::today_local());
        let store = self.store.clone();
        let inflight: Vec<(Uuid, std::time::SystemTime, String)> = self
            .documents
            .iter()
            .filter(|d| !d.persisted)
            .map(|d| {
                let blob = if matches!(d.status, DocStatus::Ready) {
                    Document::search_text_for_blocks(&d.blocks)
                } else {
                    d.first_line()
                };
                (d.id, d.created_at, blob)
            })
            .collect();
        let ram_only: Option<Vec<(Uuid, std::time::SystemTime, String)>> = if store.is_none() {
            Some(
                self.documents
                    .iter()
                    .filter(|d| d.persisted)
                    .map(|d| {
                        (
                            d.id,
                            d.created_at,
                            Document::search_text_for_blocks(&d.blocks),
                        )
                    })
                    .collect(),
            )
        } else {
            None
        };
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(120)).await;
            let ids = cx
                .background_spawn(async move {
                    let mut persisted = if let Some(store) = store {
                        store.query_ids(&query, range).unwrap_or_else(|err| {
                            eprintln!("{APP_SLUG}: search: {err}");
                            Vec::new()
                        })
                    } else {
                        ram_only
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|(_, created, blob)| {
                                store::instant_in_range(*created, range)
                                    && text_matches(&query, blob)
                            })
                            .map(|(id, _, _)| id)
                            .collect()
                    };
                    let mut out = Vec::new();
                    for (id, created, blob) in inflight {
                        if store::instant_in_range(created, range) && text_matches(&query, &blob) {
                            out.push(id);
                        }
                    }
                    persisted.retain(|id| !out.contains(id));
                    out.append(&mut persisted);
                    out
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                if this.search_gen != gen {
                    return;
                }
                this.visible_ids = ids;
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: filter task: {err}");
            }
        })
        .detach();
    }

    fn load_png_then_retry(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
            doc.status = DocStatus::Recognizing;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { store.load_png(id) }).await;
            if let Err(err) = this.update(cx, |this, cx| {
                match loaded {
                    Ok(img) => {
                        if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                            doc.image = ImageSlot::Loaded(Arc::new(img));
                            doc.blocks.clear();
                        }
                        this.touch_lru(id);
                        this.start_ocr(id, cx);
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: load png: {err}");
                        if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                            doc.image = ImageSlot::Missing;
                            doc.status = DocStatus::Ready;
                        }
                    }
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: retry load: {err}");
            }
        })
        .detach();
    }

    pub fn ensure_detail(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(doc) = self.documents.iter().find(|d| d.id == id) else {
            return;
        };
        let need_blocks = doc.persisted && !doc.blocks_loaded;
        let need_png = matches!(doc.image, ImageSlot::OnDisk);
        if need_blocks {
            self.load_blocks(id, cx);
        }
        if need_png {
            self.load_png(id, cx);
        }
    }

    fn load_blocks(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.load_blocks(id) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                match result {
                    Ok(blocks) => {
                        if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                            doc.blocks = blocks;
                            doc.blocks_loaded = true;
                            doc.refresh_first_line();
                        }
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: load blocks: {err}");
                        if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                            doc.blocks_loaded = true;
                        }
                    }
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: blocks task: {err}");
            }
        })
        .detach();
    }

    fn load_png(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { store.load_png(id) }).await;
            if let Err(err) = this.update(cx, |this, cx| {
                if this.selected != Some(id) {
                    return;
                }
                match result {
                    Ok(img) => {
                        if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                            doc.image = ImageSlot::Loaded(Arc::new(img));
                        }
                        this.touch_lru(id);
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: load png: {err}");
                        if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                            doc.image = ImageSlot::Missing;
                        }
                    }
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: png task: {err}");
            }
        })
        .detach();
    }

    fn touch_lru(&mut self, id: Uuid) {
        self.loaded_lru.retain(|x| *x != id);
        self.loaded_lru.push(id);
        while self.loaded_lru.len() > 3 {
            let Some(pos) = self
                .loaded_lru
                .iter()
                .position(|x| Some(*x) != self.selected)
            else {
                break;
            };
            let evict = self.loaded_lru.remove(pos);
            if let Some(doc) = self.documents.iter_mut().find(|d| d.id == evict) {
                if doc.persisted && matches!(doc.image, ImageSlot::Loaded(_)) {
                    doc.image = ImageSlot::OnDisk;
                }
            }
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

fn text_matches(query: &str, blob: &str) -> bool {
    let q = query.trim();
    q.is_empty() || blob.to_lowercase().contains(&q.to_lowercase())
}

fn activate_window<V: 'static>(handle: WindowHandle<V>, cx: &mut App) {
    if let Err(err) = handle.update(cx, |_, window, _| {
        window.activate_window();
    }) {
        eprintln!("{APP_SLUG}: activate window: {err}");
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
        KeyBinding::new("j", SelectNext, Some("SnipList && !SearchField")),
        KeyBinding::new("up", SelectPrev, None),
        KeyBinding::new("k", SelectPrev, Some("SnipList && !SearchField")),
        KeyBinding::new("delete", DeleteSelected, Some("SnipList && !SearchField")),
        KeyBinding::new(
            "backspace",
            DeleteSelected,
            Some("SnipList && !SearchField"),
        ),
        KeyBinding::new("ctrl-l", ToggleFormat, None),
        KeyBinding::new("ctrl-r", RetryOcr, None),
        KeyBinding::new("ctrl-q", QuitApp, None),
    ]);
    crate::ui::search_field::bind_keys(cx);
}
