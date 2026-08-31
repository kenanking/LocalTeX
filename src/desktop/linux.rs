use std::cell::RefCell;
use std::sync::mpsc::Sender;

use ksni::blocking::TrayMethods;

use crate::desktop::DesktopCmd;
use crate::icon;
use crate::identity::{APP_NAME, APP_SLUG};

pub fn start_tray(tx: Sender<DesktopCmd>) {
    let tray = LocalTexTray { tx };
    if let Err(err) = tray.spawn() {
        eprintln!("{APP_SLUG}: tray spawn: {err}");
    }
}

struct LocalTexTray {
    tx: Sender<DesktopCmd>,
}

impl ksni::Tray for LocalTexTray {
    fn id(&self) -> String {
        APP_SLUG.into()
    }

    fn title(&self) -> String {
        APP_NAME.into()
    }

    fn icon_name(&self) -> String {
        crate::identity::APP_ID.into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 48]
            .into_iter()
            .map(|size| {
                let (width, height, data) = icon::argb_bytes(size);
                ksni::Icon {
                    width,
                    height,
                    data,
                }
            })
            .collect()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        send(&self.tx, DesktopCmd::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Capture".into(),
                activate: Box::new(|this: &mut LocalTexTray| {
                    send(&this.tx, DesktopCmd::Capture);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Show".into(),
                activate: Box::new(|this: &mut LocalTexTray| {
                    send(&this.tx, DesktopCmd::Show);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut LocalTexTray| {
                    send(&this.tx, DesktopCmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn send(tx: &Sender<DesktopCmd>, cmd: DesktopCmd) {
    if let Err(err) = tx.send(cmd) {
        eprintln!("{APP_SLUG}: tray send: {err}");
    }
}

/// GPUI 0.2's X11 `write_to_clipboard` only calls `set_text`. Keep a native
/// `x11-clipboard` connection alive; dropping it kills the setter thread.
pub fn write_clipboard_png(png: Vec<u8>) -> anyhow::Result<()> {
    thread_local! {
        static CLIP: RefCell<Option<(x11_clipboard::Clipboard, x11_clipboard::Atom)>> =
            const { RefCell::new(None) };
    }
    CLIP.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let clip = x11_clipboard::Clipboard::new()?;
            let atom = clip.setter.get_atom("image/png")?;
            *slot = Some((clip, atom));
        }
        let (clip, atom) = slot.as_ref().expect("clipboard");
        clip.store(clip.setter.atoms.clipboard, *atom, png)?;
        Ok(())
    })
}

/// GNOME ignores GPUI's ICCCM `WM_CHANGE_STATE` iconify. EWMH HIDDEN on the
/// root (not ConfigureWindow / `_NET_ACTIVE_WINDOW` on GPUI's XID).
pub fn iconify_main_window() {
    if let Err(err) = set_hidden(true) {
        eprintln!("{APP_SLUG}: iconify: {err}");
    }
}

pub fn deiconify_main_window() {
    if let Err(err) = set_hidden(false) {
        eprintln!("{APP_SLUG}: deiconify: {err}");
    }
}

/// Block until the main window is no longer viewable, then a short compositor
/// settle so xcap does not catch the minimize animation.
pub fn wait_until_iconified() {
    use std::time::{Duration, Instant};
    use x11rb::connection::Connection;
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;
    let deadline = Instant::now() + Duration::from_millis(700);
    loop {
        if main_window_hidden_on(&conn, root) {
            std::thread::sleep(Duration::from_millis(120));
            return;
        }
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn intern(conn: &impl x11rb::connection::Connection, name: &str) -> anyhow::Result<u32> {
    use x11rb::protocol::xproto::ConnectionExt;
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn our_window(conn: &impl x11rb::connection::Connection, root: u32) -> anyhow::Result<Option<u32>> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let net_client_list = intern(conn, "_NET_CLIENT_LIST")?;
    let net_wm_pid = intern(conn, "_NET_WM_PID")?;
    let pid = std::process::id();
    let reply = conn
        .get_property(false, root, net_client_list, AtomEnum::WINDOW, 0, 8192)?
        .reply()?;
    let Some(values) = reply.value32() else {
        return Ok(None);
    };
    for win in values {
        let match_pid = conn
            .get_property(false, win, net_wm_pid, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32()?.next())
            == Some(pid);
        if match_pid {
            return Ok(Some(win));
        }
    }
    Ok(None)
}

fn set_hidden(hide: bool) -> anyhow::Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let Some(win) = our_window(&conn, root)? else {
        return Ok(());
    };

    let net_wm_state = intern(&conn, "_NET_WM_STATE")?;
    let hidden = intern(&conn, "_NET_WM_STATE_HIDDEN")?;
    let wm_change_state = intern(&conn, "WM_CHANGE_STATE")?;
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let action = if hide { 1 } else { 0 };
    // source 1 = application. Do not use pager (2).
    let ev = ClientMessageEvent::new(32, win, net_wm_state, [action, hidden, 0, 1, 0]);
    conn.send_event(false, root, mask, ev)?;

    // ICCCM fallback (what GPUI minimize sends). 3 = Iconic, 1 = Normal.
    let iconic = if hide { 3 } else { 1 };
    let ev = ClientMessageEvent::new(32, win, wm_change_state, [iconic, 0, 0, 0, 0]);
    conn.send_event(false, root, mask, ev)?;
    conn.flush()?;
    Ok(())
}

fn main_window_hidden_on(conn: &impl x11rb::connection::Connection, root: u32) -> bool {
    (|| -> anyhow::Result<bool> {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, MapState};
        let Some(win) = our_window(conn, root)? else {
            return Ok(false);
        };
        let attrs = conn.get_window_attributes(win)?.reply()?;
        if attrs.map_state != MapState::VIEWABLE {
            return Ok(true);
        }
        let hidden = intern(conn, "_NET_WM_STATE_HIDDEN")?;
        let state = intern(conn, "_NET_WM_STATE")?;
        let reply = conn
            .get_property(false, win, state, AtomEnum::ATOM, 0, 64)?
            .reply()?;
        Ok(reply
            .value32()
            .is_some_and(|mut vals| vals.any(|atom| atom == hidden)))
    })()
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn iconify_does_not_poke_gpui_xid() {
        let src = include_str!("linux.rs");
        let hide = src
            .split("fn set_hidden")
            .nth(1)
            .expect("set_hidden")
            .split("fn main_window_hidden")
            .next()
            .expect("body");
        assert!(
            !hide.contains("NET_ACTIVE_WINDOW"),
            "iconify must not send _NET_ACTIVE_WINDOW"
        );
        assert!(
            !hide.contains("configure_window"),
            "iconify must not ConfigureWindow GPUI's XID"
        );
        assert!(hide.contains("NET_WM_STATE_HIDDEN"));
    }
}
