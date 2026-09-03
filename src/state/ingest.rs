use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext, ClipboardItem, Context};
use image::RgbaImage;
use uuid::Uuid;

use crate::doc::{Block, DocStatus, Document, ImageSlot, PersistState, SnipKind};
use crate::export::CopyKind;
use crate::identity::APP_SLUG;
use crate::store::{WriteEvent, WriteKind, WriteResult};

use super::session::Capture;
use super::AppState;

pub enum IngestSource {
    Screen(RgbaImage),
    Files(Vec<PathBuf>),
    Strokes(Vec<Vec<[f32; 3]>>),
}

pub(crate) const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];
const INTAKE_REJECT_HOLD: Duration = Duration::from_millis(700);
const INTAKE_RESULT_HOLD: Duration = Duration::from_millis(480);
const INTAKE_LEAVE: Duration = Duration::from_millis(260);
const INTAKE_COMPLETE_HOLD: Duration = Duration::from_millis(1_500);
const INTAKE_PLATEN_OUT: Duration = Duration::from_millis(280);
pub const INTAKE_WINDOW: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntakeCounts {
    pub images: usize,
    pub skipped: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntakeWork {
    Waiting,
    Working,
    Succeeded,
    Failed,
}

impl IntakeWork {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntakeVisual {
    Backlog,
    Visible,
    Settled,
    Leaving,
    Gone,
}

impl IntakeVisual {
    pub fn is_present(self) -> bool {
        !matches!(self, Self::Gone)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntakeBatchPhase {
    Running,
    Complete,
    Fading,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntakePose {
    pub dx: f32,
    pub dy: f32,
}

pub fn intake_pose(index: usize) -> IntakePose {
    const DX: [f32; 6] = [-165.0, -99.0, -33.0, 33.0, 99.0, 165.0];
    const DY: [f32; 6] = [0.0; 6];
    let i = index.min(5);
    IntakePose {
        dx: DX[i],
        dy: DY[i],
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntakeItem {
    pub key: u64,
    pub path: PathBuf,
    pub name: String,
    pub id: Option<Uuid>,
    pub work: IntakeWork,
    pub visual: IntakeVisual,
    pub slot: Option<usize>,
}

impl IntakeItem {
    fn waiting(key: u64, path: PathBuf, slot: Option<usize>) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image")
            .to_string();
        Self {
            key,
            path,
            name,
            id: None,
            work: IntakeWork::Waiting,
            visual: if slot.is_some() {
                IntakeVisual::Visible
            } else {
                IntakeVisual::Backlog
            },
            slot,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntakeBatch {
    pub items: Vec<IntakeItem>,
    pub skipped: usize,
    pub gen: u64,
    pub phase: IntakeBatchPhase,
    next_item_key: u64,
}

impl IntakeBatch {
    pub fn from_paths(paths: Vec<PathBuf>, skipped: usize, gen: u64) -> Self {
        let items: Vec<_> = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                IntakeItem::waiting(
                    index as u64 + 1,
                    path,
                    (index < INTAKE_WINDOW).then_some(index),
                )
            })
            .collect();
        Self {
            next_item_key: items.len() as u64 + 1,
            items,
            skipped,
            gen,
            phase: IntakeBatchPhase::Running,
        }
    }

    pub fn reject(skipped: usize, gen: u64) -> Self {
        Self {
            items: Vec::new(),
            skipped,
            gen,
            phase: IntakeBatchPhase::Running,
            next_item_key: 1,
        }
    }

    pub fn is_accepting(&self) -> bool {
        self.phase == IntakeBatchPhase::Running && !self.items.is_empty()
    }

    pub fn counts(&self) -> IntakeCounts {
        IntakeCounts {
            images: self.items.len(),
            skipped: self.skipped,
        }
    }

    pub fn progress(&self) -> (usize, usize) {
        let done = self
            .items
            .iter()
            .filter(|item| item.work.is_terminal())
            .count();
        (done, self.items.len())
    }

    pub fn results(&self) -> (usize, usize) {
        let succeeded = self
            .items
            .iter()
            .filter(|item| item.work == IntakeWork::Succeeded)
            .count();
        let failed = self
            .items
            .iter()
            .filter(|item| item.work == IntakeWork::Failed)
            .count();
        (succeeded, failed)
    }

    pub fn all_gone(&self) -> bool {
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|item| item.visual == IntakeVisual::Gone)
    }

    pub fn thumb_ids(&self) -> Vec<Uuid> {
        self.visible()
            .into_iter()
            .filter_map(|(_, item)| item.id)
            .collect()
    }

    pub fn file_jobs(&self) -> Vec<(u64, u64, PathBuf)> {
        self.items
            .iter()
            .map(|item| (self.gen, item.key, item.path.clone()))
            .collect()
    }

    pub fn extend(&mut self, paths: Vec<PathBuf>, skipped: usize) -> Vec<(u64, u64, PathBuf)> {
        self.skipped = self.skipped.saturating_add(skipped);
        let mut added = Vec::new();
        for path in paths {
            let already = self
                .items
                .iter()
                .any(|item| item.path == path && !item.work.is_terminal());
            if already {
                continue;
            }
            let key = self.next_item_key;
            self.next_item_key = self.next_item_key.wrapping_add(1);
            let slot = self.first_vacant_slot();
            self.items
                .push(IntakeItem::waiting(key, path.clone(), slot));
            added.push((self.gen, key, path));
        }
        added
    }

    fn first_vacant_slot(&self) -> Option<usize> {
        (0..INTAKE_WINDOW).find(|slot| {
            !self
                .items
                .iter()
                .any(|item| item.slot == Some(*slot) && item.visual != IntakeVisual::Gone)
        })
    }

    pub fn bind_item(&mut self, key: u64, id: Uuid) {
        if let Some(item) = self.items.iter_mut().find(|item| item.key == key) {
            item.id = Some(id);
        }
    }

    pub fn mark_working(&mut self, id: Uuid) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == Some(id)) {
            if item.work == IntakeWork::Waiting {
                item.work = IntakeWork::Working;
            }
        }
    }

    pub fn finish_key(&mut self, key: u64, ok: bool) -> Option<u64> {
        let item = self.items.iter_mut().find(|item| item.key == key)?;
        Self::finish_item(item, ok)
    }

    pub fn finish_id(&mut self, id: Uuid, ok: bool) -> Option<u64> {
        let item = self.items.iter_mut().find(|item| item.id == Some(id))?;
        Self::finish_item(item, ok)
    }

    fn finish_item(item: &mut IntakeItem, ok: bool) -> Option<u64> {
        if item.work.is_terminal() {
            return None;
        }
        item.work = if ok {
            IntakeWork::Succeeded
        } else {
            IntakeWork::Failed
        };
        if item.visual == IntakeVisual::Visible {
            item.visual = IntakeVisual::Settled;
            Some(item.key)
        } else {
            None
        }
    }

    pub fn begin_leave(&mut self, key: u64) -> bool {
        let Some(item) = self.items.iter_mut().find(|item| item.key == key) else {
            return false;
        };
        if item.visual == IntakeVisual::Settled {
            item.visual = IntakeVisual::Leaving;
            true
        } else {
            false
        }
    }

    pub fn mark_gone_and_promote(&mut self, key: u64) -> Option<u64> {
        let slot = {
            let item = self.items.iter_mut().find(|item| item.key == key)?;
            if item.visual != IntakeVisual::Leaving {
                return None;
            }
            item.visual = IntakeVisual::Gone;
            item.slot.take()?
        };
        let replacement = self
            .items
            .iter_mut()
            .find(|item| item.visual == IntakeVisual::Backlog)?;
        replacement.slot = Some(slot);
        replacement.visual = if replacement.work.is_terminal() {
            IntakeVisual::Settled
        } else {
            IntakeVisual::Visible
        };
        if replacement.visual == IntakeVisual::Settled {
            Some(replacement.key)
        } else {
            None
        }
    }

    pub fn visible(&self) -> Vec<(usize, &IntakeItem)> {
        let mut visible: Vec<_> = self
            .items
            .iter()
            .filter(|item| item.visual.is_present())
            .filter_map(|item| item.slot.map(|slot| (slot, item)))
            .collect();
        visible.sort_by_key(|(slot, _)| *slot);
        visible
    }

    pub fn backlog_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.visual == IntakeVisual::Backlog)
            .count()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClassifiedPaths {
    pub images: Vec<PathBuf>,
    pub skipped: usize,
}

impl ClassifiedPaths {
    pub fn counts(&self) -> IntakeCounts {
        IntakeCounts {
            images: self.images.len(),
            skipped: self.skipped,
        }
    }
}

pub fn is_ingest_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| IMAGE_EXTS.iter().any(|ok| ext.eq_ignore_ascii_case(ok)))
}

pub fn classify_image_paths(paths: impl IntoIterator<Item = PathBuf>) -> ClassifiedPaths {
    let mut images = Vec::new();
    let mut skipped = 0;
    for path in paths {
        if is_ingest_image_path(&path) {
            images.push(path);
        } else {
            skipped += 1;
        }
    }
    ClassifiedPaths { images, skipped }
}

impl AppState {
    pub fn ingest(&mut self, source: IngestSource, cx: &mut Context<Self>) {
        if self.is_bootstrapping() {
            return;
        }
        match source {
            IngestSource::Screen(image) => {
                self.ingest_pixels(image, cx);
            }
            IngestSource::Files(paths) => self.offer_files(paths, cx),
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
                            this.flash_error("That drawing is empty", cx);
                        }
                    }) {
                        eprintln!("{APP_SLUG}: stroke ingest: {err}");
                    }
                })
                .detach();
            }
        }
    }

    pub fn offer_files(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.is_bootstrapping() || self.is_capturing() {
            return;
        }
        let classified = classify_image_paths(paths);
        if classified.images.is_empty() {
            if classified.skipped > 0 {
                if !self.intake.as_ref().is_some_and(IntakeBatch::is_accepting) {
                    self.begin_intake_reject(classified.skipped, cx);
                }
                self.flash_error("No images in this drop", cx);
            }
            return;
        }
        self.begin_intake_images(classified, cx);
        self.pump_file_ingest(cx);
    }

    fn begin_intake_images(&mut self, classified: ClassifiedPaths, cx: &mut Context<Self>) {
        let paths = classified.images;
        let skipped = classified.skipped;
        let jobs = if self.intake.as_ref().is_some_and(IntakeBatch::is_accepting) {
            self.intake
                .as_mut()
                .expect("accepting intake exists")
                .extend(paths, skipped)
        } else {
            self.intake_gen = self.intake_gen.wrapping_add(1);
            let batch = IntakeBatch::from_paths(paths, skipped, self.intake_gen);
            let jobs = batch.file_jobs();
            self.intake = Some(batch);
            jobs
        };
        self.ingest.file_queue.extend(jobs);
        cx.notify();
    }

    fn begin_intake_reject(&mut self, skipped: usize, cx: &mut Context<Self>) {
        self.intake_gen = self.intake_gen.wrapping_add(1);
        let gen = self.intake_gen;
        self.intake = Some(IntakeBatch::reject(skipped, gen));
        cx.notify();
        self.schedule_intake_dismiss(gen, INTAKE_REJECT_HOLD, cx);
    }

    fn schedule_intake_dismiss(&mut self, gen: u64, delay: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                if this.intake.as_ref().is_some_and(|batch| batch.gen == gen) {
                    this.intake = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn schedule_intake_feedback(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(gen) = self.intake.as_ref().map(|batch| batch.gen) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INTAKE_RESULT_HOLD).await;
            let _ = this.update(cx, |this, cx| {
                if !this.intake.as_ref().is_some_and(|batch| batch.gen == gen) {
                    return;
                }
                if this
                    .intake
                    .as_mut()
                    .is_some_and(|batch| batch.begin_leave(key))
                {
                    cx.notify();
                }
            });
            cx.background_executor().timer(INTAKE_LEAVE).await;
            let _ = this.update(cx, |this, cx| {
                if !this.intake.as_ref().is_some_and(|batch| batch.gen == gen) {
                    return;
                }
                let promoted = this
                    .intake
                    .as_mut()
                    .and_then(|batch| batch.mark_gone_and_promote(key));
                if let Some(promoted) = promoted {
                    this.schedule_intake_feedback(promoted, cx);
                }
                this.maybe_complete_intake(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn maybe_complete_intake(&mut self, cx: &mut Context<Self>) {
        let Some(batch) = &self.intake else {
            return;
        };
        if batch.phase != IntakeBatchPhase::Running || !batch.all_gone() {
            return;
        }
        let gen = batch.gen;
        if let Some(batch) = &mut self.intake {
            batch.phase = IntakeBatchPhase::Complete;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INTAKE_COMPLETE_HOLD).await;
            let _ =
                this.update(cx, |this, cx| {
                    let Some(batch) = this.intake.as_mut().filter(|batch| {
                        batch.gen == gen && batch.phase == IntakeBatchPhase::Complete
                    }) else {
                        return;
                    };
                    batch.phase = IntakeBatchPhase::Fading;
                    cx.notify();
                });
            cx.background_executor().timer(INTAKE_PLATEN_OUT).await;
            let _ = this.update(cx, |this, cx| {
                if this.intake.as_ref().is_some_and(|batch| {
                    batch.gen == gen && batch.phase == IntakeBatchPhase::Fading
                }) {
                    this.intake = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn intake(&self) -> Option<&IntakeBatch> {
        self.intake.as_ref()
    }

    pub(super) fn ingest_pixels(&mut self, image: RgbaImage, cx: &mut Context<Self>) -> Uuid {
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
        id
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
        let Some((gen, key, path)) = self.ingest.file_queue.pop_front() else {
            return;
        };
        self.ingest.file_loading = true;
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_spawn(async move { image::open(&path).map(|d| d.to_rgba8()) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.ingest.file_loading = false;
                let current = this.intake.as_ref().is_some_and(|batch| {
                    batch.gen == gen && batch.items.iter().any(|item| item.key == key)
                });
                if !current {
                    this.pump_file_ingest(cx);
                    return;
                }
                match decoded {
                    Ok(img) => {
                        let id = this.ingest_pixels(img, cx);
                        if let Some(batch) = &mut this.intake {
                            batch.bind_item(key, id);
                        }
                    }
                    Err(err) => {
                        eprintln!("{APP_SLUG}: open image: {err}");
                        let feedback = this
                            .intake
                            .as_mut()
                            .and_then(|batch| batch.finish_key(key, false));
                        this.flash_error("Couldn't open that image", cx);
                        if let Some(key) = feedback {
                            this.schedule_intake_feedback(key, cx);
                        }
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
        if let Some(batch) = &mut self.intake {
            batch.mark_working(id);
            cx.notify();
        }
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
            self.ingest.ocr.finish(id);
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
                this.schedule_engine_release(cx);
                let ready = this
                    .library
                    .get(id)
                    .is_some_and(|d| matches!(d.status, DocStatus::Ready));
                this.finish_intake_id(id, ready, cx);
                if ready {
                    this.persist_ready(id, cx);
                    if this.prefs.autocopy && this.library.selected() == Some(id) {
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
                }
                this.pump_ocr(cx);
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: ocr task: {err}");
            }
        })
        .detach();
    }

    fn finish_intake_id(&mut self, id: Uuid, ok: bool, cx: &mut Context<Self>) {
        let feedback = self
            .intake
            .as_mut()
            .and_then(|batch| batch.finish_id(id, ok));
        if let Some(key) = feedback {
            self.schedule_intake_feedback(key, cx);
        }
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

    pub fn copy_selected(&mut self, cx: &mut App) -> Option<(Uuid, CopyKind)> {
        let doc = self.selected_doc()?;
        let id = doc.id;
        let snip = doc.snip_kind();
        let kind = self.prefs.copy_habit.resolve(snip, self.export_fmt);
        let text = doc.text_for(kind, &self.prefs);
        if text.is_empty() {
            return None;
        }
        Self::write_clipboard(text, cx);
        self.record_copy_habit(snip, kind);
        Some((id, kind))
    }

    pub fn copy_chip_payload(&mut self, kind: CopyKind, text: String, cx: &mut App) {
        let Some(doc) = self.selected_doc() else {
            return;
        };
        let snip = doc.snip_kind();
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
        if !doc.has_ready_blocks() {
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
                        this.flash_error("Couldn't open a Word document", cx);
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
        self.finish_intake_id(id, false, cx);
        self.library.remove(id);
        self.ingest.drop_doc(id);
        if self.library.is_empty() {
            self.ingest.ocr.cancel_remaining();
        }
        if let Some(writer) = self.store_writer() {
            if let Err(err) = writer.delete(id) {
                eprintln!("{APP_SLUG}: queue delete snip: {err:#}");
                self.flash_error("Couldn't delete that snip", cx);
            }
        }
        if let Some(id) = self.library.selected() {
            self.ensure_detail(id, cx);
        }
        self.pump_ocr(cx);
        cx.notify();
    }

    pub fn wipe_library(&mut self, cx: &mut Context<Self>) {
        self.ingest.ocr.cancel_remaining();
        if let Some(id) = self.ingest.ocr.running() {
            self.ingest.ocr.remove(id);
        }
        self.ingest.file_queue.clear();
        self.intake = None;
        self.ingest.clear_docs();
        self.search.bump();
        self.library.clear();
        if let Some(writer) = self.store_writer() {
            if let Err(err) = writer.wipe() {
                eprintln!("{APP_SLUG}: queue wipe library: {err:#}");
                self.flash_error("Couldn't clear the snip library", cx);
            }
        }
        cx.notify();
    }

    pub fn request_thumb(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.ingest.thumb_blocked(id) {
            return;
        }
        let Some(doc) = self.library.get(id) else {
            return;
        };
        if !doc.thumb_jpeg.is_empty() || !doc.is_persisted() {
            return;
        }
        let Some(store) = self.store() else {
            return;
        };
        self.ingest.start_thumb(id);
        cx.spawn(async move |this, cx| {
            let jpeg = cx
                .background_spawn(async move { store.load_thumb(id) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.ingest.finish_thumb(id);
                match jpeg {
                    Ok(bytes) => {
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.thumb_jpeg = bytes;
                        }
                    }
                    Err(err) => {
                        this.ingest.fail_thumb(id);
                        eprintln!("{APP_SLUG}: load thumb: {err}");
                    }
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: thumb task: {err}");
            }
        })
        .detach();
    }

    pub fn thumbnail_failed(&self, id: Uuid) -> bool {
        self.ingest.thumb_failed(id)
    }

    pub fn mark_thumbnail_failed(&mut self, id: Uuid) {
        self.ingest.fail_thumb(id);
        if let Some(doc) = self.library.get_mut(id) {
            doc.thumb_jpeg.clear();
        }
    }

    fn persist_ready(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(writer) = self.store_writer() else {
            return;
        };
        let Some(doc) = self.library.get(id) else {
            return;
        };
        if !matches!(doc.status, DocStatus::Ready) {
            return;
        }
        let revision = doc.revision;
        let stored = match doc.persist {
            PersistState::New => false,
            PersistState::InsertPending { .. } => return,
            PersistState::Stored => true,
        };
        if !stored && doc.image.pixels().is_none() {
            return;
        }
        if !stored {
            if let Some(doc) = self.library.get_mut(id) {
                doc.persist = PersistState::InsertPending { revision };
            }
        }
        let Some(doc) = self.library.get(id).cloned() else {
            return;
        };
        let queued = if stored {
            writer.update_ocr(doc)
        } else {
            writer.insert(doc)
        };
        if let Err(err) = queued {
            if let Some(d) = self.library.get_mut(id) {
                if d.insert_pending_revision() == Some(revision) {
                    d.persist = PersistState::New;
                }
            }
            eprintln!("{APP_SLUG}: queue persist snip: {err:#}");
            self.flash_error("Couldn't save that snip", cx);
            self.schedule_persist_retry(id, cx);
        }
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
        let Some(writer) = self.store_writer() else {
            return;
        };
        let Some(doc) = self.library.get(id).cloned() else {
            return;
        };
        if !doc.is_persisted() || !matches!(doc.status, DocStatus::Ready) {
            return;
        }
        if let Err(err) = writer.update_blocks(doc) {
            eprintln!("{APP_SLUG}: queue persist edits: {err:#}");
            self.flash_error("Couldn't save those edits", cx);
            self.schedule_persist_retry(id, cx);
        }
    }

    fn load_png_then_retry(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.store() else {
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
        let need_blocks = doc.is_persisted() && !doc.blocks_loaded;
        let need_png = matches!(doc.image, ImageSlot::OnDisk);
        if need_blocks {
            self.load_blocks(id, cx);
        }
        if need_png {
            self.load_png(id, cx);
        }
    }

    fn load_blocks(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.ingest.start_blocks(id) {
            return;
        }
        let Some(store) = self.store() else {
            self.ingest.finish_blocks(id);
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.load_block_pair(id) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.ingest.finish_blocks(id);
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
        if !self.ingest.start_png(id) {
            return;
        }
        let Some(store) = self.store() else {
            self.ingest.finish_png(id);
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { store.load_png(id) }).await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.ingest.finish_png(id);
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

    pub(super) fn pump_store_events(
        &mut self,
        mut events: std::sync::mpsc::Receiver<WriteEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| loop {
            let (next, event) = cx
                .background_spawn(async move {
                    let event = events.recv();
                    (events, event)
                })
                .await;
            events = next;
            let Ok(event) = event else {
                break;
            };
            if this
                .update(cx, |this, cx| this.handle_store_event(event, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    fn handle_store_event(&mut self, event: WriteEvent, cx: &mut Context<Self>) {
        match (event.kind, event.result) {
            (WriteKind::Insert { id, revision }, Ok(WriteResult::Inserted(thumb))) => {
                let mut changed_during_insert = false;
                if let Some(doc) = self.library.get_mut(id) {
                    if doc.insert_pending_revision() == Some(revision) {
                        changed_during_insert = doc.revision != revision;
                        doc.persist = PersistState::Stored;
                        doc.thumb_jpeg = thumb;
                    }
                }
                self.library.touch_lru(id);
                self.ingest.clear_persist_retry(id);
                if changed_during_insert {
                    self.persist_edits(id, cx);
                }
                self.schedule_filter(cx);
            }
            (WriteKind::Insert { id, revision }, Err(err)) => {
                if let Some(doc) = self.library.get_mut(id) {
                    if doc.insert_pending_revision() == Some(revision) {
                        doc.persist = PersistState::New;
                    }
                }
                eprintln!("{APP_SLUG}: persist snip: {err:#}");
                self.flash_error("Couldn't save that snip", cx);
                self.schedule_persist_retry(id, cx);
            }
            (WriteKind::UpdateOcr { id }, Err(err)) => {
                eprintln!("{APP_SLUG}: persist OCR {id}: {err:#}");
                self.flash_error("Couldn't update that snip", cx);
                self.schedule_persist_retry(id, cx);
            }
            (WriteKind::UpdateBlocks { id }, Err(err)) => {
                eprintln!("{APP_SLUG}: persist edits {id}: {err:#}");
                self.flash_error("Couldn't save those edits", cx);
                self.schedule_persist_retry(id, cx);
            }
            (WriteKind::Delete { id }, Err(err)) => {
                eprintln!("{APP_SLUG}: delete snip {id}: {err:#}");
                self.flash_error("Couldn't remove all snip files", cx);
            }
            (WriteKind::Wipe, Err(err)) => {
                eprintln!("{APP_SLUG}: wipe library: {err:#}");
                self.flash_error("Couldn't clear the snip library", cx);
            }
            (
                WriteKind::UpdateOcr { id } | WriteKind::UpdateBlocks { id },
                Ok(WriteResult::Done),
            ) => {
                self.ingest.clear_persist_retry(id);
            }
            (_, Ok(WriteResult::Done)) => {}
            (kind, Ok(result)) => {
                eprintln!("{APP_SLUG}: unexpected store result {kind:?}: {result:?}");
            }
        }
        cx.notify();
    }

    fn schedule_persist_retry(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(delay) = self.ingest.begin_persist_retry(id) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                let still_pending = this.ingest.finish_persist_retry(id);
                if still_pending && this.library.get(id).is_some() {
                    this.persist_ready(id, cx);
                }
            });
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

    #[test]
    fn classify_image_paths_keeps_images_and_counts_skips() {
        let classified = classify_image_paths([
            PathBuf::from("board.PNG"),
            PathBuf::from("eq-navier.jpg"),
            PathBuf::from("table-3.webp"),
            PathBuf::from("notes.pdf"),
            PathBuf::from("slides.pptx"),
            PathBuf::from("noext"),
        ]);
        assert_eq!(
            classified.images,
            vec![
                PathBuf::from("board.PNG"),
                PathBuf::from("eq-navier.jpg"),
                PathBuf::from("table-3.webp"),
            ]
        );
        assert_eq!(classified.skipped, 3);
        assert_eq!(
            classified.counts(),
            IntakeCounts {
                images: 3,
                skipped: 3
            }
        );
    }

    #[test]
    fn classify_image_paths_empty_and_reject_only() {
        let empty = classify_image_paths(Vec::<PathBuf>::new());
        assert!(empty.images.is_empty());
        assert_eq!(empty.skipped, 0);
        let reject = classify_image_paths([PathBuf::from("a.pdf"), PathBuf::from("b.txt")]);
        assert!(reject.images.is_empty());
        assert_eq!(reject.skipped, 2);
        assert!(!is_ingest_image_path(Path::new("a.gif")));
        assert!(!is_ingest_image_path(Path::new("a.bmp")));
        assert!(is_ingest_image_path(Path::new("a.jpeg")));
    }

    #[test]
    fn intake_bind_and_finish_tracks_progress() {
        let mut batch =
            IntakeBatch::from_paths(vec![PathBuf::from("a.png"), PathBuf::from("b.png")], 1, 1);
        assert_eq!(
            batch.counts(),
            IntakeCounts {
                images: 2,
                skipped: 1
            }
        );
        let a = Uuid::from_u128(1);
        batch.bind_item(1, a);
        assert_eq!(batch.items[0].work, IntakeWork::Waiting);
        assert_eq!(batch.items[0].visual, IntakeVisual::Visible);
        assert_eq!(batch.thumb_ids(), vec![a]);
        batch.mark_working(a);
        assert_eq!(batch.items[0].work, IntakeWork::Working);
        assert_eq!(batch.finish_id(a, true), Some(1));
        assert_eq!(batch.items[0].work, IntakeWork::Succeeded);
        assert_eq!(batch.items[0].visual, IntakeVisual::Settled);
        assert_eq!(batch.progress(), (1, 2));
        assert_eq!(batch.finish_id(Uuid::from_u128(99), true), None);
        assert_eq!(batch.finish_key(2, false), Some(2));
        assert_eq!(batch.results(), (1, 1));
        assert!(!batch.all_gone());
        assert_eq!(batch.progress(), (2, 2));
    }

    #[test]
    fn intake_gone_item_refills_only_its_fixed_slot() {
        let paths: Vec<PathBuf> = (0..8).map(|i| PathBuf::from(format!("{i}.png"))).collect();
        let mut batch = IntakeBatch::from_paths(paths, 0, 1);
        assert_eq!(batch.visible().len(), INTAKE_WINDOW);
        assert_eq!(batch.backlog_count(), 3);
        assert_eq!(batch.finish_key(1, true), Some(1));
        assert!(batch.begin_leave(1));
        assert_eq!(batch.mark_gone_and_promote(1), None);
        let names: Vec<_> = batch
            .visible()
            .iter()
            .map(|(slot, item)| (*slot, item.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                (0, "5.png"),
                (1, "1.png"),
                (2, "2.png"),
                (3, "3.png"),
                (4, "4.png")
            ]
        );
        assert_eq!(batch.backlog_count(), 2);
        assert_eq!(batch.items[1].slot, Some(1));
        assert_eq!(batch.items[2].slot, Some(2));
        assert_eq!(batch.items[3].slot, Some(3));
        assert_eq!(batch.items[4].slot, Some(4));
    }

    #[test]
    fn completed_backlog_item_gets_feedback_after_promotion() {
        let paths: Vec<PathBuf> = (0..6).map(|i| PathBuf::from(format!("{i}.png"))).collect();
        let mut batch = IntakeBatch::from_paths(paths, 0, 7);
        assert_eq!(batch.finish_key(6, true), None);
        assert_eq!(batch.items[5].visual, IntakeVisual::Backlog);
        assert_eq!(batch.finish_key(1, true), Some(1));
        assert!(batch.begin_leave(1));
        assert_eq!(batch.mark_gone_and_promote(1), Some(6));
        assert_eq!(batch.items[5].slot, Some(0));
        assert_eq!(batch.items[5].visual, IntakeVisual::Settled);
        assert!(batch.begin_leave(6));
        assert_eq!(batch.mark_gone_and_promote(6), None);
    }

    #[test]
    fn intake_feedback_is_independent_per_slot() {
        let paths: Vec<PathBuf> = (0..7).map(|i| PathBuf::from(format!("{i}.png"))).collect();
        let mut batch = IntakeBatch::from_paths(paths, 0, 3);
        assert_eq!(batch.finish_key(1, true), Some(1));
        assert_eq!(batch.finish_key(2, true), Some(2));
        assert!(batch.begin_leave(1));
        assert!(batch.begin_leave(2));
        assert_eq!(batch.mark_gone_and_promote(2), None);
        assert_eq!(batch.mark_gone_and_promote(1), None);
        assert_eq!(batch.items[5].slot, Some(1));
        assert_eq!(batch.items[6].slot, Some(0));
        assert_eq!(batch.items[2].slot, Some(2));
    }

    #[test]
    fn intake_extend_keeps_generation_and_fills_vacant_slots() {
        let mut batch = IntakeBatch::from_paths(vec![PathBuf::from("a.png")], 0, 1);
        let added = batch.extend(vec![PathBuf::from("a.png"), PathBuf::from("c.png")], 2);
        assert_eq!(added, vec![(1, 2, PathBuf::from("c.png"))]);
        assert_eq!(batch.skipped, 2);
        assert_eq!(batch.items.len(), 2);
        assert_eq!(batch.items[1].slot, Some(1));
        assert_eq!(batch.gen, 1);
        assert!(batch.is_accepting());
    }

    #[test]
    fn intake_reject_has_no_items_and_is_not_accepting() {
        let batch = IntakeBatch::reject(3, 1);
        assert!(batch.items.is_empty());
        assert_eq!(batch.skipped, 3);
        assert!(!batch.is_accepting());
        assert!(batch.visible().is_empty());
    }
}
