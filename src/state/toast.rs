pub(crate) struct ToastState {
    message: Option<String>,
    gen: u64,
}

impl ToastState {
    pub fn new() -> Self {
        Self {
            message: None,
            gen: 0,
        }
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn show(&mut self, message: String) -> u64 {
        self.message = Some(message);
        self.gen = self.gen.wrapping_add(1);
        self.gen
    }

    pub fn clear(&mut self, gen: u64) -> bool {
        if self.gen != gen || self.message.is_none() {
            return false;
        }
        self.message = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_generation_does_not_clear_new_message() {
        let mut toast = ToastState::new();
        let old = toast.show("x".into());
        let current = toast.show("y".into());
        assert!(!toast.clear(old));
        assert!(toast.clear(current));
    }
}
