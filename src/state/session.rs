use std::collections::HashMap;
use std::time::Duration;

use uuid::Uuid;

use crate::ocr_queue::OcrQueue;

pub(crate) enum Capture {
    Idle,
    Grabbing,
    Failed(String),
}

pub(crate) struct CaptureSession {
    status: Capture,
    gen: u64,
    hidden: bool,
}

impl CaptureSession {
    pub fn new() -> Self {
        Self {
            status: Capture::Idle,
            gen: 0,
            hidden: false,
        }
    }

    pub fn is_grabbing(&self) -> bool {
        matches!(self.status, Capture::Grabbing)
    }

    pub fn error(&self) -> Option<&str> {
        match &self.status {
            Capture::Failed(err) => Some(err.as_str()),
            _ => None,
        }
    }

    pub fn set(&mut self, next: Capture) {
        self.status = next;
        self.gen = self.gen.wrapping_add(1);
    }

    pub fn gen(&self) -> u64 {
        self.gen
    }

    pub fn should_clear_flash(&self, gen: u64) -> bool {
        self.gen == gen && matches!(self.status, Capture::Failed(_))
    }

    pub fn push_hide(&mut self) {
        self.hidden = true;
    }

    pub fn pop_hide(&mut self) -> bool {
        let was = self.hidden;
        self.hidden = false;
        was
    }

    pub fn force_show(&mut self) {
        self.hidden = false;
    }
}

#[derive(Default)]
struct DocRuntime {
    thumb_inflight: bool,
    thumb_failed: bool,
    blocks_inflight: bool,
    png_inflight: bool,
    persist_retry_count: u8,
    persist_retry_pending: bool,
}

pub(crate) struct IngestPump {
    pub ocr: OcrQueue,
    pub file_loading: bool,
    pub clipboard_loading: bool,
    runtime: HashMap<Uuid, DocRuntime>,
}

impl IngestPump {
    pub fn new() -> Self {
        Self {
            ocr: OcrQueue::new(),
            file_loading: false,
            clipboard_loading: false,
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
            .is_some_and(|r| r.thumb_inflight || r.thumb_failed)
    }

    pub fn start_thumb(&mut self, id: Uuid) {
        self.runtime.entry(id).or_default().thumb_inflight = true;
    }

    pub fn finish_thumb(&mut self, id: Uuid) {
        if let Some(r) = self.runtime.get_mut(&id) {
            r.thumb_inflight = false;
        }
    }

    pub fn fail_thumb(&mut self, id: Uuid) {
        let r = self.runtime.entry(id).or_default();
        r.thumb_inflight = false;
        r.thumb_failed = true;
    }

    pub fn thumb_failed(&self, id: Uuid) -> bool {
        self.runtime.get(&id).is_some_and(|r| r.thumb_failed)
    }

    pub fn start_blocks(&mut self, id: Uuid) -> bool {
        let r = self.runtime.entry(id).or_default();
        if r.blocks_inflight {
            false
        } else {
            r.blocks_inflight = true;
            true
        }
    }

    pub fn finish_blocks(&mut self, id: Uuid) {
        if let Some(r) = self.runtime.get_mut(&id) {
            r.blocks_inflight = false;
        }
    }

    pub fn start_png(&mut self, id: Uuid) -> bool {
        let r = self.runtime.entry(id).or_default();
        if r.png_inflight {
            false
        } else {
            r.png_inflight = true;
            true
        }
    }

    pub fn finish_png(&mut self, id: Uuid) {
        if let Some(r) = self.runtime.get_mut(&id) {
            r.png_inflight = false;
        }
    }

    pub fn persist_retry_pending(&self, id: Uuid) -> bool {
        self.runtime
            .get(&id)
            .is_some_and(|r| r.persist_retry_pending)
    }

    pub fn begin_persist_retry(&mut self, id: Uuid) -> Option<Duration> {
        if self.persist_retry_pending(id) {
            return None;
        }
        let r = self.runtime.entry(id).or_default();
        if r.persist_retry_count >= 3 {
            return None;
        }
        r.persist_retry_count += 1;
        r.persist_retry_pending = true;
        Some(Duration::from_secs(1 << (r.persist_retry_count - 1)))
    }

    pub fn finish_persist_retry(&mut self, id: Uuid) -> bool {
        self.runtime
            .get_mut(&id)
            .map(|r| std::mem::replace(&mut r.persist_retry_pending, false))
            .unwrap_or(false)
    }

    pub fn clear_persist_retry(&mut self, id: Uuid) {
        if let Some(r) = self.runtime.get_mut(&id) {
            r.persist_retry_count = 0;
            r.persist_retry_pending = false;
        }
    }
}

pub(crate) struct SearchFilter {
    pub query: String,
    pub gen: u64,
    pub task: Option<gpui::Task<()>>,
}

impl SearchFilter {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            gen: 0,
            task: None,
        }
    }

    pub fn bump(&mut self) -> u64 {
        self.gen = self.gen.wrapping_add(1);
        self.gen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_flash_gen_does_not_clear() {
        let mut c = CaptureSession::new();
        c.set(Capture::Failed("x".into()));
        let old = c.gen();
        c.set(Capture::Idle);
        c.set(Capture::Failed("y".into()));
        assert!(!c.should_clear_flash(old));
        assert!(c.should_clear_flash(c.gen()));
    }

    #[test]
    fn hide_restores_when_hidden() {
        let mut c = CaptureSession::new();
        assert!(!c.pop_hide());
        c.push_hide();
        c.push_hide();
        assert!(c.pop_hide());
        assert!(!c.pop_hide());
    }

    #[test]
    fn drop_doc_clears_all_runtime_flags() {
        let mut pump = IngestPump::new();
        let id = Uuid::nil();
        assert!(pump.start_blocks(id));
        pump.start_thumb(id);
        pump.fail_thumb(id);
        assert!(pump.begin_persist_retry(id).is_some());
        pump.drop_doc(id);
        assert!(!pump.thumb_blocked(id));
        assert!(pump.start_blocks(id));
    }
}
