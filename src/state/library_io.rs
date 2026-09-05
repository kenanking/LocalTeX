use std::sync::Arc;

use gpui::{AppContext, Context};
use uuid::Uuid;

use crate::doc::{Block, DocStatus, ImageSlot, PersistState};
use crate::identity::APP_SLUG;
use crate::store::{WriteEvent, WriteKind, WriteResult};

use super::AppState;

impl AppState {
    pub(super) fn resume_pending_work(&mut self, cx: &mut Context<Self>) {
        let pending: Vec<_> = self
            .library
            .iter_all()
            .filter_map(|doc| {
                doc.source_pending
                    .then(|| {
                        doc.raw_text
                            .clone()
                            .map(|text| (doc.id, doc.revision, text))
                    })
                    .flatten()
            })
            .collect();
        for (id, revision, text) in pending {
            let prefs = self.prefs.clone();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_spawn(async move { crate::source::parse_source(&text, &prefs) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.apply_parsed_source(id, revision, result, cx);
                });
            })
            .detach();
        }
        for doc in self.library.iter_all() {
            if doc.is_persisted() && matches!(doc.status, DocStatus::Recognizing) {
                self.ocr.enqueue(doc.id);
            }
        }
        self.pump_ocr(cx);
    }

    pub fn request_thumb(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.documents.thumb_blocked(id) {
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
        self.documents.start_thumb(id);
        cx.spawn(async move |this, cx| {
            let jpeg = cx
                .background_spawn(async move { store.load_thumb(id) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.documents.finish_thumb(id);
                match jpeg {
                    Ok(bytes) => {
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.thumb_jpeg = bytes;
                        }
                    }
                    Err(err) => {
                        this.documents.fail_thumb(id);
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
        self.documents.thumb_failed(id)
    }

    pub fn mark_thumbnail_failed(&mut self, id: Uuid) {
        self.documents.fail_thumb(id);
        if let Some(doc) = self.library.get_mut(id) {
            doc.thumb_jpeg.clear();
        }
    }

    pub(super) fn persist_ready(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(writer) = self.store_writer() else {
            return;
        };
        let Some(doc) = self.library.get(id) else {
            return;
        };
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

    pub fn save_source(&mut self, id: Uuid, text: String, cx: &mut Context<Self>) -> Option<u64> {
        if self.is_shutting_down() {
            return None;
        }
        let doc = self.library.get_mut(id)?;
        doc.raw_text = Some(text);
        doc.source_error = None;
        doc.source_pending = true;
        doc.source_updated_at = Some(std::time::Instant::now());
        doc.refresh_first_line();
        doc.bump_revision();
        let revision = doc.revision;
        self.persist_edits(id, cx);
        self.schedule_filter(cx);
        cx.notify();
        Some(revision)
    }

    pub fn apply_parsed_source(
        &mut self,
        id: Uuid,
        revision: u64,
        result: Result<Vec<Block>, crate::source::ParseError>,
        cx: &mut Context<Self>,
    ) {
        if self.is_shutting_down() {
            return;
        }
        if let Some(doc) = self.library.get_mut(id) {
            if doc.revision != revision {
                return;
            }
            doc.source_pending = false;
            match result {
                Ok(blocks) => {
                    doc.blocks = blocks;
                    doc.source_error = None;
                }
                Err(error) => {
                    doc.blocks.clear();
                    doc.source_error = Some(error.to_string());
                }
            }
            doc.refresh_first_line();
            doc.bump_revision();
        }
        self.persist_edits(id, cx);
        self.schedule_filter(cx);
        cx.notify();
    }

    pub fn revert_ocr(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.is_shutting_down() {
            return;
        }
        let Some(doc) = self.library.get_mut(id) else {
            return;
        };
        if !doc.is_edited() {
            return;
        }
        doc.blocks = doc.ocr_blocks.clone();
        doc.raw_text = None;
        doc.source_pending = false;
        doc.source_updated_at = None;
        doc.source_error = None;
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
        let Some(doc) = self.library.get(id) else {
            return;
        };
        if matches!(doc.persist, PersistState::New) {
            self.persist_ready(id, cx);
            return;
        }
        if let Err(err) = writer.update_blocks(doc) {
            eprintln!("{APP_SLUG}: queue persist edits: {err:#}");
            self.flash_error("Couldn't save those edits", cx);
            self.schedule_persist_retry(id, cx);
        }
    }

    pub(super) fn load_png_then_retry(&mut self, id: Uuid, cx: &mut Context<Self>) {
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
        if !self.documents.start_blocks(id) {
            return;
        }
        let Some(store) = self.store() else {
            self.documents.finish_blocks(id);
            return;
        };
        let prefs = self.prefs.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let (mut blocks, ocr_blocks) = store.load_block_pair(id)?;
                    let raw = store.load_source(id)?;
                    let mut error = None;
                    if let Some(text) = &raw {
                        match crate::source::parse_source(text, &prefs) {
                            Ok(parsed) => blocks = parsed,
                            Err(err) => {
                                blocks.clear();
                                error = Some(err.to_string());
                            }
                        }
                    }
                    anyhow::Ok((blocks, ocr_blocks, raw, error))
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.documents.finish_blocks(id);
                match result {
                    Ok((blocks, ocr_blocks, raw, error)) => {
                        if let Some(doc) = this.library.get_mut(id) {
                            doc.blocks = blocks;
                            doc.ocr_blocks = ocr_blocks;
                            doc.raw_text = raw;
                            doc.source_error = error;
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
        if !self.documents.start_png(id) {
            return;
        }
        let Some(store) = self.store() else {
            self.documents.finish_png(id);
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { store.load_png(id) }).await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.documents.finish_png(id);
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
                self.documents.clear_persist_retry(id);
                if changed_during_insert {
                    self.persist_edits(id, cx);
                }
                if self
                    .library
                    .get(id)
                    .is_some_and(|doc| matches!(doc.status, DocStatus::Recognizing))
                {
                    self.enqueue_ocr(id, cx);
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
            (WriteKind::UpdateOcr { id }, Ok(WriteResult::Done)) => {
                self.documents.clear_persist_retry(id);
            }
            (_, Ok(WriteResult::Done)) => {}
            (kind, Ok(result)) => {
                eprintln!("{APP_SLUG}: unexpected store result {kind:?}: {result:?}");
            }
        }
        cx.notify();
    }

    fn schedule_persist_retry(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(delay) = self.documents.begin_persist_retry(id) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                let still_pending = this.documents.finish_persist_retry(id);
                if still_pending && this.library.get(id).is_some() {
                    this.persist_ready(id, cx);
                }
            });
        })
        .detach();
    }
}
