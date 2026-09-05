//! Serial OCR jobs. The ONNX `Engine` mutex already serializes inference;
//! this queue adds backpressure and a single running slot for batch ingest.

use std::collections::VecDeque;

use uuid::Uuid;

#[derive(Default)]
pub struct OcrQueue {
    pending: VecDeque<Uuid>,
    running: Option<Uuid>,
}

impl OcrQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn running(&self) -> Option<Uuid> {
        self.running
    }

    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.running().is_none() && self.pending.is_empty()
    }

    pub fn enqueue(&mut self, id: Uuid) {
        if self.running() == Some(id) || self.pending.iter().any(|x| *x == id) {
            return;
        }
        self.pending.push_back(id);
    }

    pub fn take_next(&mut self) -> Option<Uuid> {
        if self.running().is_some() {
            return None;
        }
        let id = self.pending.pop_front()?;
        self.running = Some(id);
        Some(id)
    }

    pub fn finish(&mut self, id: Uuid) {
        if self.running == Some(id) {
            self.running = None;
        }
        self.pending.retain(|x| *x != id);
    }

    pub fn cancel_remaining(&mut self) {
        self.pending.clear();
    }

    pub fn remove(&mut self, id: Uuid) {
        self.pending.retain(|x| *x != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_running_job_keeps_slot_until_completion() {
        let mut q = OcrQueue::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        q.enqueue(a);
        q.enqueue(b);
        assert_eq!(q.take_next(), Some(a));
        q.remove(a);
        assert_eq!(q.take_next(), None);
        q.finish(a);
        assert_eq!(q.take_next(), Some(b));
    }

    #[test]
    fn serializes_jobs() {
        let mut q = OcrQueue::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        q.enqueue(a);
        q.enqueue(b);
        assert_eq!(q.take_next(), Some(a));
        assert_eq!(q.running(), Some(a));
        assert!(q.take_next().is_none());
        q.finish(a);
        assert_eq!(q.take_next(), Some(b));
        q.cancel_remaining();
        q.finish(b);
        assert!(q.is_idle());
    }
}
