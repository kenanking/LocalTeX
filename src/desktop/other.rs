use std::sync::mpsc::Sender;
use std::thread;

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::desktop::DesktopCmd;
use crate::icon;
use crate::identity::{APP_NAME, APP_SLUG};

pub fn start_tray(tx: Sender<DesktopCmd>) -> Option<TrayIcon> {
    let icon = match app_icon() {
        Ok(icon) => icon,
        Err(err) => {
            eprintln!("{APP_SLUG}: tray icon pixels: {err}");
            return None;
        }
    };

    let capture = MenuItem::with_id("capture", "Capture", true, None);
    let show = MenuItem::with_id("show", "Show", true, None);
    let quit = MenuItem::with_id("quit", "Quit", true, None);
    let menu = Menu::new();
    for result in [
        menu.append(&capture),
        menu.append(&show),
        menu.append(&PredefinedMenuItem::separator()),
        menu.append(&quit),
    ] {
        if let Err(err) = result {
            eprintln!("{APP_SLUG}: tray menu: {err}");
            return None;
        }
    }

    let tray = match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(APP_NAME)
        .with_icon(icon)
        .build()
    {
        Ok(tray) => tray,
        Err(err) => {
            eprintln!("{APP_SLUG}: tray icon failed: {err}");
            return None;
        }
    };

    let menu_tx = tx.clone();
    thread::spawn(move || {
        let receiver = MenuEvent::receiver();
        while let Ok(event) = receiver.recv() {
            let cmd = if event.id == MenuId::new("capture") {
                DesktopCmd::Capture
            } else if event.id == MenuId::new("show") {
                DesktopCmd::Show
            } else if event.id == MenuId::new("quit") {
                DesktopCmd::Quit
            } else {
                continue;
            };
            if let Err(err) = menu_tx.send(cmd) {
                eprintln!("{APP_SLUG}: tray send: {err}");
                break;
            }
        }
    });

    thread::spawn(move || {
        let receiver = TrayIconEvent::receiver();
        while let Ok(event) = receiver.recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Err(err) = tx.send(DesktopCmd::Show) {
                    eprintln!("{APP_SLUG}: tray send: {err}");
                    break;
                }
            }
        }
    });

    Some(tray)
}

fn app_icon() -> Result<Icon, String> {
    let (w, h, rgba) = icon::rgba_bytes(32);
    Icon::from_rgba(rgba, w, h).map_err(|err| err.to_string())
}
