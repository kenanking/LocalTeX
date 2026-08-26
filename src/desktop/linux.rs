use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;

use ksni::blocking::TrayMethods;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

use crate::desktop::DesktopCmd;
use crate::icon;
use crate::identity::{APP_NAME, APP_SLUG};

const SEQ: Ordering = Ordering::SeqCst;

/// Remembered overlay XID so hide/show still works after Unmap (withdrawn
/// windows drop out of `_NET_CLIENT_LIST`).
static OVERLAY_XID: Mutex<Option<u32>> = Mutex::new(None);
static OVERLAY_ARMED: AtomicBool = AtomicBool::new(false);

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

pub fn arm_overlay() {
    OVERLAY_ARMED.store(true, SEQ);
}

/// Unminimize + map + raise + fullscreen-ADD the overlay. GPUI's activate uses
/// EWMH source=1 (application), which mutter treats as focus stealing.
pub fn raise_overlay() {
    if !OVERLAY_ARMED.load(SEQ) {
        return;
    }
    if let Err(err) = raise_overlay_x11() {
        eprintln!("{APP_SLUG}: raise overlay: {err}");
    }
}

/// Unmap the snip overlay instead of destroying it. A second GPUI X11 window
/// in the same process often never receives XI2/focus after the first is dropped.
pub fn park_overlay() {
    OVERLAY_ARMED.store(false, SEQ);
    if let Err(err) = park_overlay_x11() {
        eprintln!("{APP_SLUG}: park overlay: {err}");
    }
}

fn intern(conn: &impl Connection, name: &str) -> anyhow::Result<u32> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn title_matches(conn: &impl Connection, win: u32, wm_name: u32) -> bool {
    let title = crate::identity::OVERLAY_TITLE.as_bytes();
    conn.get_property(false, win, wm_name, AtomEnum::STRING, 0, 256)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.value == title)
}

fn xid_alive(conn: &impl Connection, id: u32) -> bool {
    conn.get_geometry(id)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some()
}

fn overlay_xids_by_title(conn: &impl Connection, root: u32) -> anyhow::Result<Vec<u32>> {
    let wm_name = intern(conn, "WM_NAME")?;
    let net_client_list = intern(conn, "_NET_CLIENT_LIST")?;
    let mut windows: Vec<u32> = Vec::new();
    if let Some(reply) = conn
        .get_property(false, root, net_client_list, AtomEnum::WINDOW, 0, 8192)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        if let Some(values) = reply.value32() {
            windows.extend(values);
        }
    }
    Ok(windows
        .into_iter()
        .filter(|&w| title_matches(conn, w, wm_name))
        .collect())
}

fn remember_xid(id: u32) {
    if let Ok(mut guard) = OVERLAY_XID.lock() {
        *guard = Some(id);
    }
}

/// Prefer the cached XID (unmap withdraws from `_NET_CLIENT_LIST`). Title
/// scan is only the first-map / stale-cache fallback.
fn overlay_xid(conn: &impl Connection, root: u32) -> anyhow::Result<Option<u32>> {
    if let Ok(guard) = OVERLAY_XID.lock() {
        if let Some(id) = *guard {
            if xid_alive(conn, id) {
                return Ok(Some(id));
            }
        }
    }
    let Some(win) = overlay_xids_by_title(conn, root)?.into_iter().max() else {
        return Ok(None);
    };
    remember_xid(win);
    Ok(Some(win))
}

fn raise_overlay_x11() -> anyhow::Result<()> {
    use x11rb::protocol::xproto::{ClientMessageEvent, ConfigureWindowAux, EventMask, StackMode};

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let Some(win) = overlay_xid(&conn, root)? else {
        return Ok(());
    };

    let net_active = intern(&conn, "_NET_ACTIVE_WINDOW")?;
    let wm_change_state = intern(&conn, "WM_CHANGE_STATE")?;
    let net_wm_state = intern(&conn, "_NET_WM_STATE")?;
    let net_fullscreen = intern(&conn, "_NET_WM_STATE_FULLSCREEN")?;
    let net_above = intern(&conn, "_NET_WM_STATE_ABOVE")?;
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;

    let attrs = conn.get_window_attributes(win)?.reply()?;
    if attrs.map_state == x11rb::protocol::xproto::MapState::UNMAPPED {
        if !OVERLAY_ARMED.load(SEQ) {
            return Ok(());
        }
        conn.map_window(win)?;
    }

    // NormalState = 1 (unminimize if GNOME iconified the new window).
    let unminimize = ClientMessageEvent::new(32, win, wm_change_state, [1, 0, 0, 0, 0]);
    conn.send_event(false, root, mask, unminimize)?;

    // ADD fullscreen + above, source 2. Minimize-to-snip hands focus to the
    // next window (Chrome); without ABOVE that window restacks over us.
    let fullscreen =
        ClientMessageEvent::new(32, win, net_wm_state, [1, net_fullscreen, net_above, 2, 0]);
    conn.send_event(false, root, mask, fullscreen)?;

    conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;

    let activate = ClientMessageEvent::new(32, win, net_active, [2, 0, 0, 0, 0]);
    conn.send_event(false, root, mask, activate)?;
    // Do not SetInputFocus from this second X connection: it fights GPUI's
    // focus/XIM state and leaves a mapped overlay that ignores keys.
    conn.flush()?;
    Ok(())
}

fn park_overlay_x11() -> anyhow::Result<()> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let mut ids = overlay_xids_by_title(&conn, root)?;
    if let Ok(guard) = OVERLAY_XID.lock() {
        if let Some(id) = *guard {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    for win in ids {
        let _ = conn.unmap_window(win);
    }
    conn.flush()?;
    Ok(())
}
