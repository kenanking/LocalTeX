use std::io;
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::desktop::DesktopCmd;
use crate::identity::APP_SLUG;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod other;
#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "linux")]
use linux as sys;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
use other as sys;
#[cfg(target_os = "windows")]
use win as sys;

const CLAIM_ATTEMPTS: u32 = 3;
const CLAIM_RETRY: Duration = Duration::from_millis(80);

/// GPUI calls the launch callback once and drops it before the platform loop.
/// Keep the exclusive listener for the process lifetime.
static HELD: Mutex<Option<Seat>> = Mutex::new(None);

pub enum Claim {
    Primary(Seat),
    AlreadyRunning,
}

pub struct Seat {
    inner: sys::Inner,
}

pub(super) struct Reveal;

pub(super) struct Endpoint {
    kind: EndpointKind,
}

pub(super) enum EndpointKind {
    Product,
    #[allow(dead_code)]
    Isolated(String),
}

impl Seat {
    pub fn serve(&mut self, tx: Sender<DesktopCmd>) {
        self.inner.serve(tx);
    }

    pub fn hold(self) {
        *HELD.lock().unwrap_or_else(|err| err.into_inner()) = Some(self);
    }
}

pub fn claim() -> Claim {
    match claim_at(&Endpoint {
        kind: EndpointKind::Product,
    }) {
        Ok(claim) => claim,
        Err(err) => {
            eprintln!("{APP_SLUG}: instance: {err}");
            std::process::exit(1);
        }
    }
}

fn claim_at(endpoint: &Endpoint) -> io::Result<Claim> {
    let mut last_err: Option<io::Error> = None;
    for attempt in 0..CLAIM_ATTEMPTS {
        match sys::try_bind(endpoint) {
            Ok(inner) => return Ok(Claim::Primary(Seat { inner })),
            Err(err) if sys::is_occupied(&err) => match sys::connect(endpoint) {
                Ok(()) => return Ok(Claim::AlreadyRunning),
                Err(connect_err) => last_err = Some(connect_err),
            },
            Err(err) => return Err(err),
        }
        if attempt + 1 < CLAIM_ATTEMPTS {
            thread::sleep(CLAIM_RETRY);
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("could not claim instance seat")))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::{Mutex, MutexGuard};
    use std::time::Duration;

    static NEXT: AtomicU64 = AtomicU64::new(0);
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock_tests() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn isolated() -> Endpoint {
        Endpoint {
            kind: EndpointKind::Isolated(format!(
                "t{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
        }
    }

    fn recv_reveal(rx: &mpsc::Receiver<DesktopCmd>) -> DesktopCmd {
        rx.recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for Reveal")
    }

    #[test]
    fn drop_seat_then_claim_is_primary_again() {
        let _lock = lock_tests();
        let endpoint = isolated();
        let first = claim_at(&endpoint).unwrap();
        assert!(matches!(first, Claim::Primary(_)));
        drop(first);
        let second = claim_at(&endpoint).unwrap();
        assert!(matches!(second, Claim::Primary(_)));
    }

    #[test]
    fn second_claim_while_alive_is_already_running_and_reveals() {
        let _lock = lock_tests();
        let endpoint = isolated();
        let mut seat = match claim_at(&endpoint).unwrap() {
            Claim::Primary(seat) => seat,
            Claim::AlreadyRunning => panic!("expected primary"),
        };
        let claimed = claim_at(&endpoint).unwrap();
        assert!(matches!(claimed, Claim::AlreadyRunning));
        let (tx, rx) = mpsc::channel();
        seat.serve(tx);
        assert_eq!(recv_reveal(&rx), DesktopCmd::Reveal);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn serve_drains_buffered_reveals_not_show() {
        let _lock = lock_tests();
        let endpoint = isolated();
        let mut seat = match claim_at(&endpoint).unwrap() {
            Claim::Primary(seat) => seat,
            Claim::AlreadyRunning => panic!("expected primary"),
        };
        assert!(matches!(
            claim_at(&endpoint).unwrap(),
            Claim::AlreadyRunning
        ));
        assert!(matches!(
            claim_at(&endpoint).unwrap(),
            Claim::AlreadyRunning
        ));
        let (tx, rx) = mpsc::channel();
        seat.serve(tx.clone());
        seat.serve(tx);
        assert_eq!(recv_reveal(&rx), DesktopCmd::Reveal);
        assert_eq!(recv_reveal(&rx), DesktopCmd::Reveal);
        assert!(rx.try_recv().is_err());
    }
}
