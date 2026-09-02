use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use crate::desktop::DesktopCmd;

use super::Endpoint;

pub(super) struct Inner {
    serving: AtomicBool,
}

impl Inner {
    pub(super) fn serve(&mut self, _tx: Sender<DesktopCmd>) {
        self.serving.store(true, Ordering::SeqCst);
    }
}

pub(super) fn try_bind(_endpoint: &Endpoint) -> io::Result<Inner> {
    Ok(Inner {
        serving: AtomicBool::new(false),
    })
}

pub(super) fn connect(_endpoint: &Endpoint) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::ConnectionRefused))
}

pub(super) fn is_occupied(_err: &io::Error) -> bool {
    false
}
