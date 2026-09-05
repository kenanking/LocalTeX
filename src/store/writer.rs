use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use uuid::Uuid;

use super::{ContentUpdate, Store};
use crate::doc::{Document, ImageSlot};

#[derive(Debug)]
pub enum WriteResult {
    Inserted(Vec<u8>),
    Done,
}

#[derive(Debug, Clone, Copy)]
pub enum WriteKind {
    Insert { id: Uuid, revision: u64 },
    UpdateOcr { id: Uuid },
    UpdateBlocks { id: Uuid },
    Delete { id: Uuid },
    Wipe,
}

#[derive(Debug)]
pub struct WriteEvent {
    pub kind: WriteKind,
    pub result: anyhow::Result<WriteResult>,
}

enum WriteCommand {
    Insert(Document),
    UpdateOcr(Document),
    UpdateBlocks(ContentUpdate),
    Delete(Uuid),
    Wipe,
}

enum Request {
    Write {
        command: Box<WriteCommand>,
        kind: WriteKind,
    },
    Flush(Sender<anyhow::Result<()>>),
    Shutdown(Sender<anyhow::Result<()>>),
}

#[derive(Clone)]
pub struct StoreWriter {
    tx: Sender<Request>,
    thread: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl StoreWriter {
    pub fn start(store: Arc<Store>) -> anyhow::Result<(Self, Receiver<WriteEvent>)> {
        let (tx, rx) = mpsc::channel::<Request>();
        let (events_tx, events_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("localtex-store-writer".into())
            .spawn(move || run(store, rx, events_tx))
            .map_err(|err| anyhow::anyhow!("spawn store writer: {err}"))?;
        Ok((
            Self {
                tx,
                thread: Arc::new(Mutex::new(Some(thread))),
            },
            events_rx,
        ))
    }

    pub fn insert(&self, doc: Document) -> anyhow::Result<()> {
        let kind = WriteKind::Insert {
            id: doc.id,
            revision: doc.revision,
        };
        self.submit(WriteCommand::Insert(doc), kind)
    }

    pub fn update_ocr(&self, doc: Document) -> anyhow::Result<()> {
        let kind = WriteKind::UpdateOcr { id: doc.id };
        self.submit(WriteCommand::UpdateOcr(without_image_payload(doc)), kind)
    }

    pub fn update_blocks(&self, doc: &Document) -> anyhow::Result<()> {
        let kind = WriteKind::UpdateBlocks { id: doc.id };
        self.submit(WriteCommand::UpdateBlocks(ContentUpdate::from(doc)), kind)
    }

    pub fn delete(&self, id: Uuid) -> anyhow::Result<()> {
        self.submit(WriteCommand::Delete(id), WriteKind::Delete { id })
    }

    pub fn wipe(&self) -> anyhow::Result<()> {
        self.submit(WriteCommand::Wipe, WriteKind::Wipe)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.flush_timeout(std::time::Duration::from_secs(30))
    }

    pub fn flush_timeout(&self, timeout: std::time::Duration) -> anyhow::Result<()> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Request::Flush(reply))
            .map_err(|_| anyhow::anyhow!("store writer stopped"))?;
        rx.recv_timeout(timeout)
            .map_err(|err| anyhow::anyhow!("store flush: {err}"))?
    }

    /// Drain all previously submitted writes, stop the writer, and join it.
    /// Call this from a background thread; it may wait on disk I/O.
    pub fn shutdown(&self) -> anyhow::Result<()> {
        let mut thread_slot = self
            .thread
            .lock()
            .map_err(|_| anyhow::anyhow!("store writer join lock"))?;
        if thread_slot.is_none() {
            return Ok(());
        }
        self.flush()?;
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Request::Shutdown(reply))
            .map_err(|_| anyhow::anyhow!("store writer stopped"))?;
        recv_ack(rx)?;
        let thread = thread_slot.take();
        if let Some(thread) = thread {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("store writer panicked"))?;
        }
        Ok(())
    }

    fn submit(&self, command: WriteCommand, kind: WriteKind) -> anyhow::Result<()> {
        self.tx
            .send(Request::Write {
                command: Box::new(command),
                kind,
            })
            .map_err(|_| anyhow::anyhow!("store writer stopped"))
    }
}

fn without_image_payload(mut doc: Document) -> Document {
    doc.image = ImageSlot::OnDisk;
    doc.thumb_jpeg.clear();
    doc.ink = None;
    doc
}

fn run(store: Arc<Store>, rx: Receiver<Request>, events: Sender<WriteEvent>) {
    let mut failures = std::collections::HashMap::new();
    let mut pending = None;
    while let Some(request) = pending.take().or_else(|| rx.recv().ok()) {
        match request {
            Request::Write { mut command, kind } => {
                if let WriteKind::UpdateBlocks { id } = kind {
                    while let Ok(next) = rx.try_recv() {
                        match next {
                            Request::Write {
                                command: newer,
                                kind: WriteKind::UpdateBlocks { id: next_id },
                            } if next_id == id => command = newer,
                            barrier => {
                                pending = Some(barrier);
                                break;
                            }
                        }
                    }
                }
                let result = match *command {
                    WriteCommand::Insert(doc) => {
                        store.insert_ready(&doc).map(WriteResult::Inserted)
                    }
                    WriteCommand::UpdateOcr(doc) => {
                        store.update_ocr(&doc).map(|()| WriteResult::Done)
                    }
                    WriteCommand::UpdateBlocks(doc) => {
                        store.update_blocks(&doc).map(|()| WriteResult::Done)
                    }
                    WriteCommand::Delete(id) => store.delete(id).map(|()| WriteResult::Done),
                    WriteCommand::Wipe => store.wipe().map(|()| WriteResult::Done),
                };
                let key = match kind {
                    WriteKind::Insert { id, .. } => (id, 0),
                    WriteKind::UpdateOcr { id } => (id, 1),
                    WriteKind::UpdateBlocks { id } => (id, 2),
                    WriteKind::Delete { id } => (id, 3),
                    WriteKind::Wipe => (Uuid::nil(), 4),
                };
                match &result {
                    Err(err) => {
                        failures.insert(key, format!("{err:#}"));
                    }
                    Ok(_) => {
                        failures.remove(&key);
                        if let WriteKind::UpdateOcr { id } = kind {
                            failures.remove(&(id, 2));
                        }
                        if let WriteKind::Delete { id } = kind {
                            failures.retain(|(failed_id, _), _| *failed_id != id);
                        }
                        if matches!(kind, WriteKind::Wipe) {
                            failures.clear();
                        }
                    }
                }
                let _ = events.send(WriteEvent { kind, result });
            }
            Request::Flush(reply) => {
                let result = if failures.is_empty() {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Unresolved writes: {:?}", failures))
                };
                let _ = reply.send(result);
            }
            Request::Shutdown(reply) => {
                let _ = reply.send(Ok(()));
                break;
            }
        }
    }
}

fn recv_ack(reply: Receiver<anyhow::Result<()>>) -> anyhow::Result<()> {
    reply
        .recv()
        .map_err(|_| anyhow::anyhow!("store writer stopped"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Block, BlockKind, DocStatus, ImageSlot, PersistState, Rect};
    use image::{Rgba, RgbaImage};
    use std::time::SystemTime;

    fn ready_doc(text: &str) -> Document {
        let image = RgbaImage::from_pixel(8, 8, Rgba([10, 20, 30, 255]));
        let blocks = vec![Block::new(
            BlockKind::Formula,
            Rect {
                x: 0,
                y: 0,
                w: 8,
                h: 8,
            },
            text,
        )];
        Document {
            id: Uuid::new_v4(),
            created_at: SystemTime::now(),
            image: ImageSlot::Loaded(Arc::new(image)),
            blocks: blocks.clone(),
            status: DocStatus::Ready,
            first_line: String::new(),
            thumb_jpeg: Vec::new(),
            persist: PersistState::New,
            blocks_loaded: true,
            ocr: None,
            ink: None,
            revision: 0,
            ocr_blocks: blocks,
            raw_text: None,
            source_error: None,
            source_pending: false,
            source_updated_at: None,
        }
    }

    #[test]
    fn commands_are_applied_in_submission_order() {
        let root = std::env::temp_dir().join(format!("localtex-writer-{}", Uuid::new_v4()));
        let store = Arc::new(Store::open(root.clone()).unwrap());
        let (writer, events) = StoreWriter::start(store.clone()).unwrap();
        let mut doc = ready_doc("before");
        let id = doc.id;

        writer.insert(doc.clone()).unwrap();
        doc.blocks[0].text = "after".into();
        writer.update_blocks(&doc).unwrap();
        assert!(matches!(
            events.recv().unwrap(),
            WriteEvent {
                kind: WriteKind::Insert { .. },
                result: Ok(WriteResult::Inserted(_))
            }
        ));
        assert!(matches!(
            events.recv().unwrap(),
            WriteEvent {
                kind: WriteKind::UpdateBlocks { .. },
                result: Ok(WriteResult::Done)
            }
        ));
        assert_eq!(store.load_blocks(id).unwrap()[0].text, "after");

        writer.delete(id).unwrap();
        assert!(matches!(
            events.recv().unwrap(),
            WriteEvent {
                kind: WriteKind::Delete { .. },
                result: Ok(WriteResult::Done)
            }
        ));
        assert!(store.list().unwrap().is_empty());

        writer.shutdown().unwrap();
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flush_and_shutdown_wait_for_prior_writes() {
        let root = std::env::temp_dir().join(format!("localtex-writer-{}", Uuid::new_v4()));
        let store = Arc::new(Store::open(root.clone()).unwrap());
        let (writer, events) = StoreWriter::start(store.clone()).unwrap();
        let id = ready_doc("queued").id;
        let mut doc = ready_doc("queued");
        doc.id = id;
        writer.insert(doc).unwrap();

        writer.flush().unwrap();
        assert!(matches!(
            events.recv().unwrap().result,
            Ok(WriteResult::Inserted(_))
        ));
        assert_eq!(store.list().unwrap().len(), 1);
        writer.shutdown().unwrap();
        writer.shutdown().unwrap();
        assert!(writer.flush().is_err());

        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coalescing_preserves_flush_delete_and_insert_barriers() {
        let root = std::env::temp_dir().join(format!("localtex-writer-{}", Uuid::new_v4()));
        let store = Arc::new(Store::open(root.clone()).unwrap());
        let mut doc = ready_doc("original");
        store.insert_ready(&doc).unwrap();
        let (tx, rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let update = |text: &str| {
            let mut content = ContentUpdate::from(&doc);
            content.raw_text = Some(text.into());
            Request::Write {
                command: Box::new(WriteCommand::UpdateBlocks(content)),
                kind: WriteKind::UpdateBlocks { id: doc.id },
            }
        };
        tx.send(update("first")).unwrap();
        tx.send(update("second")).unwrap();
        let (flush_tx, flush_rx) = mpsc::channel();
        tx.send(Request::Flush(flush_tx)).unwrap();
        tx.send(update("third")).unwrap();
        tx.send(Request::Write {
            command: Box::new(WriteCommand::Delete(doc.id)),
            kind: WriteKind::Delete { id: doc.id },
        })
        .unwrap();
        doc.raw_text = Some("replacement".into());
        tx.send(Request::Write {
            command: Box::new(WriteCommand::Insert(doc.clone())),
            kind: WriteKind::Insert {
                id: doc.id,
                revision: 0,
            },
        })
        .unwrap();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        tx.send(Request::Shutdown(shutdown_tx)).unwrap();
        run(store.clone(), rx, events_tx);
        flush_rx.recv().unwrap().unwrap();
        shutdown_rx.recv().unwrap().unwrap();
        let events: Vec<_> = events.into_iter().collect();
        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|event| event.result.is_ok()));
        assert!(matches!(events[0].kind, WriteKind::UpdateBlocks { .. }));
        assert!(matches!(events[1].kind, WriteKind::UpdateBlocks { .. }));
        assert!(matches!(events[2].kind, WriteKind::Delete { .. }));
        assert!(matches!(events[3].kind, WriteKind::Insert { .. }));
        assert_eq!(
            store.load_source(doc.id).unwrap().as_deref(),
            Some("replacement")
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flush_reports_failed_insert_until_retry_succeeds() {
        let root = std::env::temp_dir().join(format!("localtex-writer-{}", Uuid::new_v4()));
        let store = Arc::new(Store::open(root.clone()).unwrap());
        let (writer, events) = StoreWriter::start(store.clone()).unwrap();
        let doc = ready_doc("saved after retry");
        let mut missing = doc.clone();
        missing.image = ImageSlot::Missing;
        writer.insert(missing).unwrap();
        assert!(events.recv().unwrap().result.is_err());
        assert!(writer.flush().is_err());
        assert!(writer.shutdown().is_err());
        writer.insert(doc).unwrap();
        writer.flush().unwrap();
        writer.shutdown().unwrap();
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
