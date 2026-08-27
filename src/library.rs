//! Snip index: HashMap documents + recency order + visible filter ids.

use std::collections::HashMap;
use std::sync::Arc;

use image::RgbaImage;
use uuid::Uuid;

use crate::cache::{pixel_bytes, PIXEL_BUDGET, PIXEL_MAX_ENTRIES};
use crate::doc::{Document, ImageSlot};
use crate::store::{CivilDate, DateRange, SnipListItem};

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

pub const DATE_PRESETS: [(&str, &str, DatePreset); 4] = [
    ("date-all", "All", DatePreset::All),
    ("date-today", "Today", DatePreset::Today),
    ("date-7d", "7d", DatePreset::Last7Days),
    ("date-30d", "30d", DatePreset::Last30Days),
];

pub struct Library {
    pub docs: HashMap<Uuid, Document>,
    pub order: Vec<Uuid>,
    pub visible_ids: Vec<Uuid>,
    pub selected: Option<Uuid>,
    pub date_preset: DatePreset,
    loaded_lru: Vec<Uuid>,
}

impl Library {
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            order: Vec::new(),
            visible_ids: Vec::new(),
            selected: None,
            date_preset: DatePreset::All,
            loaded_lru: Vec::new(),
        }
    }

    pub fn from_list(items: Vec<SnipListItem>) -> Self {
        let mut lib = Self::new();
        for item in items {
            let doc = Document::from_list_item(item);
            lib.order.push(doc.id);
            lib.docs.insert(doc.id, doc);
        }
        lib.selected = lib.order.first().copied();
        lib.visible_ids = lib.order.clone();
        lib
    }

    pub fn get(&self, id: Uuid) -> Option<&Document> {
        self.docs.get(&id)
    }

    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut Document> {
        self.docs.get_mut(&id)
    }

    pub fn selected_doc(&self) -> Option<&Document> {
        self.selected.and_then(|id| self.docs.get(&id))
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn insert_newest(&mut self, doc: Document) {
        let id = doc.id;
        self.docs.insert(id, doc);
        self.order.insert(0, id);
        self.selected = Some(id);
        if !self.visible_ids.contains(&id) {
            self.visible_ids.insert(0, id);
        }
        self.touch_lru(id);
    }

    pub fn remove(&mut self, id: Uuid) {
        self.docs.remove(&id);
        self.order.retain(|x| *x != id);
        self.visible_ids.retain(|x| *x != id);
        self.loaded_lru.retain(|x| *x != id);
        self.selected = self
            .visible_ids
            .first()
            .copied()
            .or_else(|| self.order.first().copied());
    }

    pub fn visible_docs(&self) -> impl Iterator<Item = &Document> {
        self.visible_ids.iter().filter_map(|id| self.docs.get(id))
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &Document> {
        self.order.iter().filter_map(|id| self.docs.get(id))
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

    pub fn touch_lru(&mut self, id: Uuid) {
        self.loaded_lru.retain(|x| *x != id);
        self.loaded_lru.push(id);
        loop {
            let loaded: Vec<Uuid> = self
                .loaded_lru
                .iter()
                .copied()
                .filter(|x| {
                    self.docs
                        .get(x)
                        .is_some_and(|d| matches!(d.image, ImageSlot::Loaded(_)))
                })
                .collect();
            let bytes: u64 = loaded
                .iter()
                .filter_map(|x| {
                    self.docs
                        .get(x)
                        .and_then(|d| d.image.pixels().map(|p| pixel_bytes(p)))
                })
                .sum();
            let over_n = loaded.len() > PIXEL_MAX_ENTRIES;
            let over_b = bytes > PIXEL_BUDGET;
            if !over_n && !over_b {
                break;
            }
            let Some(pos) = self
                .loaded_lru
                .iter()
                .position(|x| Some(*x) != self.selected)
            else {
                break;
            };
            let evict = self.loaded_lru.remove(pos);
            if let Some(doc) = self.docs.get_mut(&evict) {
                if doc.persisted && matches!(doc.image, ImageSlot::Loaded(_)) {
                    doc.image = ImageSlot::OnDisk;
                }
            }
        }
    }

    pub fn pixels(&self, id: Uuid) -> Option<Arc<RgbaImage>> {
        self.docs.get(&id).and_then(|d| d.image.pixels().cloned())
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use image::RgbaImage;

    #[test]
    fn hashmap_survives_delete_without_index_shift() {
        let mut lib = Library::new();
        let a = Document::pending(Arc::new(RgbaImage::new(4, 4)));
        let b = Document::pending(Arc::new(RgbaImage::new(4, 4)));
        let id_a = a.id;
        let id_b = b.id;
        lib.insert_newest(a);
        lib.insert_newest(b);
        assert_eq!(lib.order[0], id_b);
        lib.remove(id_b);
        assert_eq!(lib.get(id_a).map(|d| d.id), Some(id_a));
        assert_eq!(lib.order, vec![id_a]);
        assert!(lib.pixels(id_a).is_some());
        assert_eq!(lib.visible_ids.len(), 1);
    }
}
