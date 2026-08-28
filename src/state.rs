use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
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
use crate::ingest::IngestSource;
use crate::library::Library;
use crate::ocr::Engine;
use crate::ocr_queue::OcrQueue;
use crate::prefs::{Prefs, WindowCloseAction};
use crate::store::{self, CivilDate, Store};
use crate::ui::MainWindow;

pub use crate::library::DatePreset;

enum Capture {
    Idle,
    Grabbing,
    Failed(String),
}

pub struct AppState {
    pub library: Library,
    export_fmt: ExportFmt,
    pub prefs: Prefs,
    engine: Arc<Engine>,
    store: Option<Arc<Store>>,
    search_query: String,
    search_gen: u64,
    ocr_queue: OcrQueue,
    file_queue: VecDeque<PathBuf>,
    file_loading: bool,
    thumb_inflight: HashSet<Uuid>,
    capture: Capture,
    /// Bumped on every capture-status change so a flash timer cannot clear a
    /// newer error (or a later Idle/Grabbing).
    capture_gen: u64,
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
            search_query: String::new(),
            search_gen: 0,
            ocr_queue: OcrQueue::new(),
            file_queue: VecDeque::new(),
            file_loading: false,
            thumb_inflight: HashSet::new(),
            capture: Capture::Idle,
            capture_gen: 0,
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

    fn set_capture(&mut self, next: Capture) {
        self.capture = next;
        self.capture_gen = self.capture_gen.wrapping_add(1);
    }

    /// Footer hint for a recoverable miss. Clears itself so it cannot stick
    /// after the user has moved on (paste with a real image, snip, etc.).
    fn flash_capture_error(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.set_capture(Capture::Failed(msg.into()));
        let gen = self.capture_gen;
        cx.notify();
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_secs(4)).await;
            let _ = this.update(cx, |this, cx| {
                if this.capture_gen == gen && matches!(this.capture, Capture::Failed(_)) {
                    this.set_capture(Capture::Idle);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn engine_status(&self) -> crate::ocr::EngineStatus {
        self.engine.status()
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.library.selected
    }

    pub fn selected_doc(&self) -> Option<&Document> {
        self.library.selected_doc()
    }

    pub fn visible_ids(&self) -> &[Uuid] {
        &self.library.visible_ids
    }

    pub fn date_preset(&self) -> DatePreset {
        self.library.date_preset
    }

    pub fn is_empty(&self) -> bool {
        self.library.is_empty()
    }

    pub fn gpu_full_ids(&self) -> Vec<Uuid> {
        self.library.gpu_full_ids()
    }

    pub fn request_capture(&mut self, cx: &mut Context<Self>) {
        if matches!(self.capture, Capture::Grabbing) {
            return;
        }
        self.set_capture(Capture::Grabbing);
        cx.notify();
        self.dismiss_main_sheet(cx);
        crate::desktop::prepare_snip_input();

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
                    this.set_capture(Capture::Idle);
                    this.reveal_on_main = false;
                    this.restore_after_hide(cx);
                    cx.notify();
                }
                Err(err) => {
                    this.flash_capture_error(err.to_string(), cx);
                    this.restore_after_hide(cx);
                }
            }) {
                eprintln!("{APP_SLUG}: capture task: {err}");
            }
        })
        .detach();
    }

    pub fn finish_capture(&mut self, crop: RgbaImage, cx: &mut Context<Self>) {
        self.reveal_on_main = true;
        self.set_capture(Capture::Idle);
        self.ingest(IngestSource::Screen(crop), cx);
    }

    pub fn ingest(&mut self, source: IngestSource, cx: &mut Context<Self>) {
        match source {
            IngestSource::Screen(image) => self.ingest_pixels(image, cx),
            IngestSource::Files(paths) => {
                self.file_queue.extend(paths);
                self.pump_file_ingest(cx);
            }
            IngestSource::Strokes(pts) => {
                cx.spawn(async move |this, cx| {
                    let img = cx
                        .background_spawn(async move { crate::imgutil::rasterize_strokes(&pts, 3) })
                        .await;
                    if let Err(err) = this.update(cx, |this, cx| {
                        if let Some(img) = img {
                            this.ingest_pixels(img, cx);
                        } else {
                            this.flash_capture_error("That drawing is empty", cx);
                        }
                    }) {
                        eprintln!("{APP_SLUG}: stroke ingest: {err}");
                    }
                })
                .detach();
            }
            IngestSource::PdfPages { .. } => {
                self.flash_capture_error("PDF ingest is not implemented yet", cx);
            }
        }
    }

    fn ingest_pixels(&mut self, image: RgbaImage, cx: &mut Context<Self>) {
        self.set_capture(Capture::Idle);
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
        self.library.insert_newest(doc);
        self.enqueue_ocr(id, cx);
        cx.notify();
    }

    pub fn request_upload(&mut self, cx: &mut Context<Self>) {
        if self.is_capturing() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                .pick_files()
                .await;
            let Some(handles) = picked else {
                return;
            };
            let paths: Vec<PathBuf> = handles.iter().map(|h| h.path().to_path_buf()).collect();
            if let Err(err) = this.update(cx, |this, cx| {
                this.ingest(IngestSource::Files(paths), cx);
            }) {
                eprintln!("{APP_SLUG}: upload task: {err}");
            }
        })
        .detach();
    }

    /// Home "Paste" entry: clipboard image → OCR; image file path(s) → open + OCR.
    pub fn request_paste(&mut self, cx: &mut Context<Self>) {
        if self.is_capturing() {
            return;
        }
        // Windows: GPUI skips CF_DIB/CF_BITMAP and can return text when a
        // bitmap is also present. Try every native candidate (PNG may be a
        // stub; DIB/HBITMAP still work).
        let candidates = crate::desktop::read_clipboard_image();
        if !candidates.is_empty() {
            self.spawn_paste_image(
                move || {
                    let mut last = None;
                    for raw in candidates {
                        match crate::desktop::decode_clipboard_image(raw) {
                            Ok(img) => return Ok(img),
                            Err(err) => last = Some(err),
                        }
                    }
                    Err(last.unwrap_or_else(|| anyhow::anyhow!("no clipboard image")))
                },
                cx,
            );
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            self.flash_capture_error("Clipboard is empty — copy an image first", cx);
            return;
        };
        let image_bytes = item.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(img) => Some(img.bytes.clone()),
            _ => None,
        });
        if let Some(bytes) = image_bytes {
            self.spawn_paste_image(
                move || {
                    image::load_from_memory(&bytes)
                        .map(|d| d.to_rgba8())
                        .map_err(|err| anyhow::anyhow!("{err}"))
                },
                cx,
            );
            return;
        }
        let paths = item
            .text()
            .map(|text| clipboard_image_paths(&text))
            .unwrap_or_default();
        if !paths.is_empty() {
            self.ingest(IngestSource::Files(paths), cx);
            return;
        }
        self.flash_capture_error("Nothing to paste — copy an image first", cx);
    }

    fn spawn_paste_image<F>(&mut self, decode: F, cx: &mut Context<Self>)
    where
        F: FnOnce() -> anyhow::Result<image::RgbaImage> + Send + 'static,
    {
        self.set_capture(Capture::Idle);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let decoded = cx.background_spawn(async move { decode() }).await;
            if let Err(err) = this.update(cx, |this, cx| match decoded {
                Ok(img) => this.ingest_pixels(img, cx),
                Err(err) => {
                    eprintln!("{APP_SLUG}: clipboard image: {err}");
                    this.flash_capture_error("Couldn't read that clipboard image", cx);
                }
            }) {
                eprintln!("{APP_SLUG}: paste task: {err}");
            }
        })
        .detach();
    }

    fn pump_file_ingest(&mut self, cx: &mut Context<Self>) {
        if self.file_loading || !self.ocr_queue.is_idle() {
            return;
        }
        let Some(path) = self.file_queue.pop_front() else {
            return;
        };
        self.file_loading = true;
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_spawn(async move { image::open(&path).map(|d| d.to_rgba8()) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.file_loading = false;
                match decoded {
                    Ok(img) => this.ingest_pixels(img, cx),
                    Err(err) => {
                        eprintln!("{APP_SLUG}: open image: {err}");
                        this.flash_capture_error("Couldn't open that image", cx);
                    }
                }
                this.pump_file_ingest(cx);
            }) {
                eprintln!("{APP_SLUG}: file decode: {err}");
            }
        })
        .detach();
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

    fn enqueue_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.ocr_queue.enqueue(id);
        self.pump_ocr(cx);
    }

    fn pump_ocr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.ocr_queue.take_next() else {
            self.pump_file_ingest(cx);
            return;
        };
        self.start_ocr(id, cx);
    }

    pub fn start_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(image) = self.library.pixels(id) else {
            self.ocr_queue.finish(id);
            self.pump_ocr(cx);
            return;
        };
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.recognize(image.as_ref()) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                if let Some(doc) = this.library.get_mut(id) {
                    match result {
                        Ok(out) => {
                            doc.blocks = out.blocks;
                            doc.blocks_loaded = true;
                            doc.ocr = out.meta;
                            doc.status = DocStatus::Ready;
                            doc.refresh_first_line();
                            doc.bump_revision();
                        }
                        Err(err) => {
                            doc.status = DocStatus::Failed(err.to_string());
                            doc.bump_revision();
                        }
                    }
                }
                this.ocr_queue.finish(id);
                let ready = this
                    .library
                    .get(id)
                    .is_some_and(|d| matches!(d.status, DocStatus::Ready));
                if ready {
                    this.persist_ready(id, cx);
                    if this.prefs.autocopy && this.library.selected == Some(id) {
                        this.copy_selected(cx);
                    }
                }
                this.restore_after_hide(cx);
                if this.reveal_on_main {
                    this.reveal_on_main = false;
                    this.dismiss_main_sheet(cx);
                }
                this.pump_ocr(cx);
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: ocr task: {err}");
            }
        })
        .detach();
    }

    pub fn retry_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected else {
            return;
        };
        let slot = self.library.get(id).map(|d| d.image.clone());
        match slot {
            None | Some(ImageSlot::Missing) => (),
            Some(ImageSlot::Loaded(_)) => {
                if let Some(doc) = self.library.get_mut(id) {
                    doc.status = DocStatus::Recognizing;
                    doc.blocks.clear();
                    doc.bump_revision();
                }
                self.enqueue_ocr(id, cx);
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
        if self.library.visible_ids.is_empty() {
            return;
        }
        let idx = self
            .library
            .selected
            .and_then(|id| self.library.visible_ids.iter().position(|x| *x == id))
            .unwrap_or(0) as isize;
        let next = (idx + delta).clamp(0, self.library.visible_ids.len() as isize - 1) as usize;
        self.select(self.library.visible_ids[next], cx);
    }

    pub fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected else {
            return;
        };
        self.ocr_queue.remove(id);
        self.library.remove(id);
        self.thumb_inflight.remove(&id);
        if self.library.is_empty() {
            self.ocr_queue.cancel_remaining();
        }
        if let Some(store) = self.store.clone() {
            cx.background_spawn(async move {
                if let Err(err) = store.delete(id) {
                    eprintln!("{APP_SLUG}: delete snip: {err}");
                }
            })
            .detach();
        }
        if let Some(id) = self.library.selected {
            self.ensure_detail(id, cx);
        }
        self.pump_ocr(cx);
        cx.notify();
    }

    pub fn toggle_format(&mut self, cx: &mut Context<Self>) {
        self.export_fmt = self.export_fmt.toggle();
        cx.notify();
    }

    pub fn select(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.library.selected = Some(id);
        self.library.touch_lru(id);
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
        if self.library.date_preset == preset {
            return;
        }
        self.library.date_preset = preset;
        self.schedule_filter(cx);
    }

    pub fn boot_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.library.selected {
            self.ensure_detail(id, cx);
        }
    }

    pub fn request_thumb(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.thumb_inflight.contains(&id) {
            return;
        }
        let Some(doc) = self.library.get(id) else {
            return;
        };
        if !doc.thumb_jpeg.is_empty() || !doc.persisted {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.thumb_inflight.insert(id);
        cx.spawn(async move |this, cx| {
            let jpeg = cx
                .background_spawn(async move { store.load_thumb(id) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.thumb_inflight.remove(&id);
                match jpeg {
                    Ok(bytes) => {
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.thumb_jpeg = bytes;
                        }
                    }
                    Err(err) => eprintln!("{APP_SLUG}: load thumb: {err}"),
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: thumb task: {err}");
            }
        })
        .detach();
    }

    fn persist_ready(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let Some(doc) = self.library.get(id).cloned() else {
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
                        if let Some(d) = this.library.get_mut(id) {
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
        let range = self.library.date_preset.to_range(CivilDate::today_local());
        let store = self.store.clone();
        let inflight: Vec<(Uuid, std::time::SystemTime, String)> = self
            .library
            .iter_all()
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
                self.library
                    .iter_all()
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
                this.library.visible_ids = ids;
                // Keep the detail pane on a visible row while filtering.
                let sel_gone = this
                    .library
                    .selected
                    .is_none_or(|s| !this.library.visible_ids.contains(&s));
                if sel_gone {
                    this.library.selected = this.library.visible_ids.first().copied();
                    if let Some(id) = this.library.selected {
                        this.ensure_detail(id, cx);
                    }
                }
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
        if let Some(doc) = self.library.get_mut(id) {
            doc.status = DocStatus::Recognizing;
            doc.bump_revision();
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { store.load_png(id) }).await;
            if let Err(err) = this.update(cx, |this, cx| {
                match loaded {
                    Ok(img) => {
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.image = ImageSlot::Loaded(Arc::new(img));
                            doc.blocks.clear();
                        }
                        this.library.touch_lru(id);
                        this.enqueue_ocr(id, cx);
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: load png: {err}");
                        if let Some(doc) = this.library.get_mut(id) {
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
        let Some(doc) = self.library.get(id) else {
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
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.blocks = blocks;
                            doc.blocks_loaded = true;
                            doc.refresh_first_line();
                            doc.bump_revision();
                        }
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: load blocks: {err}");
                        if let Some(doc) = this.library.get_mut(id) {
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
                if this.library.selected != Some(id) {
                    return;
                }
                match result {
                    Ok(img) => {
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.image = ImageSlot::Loaded(Arc::new(img));
                        }
                        this.library.touch_lru(id);
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: load png: {err}");
                        if let Some(doc) = this.library.get_mut(id) {
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

/// Clipboard text as image paths: one per line, `file://` tolerated.
fn clipboard_image_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| PathBuf::from(line.strip_prefix("file://").unwrap_or(line)))
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
                    )
                })
        })
        .collect()
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
        KeyBinding::new("ctrl-v", PasteSnip, None),
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
