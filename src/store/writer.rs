use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use uuid::Uuid;

use super::Store;
use crate::doc::Document;

#[derive(Debug)]
pub enum WriteResult {
    Inserted(Vec<u8>),
    Done,
}

enum WriteCommand {
    Insert(Document),
    UpdateOcr(Document),
    UpdateBlocks(Document),
    Delete(Uuid),
    Wipe,
}

struct Request {
    command: WriteCommand,
    reply: Sender<anyhow::Result<WriteResult>>,
}

#[derive(Clone)]
pub struct StoreWriter {
    tx: Sender<Request>,
}

impl StoreWriter {
    pub fn start(store: Arc<Store>) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("localtex-store-writer".into())
            .spawn(move || run(store, rx))
            .map_err(|err| anyhow::anyhow!("spawn store writer: {err}"))?;
        Ok(Self { tx })
    }

    pub fn insert(&self, doc: Document) -> Receiver<anyhow::Result<WriteResult>> {
        self.submit(WriteCommand::Insert(doc))
    }

    pub fn update_ocr(&self, doc: Document) -> Receiver<anyhow::Result<WriteResult>> {
        self.submit(WriteCommand::UpdateOcr(doc))
    }

    pub fn update_blocks(&self, doc: Document) -> Receiver<anyhow::Result<WriteResult>> {
        self.submit(WriteCommand::UpdateBlocks(doc))
    }

    pub fn delete(&self, id: Uuid) -> Receiver<anyhow::Result<WriteResult>> {
        self.submit(WriteCommand::Delete(id))
    }

    pub fn wipe(&self) -> Receiver<anyhow::Result<WriteResult>> {
        self.submit(WriteCommand::Wipe)
    }

    fn submit(&self, command: WriteCommand) -> Receiver<anyhow::Result<WriteResult>> {
        let (reply, rx) = mpsc::channel();
        if let Err(err) = self.tx.send(Request { command, reply }) {
            let Request { reply, .. } = err.0;
            let _ = reply.send(Err(anyhow::anyhow!("store writer stopped")));
        }
        rx
    }
}

fn run(store: Arc<Store>, rx: Receiver<Request>) {
    while let Ok(request) = rx.recv() {
        let result = match request.command {
            WriteCommand::Insert(doc) => store.insert_ready(&doc).map(WriteResult::Inserted),
            WriteCommand::UpdateOcr(doc) => store.update_ocr(&doc).map(|()| WriteResult::Done),
            WriteCommand::UpdateBlocks(doc) => {
                store.update_blocks(&doc).map(|()| WriteResult::Done)
            }
            WriteCommand::Delete(id) => store.delete(id).map(|()| WriteResult::Done),
            WriteCommand::Wipe => store.wipe().map(|()| WriteResult::Done),
        };
        let _ = request.reply.send(result);
    }
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
        }
    }

    #[test]
    fn commands_are_applied_in_submission_order() {
        let root = std::env::temp_dir().join(format!("localtex-writer-{}", Uuid::new_v4()));
        let store = Arc::new(Store::open(root.clone()).unwrap());
        let writer = StoreWriter::start(store.clone()).unwrap();
        let mut doc = ready_doc("before");
        let id = doc.id;

        let inserted = writer.insert(doc.clone());
        doc.blocks[0].text = "after".into();
        let updated = writer.update_blocks(doc);
        assert!(matches!(
            inserted.recv().unwrap().unwrap(),
            WriteResult::Inserted(_)
        ));
        assert!(matches!(
            updated.recv().unwrap().unwrap(),
            WriteResult::Done
        ));
        assert_eq!(store.load_blocks(id).unwrap()[0].text, "after");

        let deleted = writer.delete(id);
        assert!(matches!(
            deleted.recv().unwrap().unwrap(),
            WriteResult::Done
        ));
        assert!(store.list().unwrap().is_empty());

        drop(writer);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
