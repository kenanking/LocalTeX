use std::collections::HashSet;
use std::path::{Path, PathBuf};

use uuid::Uuid;

pub(crate) const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntakeItem {
    pub key: u64,
    pub path: PathBuf,
    pub name: String,
    pub id: Option<Uuid>,
    pub work: IntakeWork,
}

impl IntakeItem {
    fn waiting(key: u64, path: PathBuf) -> Self {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image")
            .to_string();
        Self {
            key,
            path,
            name,
            id: None,
            work: IntakeWork::Waiting,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntakeBatch {
    pub items: Vec<IntakeItem>,
    pub skipped: usize,
    pub generation: u64,
    next_item_key: u64,
}

impl IntakeBatch {
    pub fn from_paths(paths: Vec<PathBuf>, skipped: usize, generation: u64) -> Self {
        let items: Vec<_> = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| IntakeItem::waiting(index as u64 + 1, path))
            .collect();
        Self {
            next_item_key: items.len() as u64 + 1,
            items,
            skipped,
            generation,
        }
    }

    pub fn reject(skipped: usize, generation: u64) -> Self {
        Self {
            items: Vec::new(),
            skipped,
            generation,
            next_item_key: 1,
        }
    }

    pub fn is_accepting(&self) -> bool {
        !self.items.is_empty() && !self.all_terminal()
    }

    pub fn counts(&self) -> IntakeCounts {
        IntakeCounts {
            images: self.items.len(),
            skipped: self.skipped,
        }
    }

    pub fn all_terminal(&self) -> bool {
        !self.items.is_empty() && self.items.iter().all(|item| item.work.is_terminal())
    }

    pub fn extend(&mut self, paths: Vec<PathBuf>, skipped: usize) {
        self.skipped = self.skipped.saturating_add(skipped);
        let mut active: HashSet<_> = self
            .items
            .iter()
            .filter(|item| !item.work.is_terminal())
            .map(|item| item.path.clone())
            .collect();
        for path in paths {
            if !active.insert(path.clone()) {
                continue;
            }
            let key = self.next_item_key;
            self.next_item_key = self.next_item_key.wrapping_add(1);
            self.items.push(IntakeItem::waiting(key, path));
        }
    }

    pub fn next_pending(&self) -> Option<(u64, PathBuf)> {
        self.items
            .iter()
            .find(|item| item.id.is_none() && item.work == IntakeWork::Waiting)
            .map(|item| (item.key, item.path.clone()))
    }

    pub fn bind_item(&mut self, key: u64, id: Uuid) {
        if let Some(item) = self.items.iter_mut().find(|item| item.key == key) {
            item.id = Some(id);
        }
    }

    pub fn mark_working(&mut self, id: Uuid) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == Some(id))
            && item.work == IntakeWork::Waiting
        {
            item.work = IntakeWork::Working;
        }
    }

    pub fn finish_key(&mut self, key: u64, ok: bool) -> bool {
        self.items
            .iter_mut()
            .find(|item| item.key == key)
            .is_some_and(|item| Self::finish_item(item, ok))
    }

    pub fn finish_id(&mut self, id: Uuid, ok: bool) -> bool {
        self.items
            .iter_mut()
            .find(|item| item.id == Some(id))
            .is_some_and(|item| Self::finish_item(item, ok))
    }

    fn finish_item(item: &mut IntakeItem, ok: bool) -> bool {
        if item.work.is_terminal() {
            return false;
        }
        item.work = if ok {
            IntakeWork::Succeeded
        } else {
            IntakeWork::Failed
        };
        true
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
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_paths_keeps_supported_images_and_counts_skips() {
        let classified = classify_image_paths([
            PathBuf::from("board.PNG"),
            PathBuf::from("eq-navier.jpg"),
            PathBuf::from("table-3.webp"),
            PathBuf::from("notes.pdf"),
            PathBuf::from("slides.pptx"),
            PathBuf::from("noext"),
        ]);
        assert_eq!(classified.images.len(), 3);
        assert_eq!(classified.skipped, 3);
        assert!(!is_ingest_image_path(Path::new("a.gif")));
        assert!(is_ingest_image_path(Path::new("a.jpeg")));
    }

    #[test]
    fn batch_is_the_decode_queue_and_keeps_generation_on_append() {
        let mut batch = IntakeBatch::from_paths(vec![PathBuf::from("a.png")], 0, 7);
        assert_eq!(batch.next_pending(), Some((1, PathBuf::from("a.png"))));
        batch.bind_item(1, Uuid::nil());
        assert_eq!(batch.next_pending(), None);

        batch.extend(vec![PathBuf::from("b.png")], 2);
        assert_eq!(batch.generation, 7);
        assert_eq!(batch.next_pending(), Some((2, PathBuf::from("b.png"))));
        assert_eq!(batch.counts().skipped, 2);
    }

    #[test]
    fn batch_finishes_each_item_once() {
        let mut batch = IntakeBatch::from_paths(vec![PathBuf::from("a.png")], 0, 3);
        batch.bind_item(1, Uuid::nil());
        batch.mark_working(Uuid::nil());
        assert!(batch.finish_id(Uuid::nil(), true));
        assert!(!batch.finish_id(Uuid::nil(), false));
        assert!(batch.all_terminal());
        assert_eq!(batch.items[0].work, IntakeWork::Succeeded);
    }

    #[test]
    fn completed_batch_stops_accepting_new_files() {
        let mut batch = IntakeBatch::from_paths(vec![PathBuf::from("a.png")], 0, 3);
        assert!(batch.is_accepting());
        assert!(batch.finish_key(1, true));
        assert!(!batch.is_accepting());
    }
}
