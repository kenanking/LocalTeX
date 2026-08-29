use std::collections::{HashSet, VecDeque};
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
    hide_depth: u32,
    reveal_on_main: bool,
}

impl CaptureSession {
    pub fn new() -> Self {
        Self {
            status: Capture::Idle,
            gen: 0,
            hide_depth: 0,
            reveal_on_main: false,
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
        self.hide_depth = self.hide_depth.saturating_add(1);
    }

    pub fn pop_hide(&mut self) -> bool {
        if self.hide_depth == 0 {
            return false;
        }
        self.hide_depth -= 1;
        self.hide_depth == 0
    }

    pub fn force_show(&mut self) {
        self.hide_depth = 0;
    }

    pub fn set_reveal_on_main(&mut self, v: bool) {
        self.reveal_on_main = v;
    }

    pub fn take_reveal_on_main(&mut self) -> bool {
        let v = self.reveal_on_main;
        self.reveal_on_main = false;
        v
    }
}

pub(crate) struct IngestPump {
    pub ocr: OcrQueue,
    pub file_queue: VecDeque<PathBuf>,
    pub file_loading: bool,
    pub thumb_inflight: HashSet<Uuid>,
}

impl IngestPump {
    pub fn new() -> Self {
        Self {
            ocr: OcrQueue::new(),
            file_queue: VecDeque::new(),
            file_loading: false,
            thumb_inflight: HashSet::new(),
        }
    }
}

pub(crate) struct SearchFilter {
    pub query: String,
    pub gen: u64,
}

impl SearchFilter {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            gen: 0,
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
    fn hide_depth_restores_only_at_zero() {
        let mut c = CaptureSession::new();
        c.push_hide();
        c.push_hide();
        assert!(!c.pop_hide());
        assert!(c.pop_hide());
    }
}
