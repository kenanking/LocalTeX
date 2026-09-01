use std::path::PathBuf;
use std::sync::Arc;

use gpui::{App, AppContext, ClipboardItem, Context};
use image::RgbaImage;
use uuid::Uuid;

use crate::doc::{Block, DocStatus, Document, ImageSlot, SnipKind};
use crate::export::CopyKind;
use crate::identity::APP_SLUG;

use super::session::Capture;
use super::AppState;

pub enum IngestSource {
    Screen(RgbaImage),
    Files(Vec<PathBuf>),
    Strokes(Vec<Vec<[f32; 3]>>),
}

impl AppState {
    pub fn ingest(&mut self, source: IngestSource, cx: &mut Context<Self>) {
        match source {
            IngestSource::Screen(image) => self.ingest_pixels(image, cx),
            IngestSource::Files(paths) => {
                self.ingest.file_queue.extend(paths);
                self.pump_file_ingest(cx);
            }
            IngestSource::Strokes(pts) => {
                let xy = crate::imgutil::traces_xy(&pts);
                cx.spawn(async move |this, cx| {
                    let img = cx
                        .background_spawn(async move { crate::imgutil::rasterize_strokes(&xy, 3) })
                        .await;
                    if let Err(err) = this.update(cx, |this, cx| {
                        if let Some(img) = img {
                            this.ingest_drawing(pts, img, cx);
                        } else {
                            this.flash_capture_error("That drawing is empty", cx);
                        }
                    }) {
                        eprintln!("{APP_SLUG}: stroke ingest: {err}");
                    }
                })
                .detach();
            }
        }
    }

    pub(super) fn ingest_pixels(&mut self, image: RgbaImage, cx: &mut Context<Self>) {
        self.capture.set(Capture::Idle);
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

    fn ingest_drawing(
        &mut self,
        traces: Vec<Vec<[f32; 3]>>,
        image: RgbaImage,
        cx: &mut Context<Self>,
    ) {
        self.capture.set(Capture::Idle);
        let image = crate::imgutil::cap_megapixels(image);
        let mut doc = Document::pending(Arc::new(image));
        doc.ink = Some(Arc::new(traces));
        let id = doc.id;
        self.library.insert_newest(doc);
        self.enqueue_ocr(id, cx);
        cx.notify();
    }

    fn pump_file_ingest(&mut self, cx: &mut Context<Self>) {
        if self.ingest.file_loading {
            return;
        }
        let Some(path) = self.ingest.file_queue.pop_front() else {
            return;
        };
        self.ingest.file_loading = true;
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_spawn(async move { image::open(&path).map(|d| d.to_rgba8()) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.ingest.file_loading = false;
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

    fn enqueue_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.ingest.ocr.enqueue(id);
        self.pump_ocr(cx);
    }

    fn pump_ocr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.ingest.ocr.take_next() else {
            self.pump_file_ingest(cx);
            return;
        };
        self.start_ocr(id, cx);
    }

    pub fn start_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let slot = self.library.get(id).map(|d| d.image.clone());
        match missing_pixels_action(slot.as_ref()) {
            MissingPixels::Run => {}
            MissingPixels::LoadPng => {
                self.ingest.ocr.finish(id);
                self.load_png_then_retry(id, cx);
                return;
            }
            MissingPixels::Fail(msg) => {
                if let Some(doc) = self.library.get_mut(id) {
                    doc.status = DocStatus::Failed(msg.into());
                    doc.bump_revision();
                }
                self.ingest.ocr.finish(id);
                self.pump_ocr(cx);
                cx.notify();
                return;
            }
        }
        let Some(image) = self.library.pixels(id) else {
            if let Some(doc) = self.library.get_mut(id) {
                doc.status = DocStatus::Failed("Original image is missing".into());
                doc.bump_revision();
            }
            self.ingest.ocr.finish(id);
            self.pump_ocr(cx);
            cx.notify();
            return;
        };
        let ink = self.library.get(id).and_then(|d| d.ink.clone());
        let size = (image.width(), image.height());
        let engine = self.engine.clone();
        let store = self.store.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let traces = resolve_ink_traces(ink, store.as_ref().map(|s| s.load_ink(id)))?;
                    if let Some(traces) = traces {
                        engine.recognize_ink(traces.as_ref(), size)
                    } else {
                        engine.recognize(image.as_ref())
                    }
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                if let Some(doc) = this.library.get_mut(id) {
                    match result {
                        Ok(out) => {
                            doc.blocks = out.blocks;
                            doc.ocr_blocks = doc.blocks.clone();
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
                this.ingest.ocr.finish(id);
                let ready = this
                    .library
                    .get(id)
                    .is_some_and(|d| matches!(d.status, DocStatus::Ready));
                if ready {
                    this.persist_ready(id, cx);
                    if this.prefs.autocopy && this.library.selected() == Some(id) {
                        this.copy_selected(cx);
                    }
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
        let Some(id) = self.library.selected() else {
            return;
        };
        if self.library.get(id).is_some_and(|d| d.is_edited()) {
            return;
        }
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

    pub fn copy_selected(&mut self, cx: &mut App) {
        let Some(doc) = self.selected_doc() else {
            return;
        };
        let snip = doc.snip_kind();
        let kind = self.prefs.copy_habit.resolve(snip, self.export_fmt);
        let text = doc.text_for(kind, &self.prefs);
        if text.is_empty() {
            return;
        }
        Self::write_clipboard(text, cx);
        self.record_copy_habit(snip, kind);
    }

    pub fn copy_chip(&mut self, kind: CopyKind, cx: &mut App) {
        let Some(doc) = self.selected_doc() else {
            return;
        };
        let snip = doc.snip_kind();
        let text = doc.text_for(kind, &self.prefs);
        if text.is_empty() {
            return;
        }
        Self::write_clipboard(text, cx);
        self.record_copy_habit(snip, kind);
    }

    fn record_copy_habit(&mut self, snip: SnipKind, kind: CopyKind) {
        if self.prefs.copy_habit.remember(snip, kind) {
            self.persist_prefs();
        }
    }

    pub fn open_docx_selected(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.selected_doc() else {
            return;
        };
        if !matches!(doc.status, DocStatus::Ready) || !doc.blocks_loaded {
            return;
        }
        let blocks = doc.blocks.clone();
        let id = doc.id;
        cx.spawn(async move |this, cx| {
            let built = cx
                .background_spawn(async move {
                    let bytes = crate::office::build_docx(&blocks)?;
                    let path = std::env::temp_dir().join(format!("{APP_SLUG}-{id}.docx"));
                    std::fs::write(&path, bytes)?;
                    anyhow::Ok(path)
                })
                .await;
            match built {
                Ok(path) => {
                    let _ = this.update(cx, |_, cx| {
                        cx.open_with_system(&path);
                    });
                }
                Err(err) => {
                    eprintln!("{APP_SLUG}: open docx: {err:#}");
                    let _ = this.update(cx, |this, cx| {
                        this.flash_capture_error("Couldn't open a Word document", cx);
                    });
                }
            }
        })
        .detach();
    }

    fn write_clipboard(text: String, cx: &mut App) {
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected() else {
            return;
        };
        self.ingest.ocr.remove(id);
        self.library.remove(id);
        self.ingest.thumb_inflight.remove(&id);
        if self.library.is_empty() {
            self.ingest.ocr.cancel_remaining();
        }
        if let Some(store) = self.store.clone() {
            cx.background_spawn(async move {
                if let Err(err) = store.delete(id) {
                    eprintln!("{APP_SLUG}: delete snip: {err}");
                }
            })
            .detach();
        }
        if let Some(id) = self.library.selected() {
            self.ensure_detail(id, cx);
        }
        self.pump_ocr(cx);
        cx.notify();
    }

    pub fn request_thumb(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.ingest.thumb_inflight.contains(&id) {
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
        self.ingest.thumb_inflight.insert(id);
        cx.spawn(async move |this, cx| {
            let jpeg = cx
                .background_spawn(async move { store.load_thumb(id) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.ingest.thumb_inflight.remove(&id);
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

    pub fn apply_parsed_source(&mut self, id: Uuid, blocks: Vec<Block>, cx: &mut Context<Self>) {
        if let Some(doc) = self.library.get_mut(id) {
            if !matches!(doc.status, DocStatus::Ready) {
                return;
            }
            doc.blocks = blocks;
            doc.refresh_first_line();
            doc.bump_revision();
        }
        self.persist_edits(id, cx);
        self.schedule_filter(cx);
        cx.notify();
    }

    pub fn revert_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(doc) = self.library.get_mut(id) else {
            return;
        };
        if !doc.is_edited() {
            return;
        }
        doc.blocks = doc.ocr_blocks.clone();
        doc.refresh_first_line();
        doc.bump_revision();
        self.persist_edits(id, cx);
        self.schedule_filter(cx);
        cx.notify();
    }

    fn persist_edits(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let Some(doc) = self.library.get(id).cloned() else {
            return;
        };
        if !doc.persisted || !matches!(doc.status, DocStatus::Ready) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.update_blocks(&doc) })
                .await;
            if let Err(err) = this.update(cx, |_, cx| {
                if let Err(err) = result {
                    eprintln!("{APP_SLUG}: persist edits: {err}");
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: persist edits task: {err}");
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
                            doc.status =
                                DocStatus::Failed("Couldn't load the original image".into());
                            doc.bump_revision();
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
                .background_spawn(async move { store.load_block_pair(id) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                match result {
                    Ok((blocks, ocr_blocks)) => {
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.blocks = blocks;
                            doc.ocr_blocks = ocr_blocks;
                            doc.blocks_loaded = true;
                            doc.refresh_first_line();
                            doc.bump_revision();
                        }
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: load blocks: {err}");
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.status = DocStatus::Failed("Couldn't load recognized text".into());
                            doc.bump_revision();
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
                if this.library.selected() != Some(id) {
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
}

#[derive(Debug, PartialEq, Eq)]
enum MissingPixels {
    Run,
    LoadPng,
    Fail(&'static str),
}

fn missing_pixels_action(slot: Option<&ImageSlot>) -> MissingPixels {
    match slot {
        Some(ImageSlot::Loaded(_)) => MissingPixels::Run,
        Some(ImageSlot::OnDisk) => MissingPixels::LoadPng,
        Some(ImageSlot::Missing) => MissingPixels::Fail("Original image is missing"),
        None => MissingPixels::Fail("Snip is gone"),
    }
}

type InkTraces = Vec<Vec<[f32; 3]>>;

fn resolve_ink_traces(
    ram: Option<Arc<InkTraces>>,
    disk: Option<anyhow::Result<Option<InkTraces>>>,
) -> anyhow::Result<Option<Arc<InkTraces>>> {
    if let Some(traces) = ram {
        return Ok(Some(traces));
    }
    match disk {
        None | Some(Ok(None)) => Ok(None),
        Some(Ok(Some(traces))) => Ok(Some(Arc::new(traces))),
        Some(Err(err)) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn missing_pixels_do_not_drop_recognizing_without_status() {
        let loaded = ImageSlot::Loaded(Arc::new(RgbaImage::new(2, 2)));
        assert_eq!(missing_pixels_action(Some(&loaded)), MissingPixels::Run);
        assert_eq!(
            missing_pixels_action(Some(&ImageSlot::OnDisk)),
            MissingPixels::LoadPng
        );
        assert!(matches!(
            missing_pixels_action(Some(&ImageSlot::Missing)),
            MissingPixels::Fail(_)
        ));
        assert!(matches!(
            missing_pixels_action(None),
            MissingPixels::Fail(_)
        ));
    }

    #[test]
    fn disk_ink_error_does_not_fall_through_to_page_ocr() {
        let err = anyhow::anyhow!("decode ink");
        let out = resolve_ink_traces(None, Some(Err(err)));
        assert!(out.is_err());
        let none = resolve_ink_traces(None, Some(Ok(None))).unwrap();
        assert!(none.is_none());
        let ram = Arc::new(vec![vec![[0.0, 0.0, 0.0]]]);
        let traces = resolve_ink_traces(Some(ram.clone()), Some(Err(anyhow::anyhow!("ignored"))))
            .unwrap()
            .expect("ram wins");
        assert!(Arc::ptr_eq(&traces, &ram));
    }
}
