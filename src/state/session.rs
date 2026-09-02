use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

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

pub(crate) struct IngestPump {
    pub ocr: OcrQueue,
    pub file_queue: VecDeque<PathBuf>,
    pub file_loading: bool,
    pub clipboard_loading: bool,
    pub thumb_inflight: HashSet<Uuid>,
    pub thumb_failed: HashSet<Uuid>,
    pub blocks_inflight: HashSet<Uuid>,
    pub png_inflight: HashSet<Uuid>,
    pub persist_retry_counts: HashMap<Uuid, u8>,
    pub persist_retry_pending: HashSet<Uuid>,
}

impl IngestPump {
    pub fn new() -> Self {
        Self {
            ocr: OcrQueue::new(),
            file_queue: VecDeque::new(),
            file_loading: false,
            clipboard_loading: false,
            thumb_inflight: HashSet::new(),
            thumb_failed: HashSet::new(),
            blocks_inflight: HashSet::new(),
            png_inflight: HashSet::new(),
            persist_retry_counts: HashMap::new(),
            persist_retry_pending: HashSet::new(),
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
}
