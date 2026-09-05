use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AppContext, Context};
use image::RgbaImage;
use uuid::Uuid;

use crate::doc::{DocStatus, Document, ImageSlot};
use crate::identity::APP_SLUG;

use super::intake::{classify_image_paths, ClassifiedPaths, IntakeBatch};
use super::session::Capture;
use super::AppState;

impl AppState {
    pub fn ingest_strokes(&mut self, pts: Vec<Vec<[f32; 3]>>, cx: &mut Context<Self>) {
        if self.is_bootstrapping() || self.is_shutting_down() {
            return;
        }
        let xy = crate::imgutil::traces_xy(&pts);
        cx.spawn(async move |this, cx| {
            let img = cx
                .background_spawn(async move {
                    crate::imgutil::rasterize_strokes(&xy, 3).map(crate::imgutil::cap_megapixels)
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                if let Some(img) = img {
                    this.ingest_drawing(pts, img, cx);
                } else {
                    this.flash_error("That drawing is empty", cx);
                }
            }) {
                eprintln!("{APP_SLUG}: stroke ingest: {err}");
            }
        })
        .detach();
    }

    pub fn offer_files(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        #[cfg(target_os = "windows")]
        if crate::desktop::session_pending() {
            return;
        }
        if self.is_bootstrapping() || self.is_capturing() || self.is_shutting_down() {
            return;
        }
        let classified = classify_image_paths(paths);
        if classified.images.is_empty() {
            if classified.skipped > 0 {
                if let Some(batch) = self.intake.as_mut().filter(|batch| batch.is_accepting()) {
                    batch.skipped = batch.skipped.saturating_add(classified.skipped);
                    cx.notify();
                } else {
                    self.begin_intake_reject(classified.skipped, cx);
                }
            }
            return;
        }
        self.begin_intake_images(classified, cx);
        self.pump_file_ingest(cx);
    }

    fn begin_intake_images(&mut self, classified: ClassifiedPaths, cx: &mut Context<Self>) {
        let paths = classified.images;
        let skipped = classified.skipped;
        if self.intake.as_ref().is_some_and(IntakeBatch::is_accepting) {
            self.intake
                .as_mut()
                .expect("accepting intake exists")
                .extend(paths, skipped);
        } else {
            self.intake_gen = self.intake_gen.wrapping_add(1);
            self.intake = Some(IntakeBatch::from_paths(paths, skipped, self.intake_gen));
        }
        cx.notify();
    }

    fn begin_intake_reject(&mut self, skipped: usize, cx: &mut Context<Self>) {
        self.intake_gen = self.intake_gen.wrapping_add(1);
        let gen = self.intake_gen;
        self.intake = Some(IntakeBatch::reject(skipped, gen));
        cx.notify();
    }

    pub fn intake(&self) -> Option<&IntakeBatch> {
        self.intake.as_ref()
    }

    pub fn acknowledge_intake(&mut self, gen: u64, cx: &mut Context<Self>) {
        if self.intake.as_ref().is_some_and(|batch| {
            batch.gen == gen && (batch.items.is_empty() || batch.all_terminal())
        }) {
            self.intake = None;
            cx.notify();
        }
    }

    pub(super) fn ingest_pixels(&mut self, image: RgbaImage, cx: &mut Context<Self>) {
        if self.is_shutting_down() {
            return;
        }
        let id = self.insert_pixels(image);
        self.persist_ready(id, cx);
        cx.notify();
    }

    fn insert_pixels(&mut self, image: RgbaImage) -> Uuid {
        self.capture.set(Capture::Idle);
        let doc = Document::pending(Arc::new(image));
        let id = doc.id;
        self.library.insert_newest(doc);
        id
    }

    fn ingest_drawing(
        &mut self,
        traces: Vec<Vec<[f32; 3]>>,
        image: RgbaImage,
        cx: &mut Context<Self>,
    ) {
        self.capture.set(Capture::Idle);
        if self.is_shutting_down() {
            return;
        }
        let mut doc = Document::pending(Arc::new(image));
        doc.ink = Some(Arc::new(traces));
        let id = doc.id;
        self.library.insert_newest(doc);
        self.persist_ready(id, cx);
        cx.notify();
    }

    fn pump_file_ingest(&mut self, cx: &mut Context<Self>) {
        if self.file_intake.loading
            || self.is_shutting_down()
            || self.intake.as_ref().is_some_and(|batch| {
                batch.items.iter().any(|item| {
                    item.id.is_some() && item.id != self.ocr.running() && !item.work.is_terminal()
                })
            })
        {
            return;
        }
        let Some((gen, key, path)) = self.intake.as_ref().and_then(|batch| {
            batch
                .next_pending()
                .map(|(key, path)| (batch.gen, key, path))
        }) else {
            return;
        };
        self.file_intake.loading = true;
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_spawn(async move {
                    image::open(&path).map(|d| crate::imgutil::cap_megapixels(d.to_rgba8()))
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.file_intake.loading = false;
                if this.is_shutting_down() {
                    return;
                }
                let current = this.intake.as_ref().is_some_and(|batch| {
                    batch.gen == gen && batch.items.iter().any(|item| item.key == key)
                });
                if !current {
                    this.pump_file_ingest(cx);
                    return;
                }
                match decoded {
                    Ok(img) => {
                        let id = this.insert_pixels(img);
                        if let Some(batch) = &mut this.intake {
                            batch.bind_item(key, id);
                        }
                        this.persist_ready(id, cx);
                        cx.notify();
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: open image: {err}");
                        if let Some(batch) = &mut this.intake {
                            batch.finish_key(key, false);
                        }
                        cx.notify();
                    }
                }
                this.pump_file_ingest(cx);
            }) {
                eprintln!("{APP_SLUG}: file decode: {err}");
            }
        })
        .detach();
    }

    pub(super) fn enqueue_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.is_shutting_down() || self.library.get(id).is_none() {
            return;
        }
        self.ocr.enqueue(id);
        self.pump_ocr(cx);
        self.pump_file_ingest(cx);
    }

    pub(super) fn pump_ocr(&mut self, cx: &mut Context<Self>) {
        if self.is_shutting_down() {
            return;
        }
        let Some(id) = self.ocr.take_next() else {
            self.pump_file_ingest(cx);
            return;
        };
        self.start_ocr(id, cx);
    }

    pub fn start_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Some(batch) = &mut self.intake {
            batch.mark_working(id);
            cx.notify();
        }
        let slot = self.library.get(id).map(|d| d.image.clone());
        match missing_pixels_action(slot.as_ref()) {
            MissingPixels::Run => {}
            MissingPixels::LoadPng => {
                self.ocr.finish(id);
                self.load_png_then_retry(id, cx);
                return;
            }
            MissingPixels::Fail(msg) => {
                if let Some(doc) = self.library.get_mut(id) {
                    doc.status = DocStatus::Failed(msg.into());
                    doc.bump_revision();
                }
                self.ocr.finish(id);
                self.finish_intake_id(id, false, cx);
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
            self.ocr.finish(id);
            self.finish_intake_id(id, false, cx);
            self.pump_ocr(cx);
            cx.notify();
            return;
        };
        let ink = self.library.get(id).and_then(|d| d.ink.clone());
        let size = (image.width(), image.height());
        let engine = self.engine.clone();
        let store = self.store();
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
                if this.is_shutting_down() {
                    this.ocr.finish(id);
                    return;
                }
                if let Some(doc) = this.library.get_mut(id) {
                    match result {
                        Ok(out) => {
                            doc.ocr_blocks = out.blocks;
                            if doc.raw_text.is_none() {
                                doc.blocks = doc.ocr_blocks.clone();
                            }
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
                this.ocr.finish(id);
                this.schedule_engine_release(cx);
                let ready = this
                    .library
                    .get(id)
                    .is_some_and(|d| matches!(d.status, DocStatus::Ready));
                this.finish_intake_id(id, ready, cx);
                this.persist_ready(id, cx);
                if ready && this.prefs.autocopy && this.library.selected() == Some(id) {
                    if let Some((id, kind)) = this.copy_selected(cx) {
                        let handle = this.main_window.handle();
                        cx.defer(move |cx| {
                            if let Some(handle) = handle {
                                if let Err(err) = handle.update(cx, |view, _, cx| {
                                    view.flash_copied(id, kind, cx);
                                }) {
                                    eprintln!("{APP_SLUG}: autocopy flash: {err}");
                                }
                            }
                        });
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

    pub(super) fn finish_intake_id(&mut self, id: Uuid, ok: bool, cx: &mut Context<Self>) {
        if self
            .intake
            .as_mut()
            .is_some_and(|batch| batch.finish_id(id, ok))
        {
            cx.notify();
        }
    }

    pub fn retry_selected(&mut self, cx: &mut Context<Self>) {
        if self.is_shutting_down() {
            return;
        }
        let Some(id) = self.library.selected() else {
            return;
        };
        if self.library.get(id).is_some_and(|doc| !doc.blocks_loaded) {
            self.ensure_detail(id, cx);
            return;
        }
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
