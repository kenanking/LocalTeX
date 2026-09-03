pub(crate) enum Capture {
    Idle,
    Grabbing,
}

pub(crate) struct CaptureSession {
    status: Capture,
    hidden: bool,
    clipboard_loading: bool,
}

impl CaptureSession {
    pub fn new() -> Self {
        Self {
            status: Capture::Idle,
            hidden: false,
            clipboard_loading: false,
        }
    }

    pub fn is_grabbing(&self) -> bool {
        matches!(self.status, Capture::Grabbing)
    }

    pub fn set(&mut self, next: Capture) {
        self.status = next;
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

    pub fn clipboard_loading(&self) -> bool {
        self.clipboard_loading
    }

    #[cfg(target_os = "windows")]
    pub fn set_clipboard_loading(&mut self, loading: bool) {
        self.clipboard_loading = loading;
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
    fn hide_restores_when_hidden() {
        let mut c = CaptureSession::new();
        assert!(!c.pop_hide());
        c.push_hide();
        c.push_hide();
        assert!(c.pop_hide());
        assert!(!c.pop_hide());
    }
}
