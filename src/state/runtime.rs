use std::collections::HashMap;
use std::time::Duration;

use uuid::Uuid;

#[derive(Default)]
struct DocRuntime {
    thumb_inflight: bool,
    thumb_failed: bool,
    blocks_inflight: bool,
    png_inflight: bool,
    persist_retry_count: u8,
    persist_retry_pending: bool,
}

pub(crate) struct DocumentRuntime {
    runtime: HashMap<Uuid, DocRuntime>,
}

impl DocumentRuntime {
    pub fn new() -> Self {
        Self {
            runtime: HashMap::new(),
        }
    }

    pub fn drop_doc(&mut self, id: Uuid) {
        self.runtime.remove(&id);
    }

    pub fn clear_docs(&mut self) {
        self.runtime.clear();
    }

    pub fn thumb_blocked(&self, id: Uuid) -> bool {
        self.runtime
            .get(&id)
            .is_some_and(|runtime| runtime.thumb_inflight || runtime.thumb_failed)
    }

    pub fn start_thumb(&mut self, id: Uuid) {
        self.runtime.entry(id).or_default().thumb_inflight = true;
    }

    pub fn finish_thumb(&mut self, id: Uuid) {
        if let Some(runtime) = self.runtime.get_mut(&id) {
            runtime.thumb_inflight = false;
        }
    }

    pub fn fail_thumb(&mut self, id: Uuid) {
        let runtime = self.runtime.entry(id).or_default();
        runtime.thumb_inflight = false;
        runtime.thumb_failed = true;
    }

    pub fn thumb_failed(&self, id: Uuid) -> bool {
        self.runtime
            .get(&id)
            .is_some_and(|runtime| runtime.thumb_failed)
    }

    pub fn start_blocks(&mut self, id: Uuid) -> bool {
        let runtime = self.runtime.entry(id).or_default();
        if runtime.blocks_inflight {
            false
        } else {
            runtime.blocks_inflight = true;
            true
        }
    }

    pub fn finish_blocks(&mut self, id: Uuid) {
        if let Some(runtime) = self.runtime.get_mut(&id) {
            runtime.blocks_inflight = false;
        }
    }

    pub fn start_png(&mut self, id: Uuid) -> bool {
        let runtime = self.runtime.entry(id).or_default();
        if runtime.png_inflight {
            false
        } else {
            runtime.png_inflight = true;
            true
        }
    }

    pub fn finish_png(&mut self, id: Uuid) {
        if let Some(runtime) = self.runtime.get_mut(&id) {
            runtime.png_inflight = false;
        }
    }

    pub fn begin_persist_retry(&mut self, id: Uuid) -> Option<Duration> {
        let runtime = self.runtime.entry(id).or_default();
        if runtime.persist_retry_pending || runtime.persist_retry_count >= 3 {
            return None;
        }
        runtime.persist_retry_count += 1;
        runtime.persist_retry_pending = true;
        Some(Duration::from_secs(1 << (runtime.persist_retry_count - 1)))
    }

    pub fn finish_persist_retry(&mut self, id: Uuid) -> bool {
        self.runtime
            .get_mut(&id)
            .map(|runtime| std::mem::replace(&mut runtime.persist_retry_pending, false))
            .unwrap_or(false)
    }

    pub fn clear_persist_retry(&mut self, id: Uuid) {
        if let Some(runtime) = self.runtime.get_mut(&id) {
            runtime.persist_retry_count = 0;
            runtime.persist_retry_pending = false;
        }
    }
}

pub(crate) struct FileIntakeSession {
    pub loading: bool,
}

impl FileIntakeSession {
    pub fn new() -> Self {
        Self { loading: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_a_document_clears_all_runtime_flags() {
        let mut runtime = DocumentRuntime::new();
        let id = Uuid::nil();
        assert!(runtime.start_blocks(id));
        runtime.start_thumb(id);
        runtime.fail_thumb(id);
        assert!(runtime.begin_persist_retry(id).is_some());
        runtime.drop_doc(id);
        assert!(!runtime.thumb_blocked(id));
        assert!(runtime.start_blocks(id));
    }
}
