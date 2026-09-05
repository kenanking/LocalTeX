use std::io;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Security::{
    GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId,
    WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, ResetEvent, SetEvent, WaitForSingleObject,
};
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

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
    handle: isize,
    event: isize,
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
            unsafe {
                let _ = SetEvent(HANDLE(server.event as *mut core::ffi::c_void));
                let _ = CloseHandle(HANDLE(server.handle as *mut core::ffi::c_void));
            }
            if let Some(accept) = server.accept.take() {
                let _ = accept.join();
            }
            unsafe {
                let _ = CloseHandle(HANDLE(server.event as *mut core::ffi::c_void));
            }
        }
    }
}

pub(super) fn try_bind(endpoint: &Endpoint) -> io::Result<Inner> {
    let name = pipe_name(endpoint)?;
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            0,
            0,
            0,
            None,
        )
    };
    if handle.is_invalid() {
        let code = unsafe { GetLastError() };
        return Err(io::Error::from_raw_os_error(code.0 as i32));
    }
    let event = match unsafe { CreateEventW(None, true, false, PCWSTR::null()) } {
        Ok(event) => event,
        Err(err) => {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(io::Error::from_raw_os_error(win32_code(&err) as i32));
        }
    };
    let (pending_tx, pending_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let accept_stop = stop.clone();
    let accept_raw = handle.0 as isize;
    let event_raw = event.0 as isize;
    let accept = thread::spawn(move || {
        let accept_handle = HANDLE(accept_raw as *mut core::ffi::c_void);
        let event = HANDLE(event_raw as *mut core::ffi::c_void);
        loop {
            if accept_stop.load(Ordering::SeqCst) {
                break;
            }
            let _ = unsafe { ResetEvent(event) };
            let mut overlapped = OVERLAPPED {
                hEvent: event,
                ..Default::default()
            };
            // Overlapped listen: same-process blocking ConnectNamedPipe + CreateFile can hang.
            let connected = match unsafe {
                ConnectNamedPipe(accept_handle, Some(&mut overlapped as *mut _))
            } {
                Ok(()) => true,
                Err(err) if win32_code(&err) == ERROR_PIPE_CONNECTED.0 => true,
                Err(err) if win32_code(&err) == ERROR_IO_PENDING.0 => loop {
                    if accept_stop.load(Ordering::SeqCst) {
                        return;
                    }
                    if unsafe { WaitForSingleObject(event, 50) }.0 == 0 {
                        let mut transferred = 0;
                        break unsafe {
                            GetOverlappedResult(accept_handle, &overlapped, &mut transferred, false)
                        }
                        .is_ok();
                    }
                },
                Err(_) => false,
            };
            if !connected {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            if accept_stop.load(Ordering::SeqCst) {
                break;
            }
            if pending_tx.send(Reveal).is_err() {
                break;
            }
            let _ = unsafe { DisconnectNamedPipe(accept_handle) };
        }
    });
    Ok(Inner {
        server: Some(Server {
            stop,
            handle: handle.0 as isize,
            event: event.0 as isize,
            accept: Some(accept),
        }),
        pending: Some(pending_rx),
        serving: AtomicBool::new(false),
    })
}

pub(super) fn connect(endpoint: &Endpoint) -> io::Result<()> {
    let name = pipe_name(endpoint)?;
    let client = open_pipe_retry(&name)?;
    let mut pid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(client, &mut pid) }.is_ok() && pid != 0 {
        // The activating click belongs to this loser; restore_main needs the grant.
        let _ = unsafe { AllowSetForegroundWindow(pid) };
    }
    // This connection is the reveal signal, so keep it alive until the accept
    // thread has observed it. There is no request/ack payload to wait on.
    thread::sleep(Duration::from_millis(50));
    unsafe {
        let _ = CloseHandle(client);
    }
    Ok(())
}

pub(super) fn is_occupied(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(code)
            if code == ERROR_ACCESS_DENIED.0 as i32 || code == ERROR_PIPE_BUSY.0 as i32
    )
}

fn open_pipe_retry(name: &[u16]) -> io::Result<HANDLE> {
    for _ in 0..80 {
        let _ = unsafe { WaitNamedPipeW(PCWSTR(name.as_ptr()), 50) };
        match open_pipe(name) {
            Ok(handle) => return Ok(handle),
            Err(err)
                if matches!(
                    err.raw_os_error(),
                    Some(code)
                        if code == ERROR_PIPE_BUSY.0 as i32
                            || code == ERROR_FILE_NOT_FOUND.0 as i32
                ) =>
            {
                thread::sleep(Duration::from_millis(25));
            }
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "connect to primary timed out",
    ))
}

fn open_pipe(name: &[u16]) -> io::Result<HANDLE> {
    unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|err| io::Error::from_raw_os_error(win32_code(&err) as i32))
}

fn pipe_name(endpoint: &Endpoint) -> io::Result<Vec<u16>> {
    let sid = user_sid()?;
    let name = match &endpoint.kind {
        EndpointKind::Product => format!(r"\\.\pipe\{APP_ID}.{sid}"),
        EndpointKind::Isolated(suffix) => format!(r"\\.\pipe\{APP_ID}.{sid}.{suffix}"),
    };
    Ok(std::ffi::OsStr::new(&name)
        .encode_wide()
        .chain(Some(0))
        .collect())
}

fn user_sid() -> io::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|err| io::Error::from_raw_os_error(win32_code(&err) as i32))?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        let mut buf = vec![0u8; needed as usize];
        let info = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        info.map_err(|err| io::Error::from_raw_os_error(win32_code(&err) as i32))?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        sid_to_string(user.User.Sid)
    }
}

fn sid_to_string(sid: windows::Win32::Security::PSID) -> io::Result<String> {
    let len = unsafe { GetLengthSid(sid) } as usize;
    if len < 8 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "sid too short"));
    }
    let bytes = unsafe { std::slice::from_raw_parts(sid.0 as *const u8, len) };
    let revision = bytes[0];
    let sub_auth_count = bytes[1] as usize;
    let mut authority: u64 = 0;
    for &b in &bytes[2..8] {
        authority = (authority << 8) | u64::from(b);
    }
    let needed = 8 + sub_auth_count * 4;
    if bytes.len() < needed {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "sid truncated"));
    }
    let mut out = format!("S-{revision}-{authority}");
    for i in 0..sub_auth_count {
        let off = 8 + i * 4;
        let sub = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        out.push('-');
        out.push_str(&sub.to_string());
    }
    Ok(out)
}

fn win32_code(err: &windows::core::Error) -> u32 {
    let h = err.code().0 as u32;
    if h & 0xFFFF_0000 == 0x8007_0000 {
        h & 0xFFFF
    } else {
        h
    }
}
