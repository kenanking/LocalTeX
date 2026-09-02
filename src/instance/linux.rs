use std::io;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::desktop::DesktopCmd;
use crate::identity::APP_ID;

use super::{Endpoint, EndpointKind, Reveal};

pub(super) struct Inner {
    server: Option<Server>,
    pending: Option<mpsc::Receiver<Reveal>>,
    serving: AtomicBool,
}

struct Server {
    stop: Arc<AtomicBool>,
    addr: SocketAddr,
    _listener: Arc<UnixListener>,
    accept: Option<JoinHandle<()>>,
}

impl Inner {
    pub(super) fn serve(&mut self, tx: Sender<DesktopCmd>) {
        if self.serving.swap(true, Ordering::SeqCst) {
            return;
        }
        let pending = self.pending.take().expect("pending");
        while pending.try_recv().is_ok() {
            if tx.send(DesktopCmd::Reveal).is_err() {
                return;
            }
        }
        thread::spawn(move || {
            for _ in pending {
                if tx.send(DesktopCmd::Reveal).is_err() {
                    break;
                }
            }
        });
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(mut server) = self.server.take() {
            server.stop.store(true, Ordering::SeqCst);
            // Unblock accept() so Drop can join before the next test binds.
            let _ = UnixStream::connect_addr(&server.addr);
            if let Some(accept) = server.accept.take() {
                let _ = accept.join();
            }
        }
    }
}

pub(super) fn try_bind(endpoint: &Endpoint) -> io::Result<Inner> {
    let addr = socket_addr(endpoint)?;
    let listener = Arc::new(UnixListener::bind_addr(&addr)?);
    let (pending_tx, pending_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let accept_listener = listener.clone();
    let accept_stop = stop.clone();
    let accept = thread::spawn(move || {
        for stream in accept_listener.incoming() {
            let Ok(stream) = stream else {
                break;
            };
            drop(stream);
            if accept_stop.load(Ordering::SeqCst) {
                break;
            }
            if pending_tx.send(Reveal).is_err() {
                break;
            }
        }
    });
    Ok(Inner {
        server: Some(Server {
            stop,
            addr,
            _listener: listener,
            accept: Some(accept),
        }),
        pending: Some(pending_rx),
        serving: AtomicBool::new(false),
    })
}

pub(super) fn connect(endpoint: &Endpoint) -> io::Result<()> {
    let addr = socket_addr(endpoint)?;
    drop(UnixStream::connect_addr(&addr)?);
    Ok(())
}

pub(super) fn is_occupied(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::AddrInUse
}

fn socket_addr(endpoint: &Endpoint) -> io::Result<SocketAddr> {
    let uid = unsafe { libc::getuid() };
    let name = match &endpoint.kind {
        EndpointKind::Product => format!("{APP_ID}.{uid}"),
        EndpointKind::Isolated(suffix) => format!("{APP_ID}.{uid}.{suffix}"),
    };
    SocketAddr::from_abstract_name(name.as_bytes())
}
