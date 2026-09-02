// Explorer (and shortcuts) attach a console to CONSOLE-subsystem PE files.
// `dev-opt` / `release` inherit `debug_assertions = false`, so those builds
// are WINDOWS-subsystem. Default `cargo run` stays CONSOLE so logs still print.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod autostart;
mod cache;
mod capture;
mod desktop;
mod doc;
mod export;
mod icon;
mod identity;
mod imgutil;
mod keymap;
mod library;
mod math;
mod ocr;
mod ocr_queue;
mod office;
mod prefs;
mod preview;
mod source;
mod state;
mod store;
mod sysmon;
mod table;
mod ui;

#[cfg(target_os = "linux")]
use gpui::WindowDecorations;
use gpui::{
    px, size, App, AppContext, Bounds, Entity, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions,
};

use crate::actions::{
    Capture, CopyExport, DeleteSelected, OpenDocx, OpenSettings, PasteSnip, QuitApp, RetryOcr,
    StartDraw, ToggleFormat, UploadImage,
};
use crate::identity::{APP_ID, APP_NAME};
use crate::state::AppState;
use crate::ui::MainWindow;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartupMode {
    Interactive,
    Autostart,
}

impl StartupMode {
    fn from_args() -> Self {
        if std::env::args_os().skip(1).any(|arg| arg == "--autostart") {
            Self::Autostart
        } else {
            Self::Interactive
        }
    }
}

fn main() {
    let startup_mode = StartupMode::from_args();
    pin_display_vulkan();
    crate::icon::install_desktop_identity();
    gpui_platform::application()
        .with_assets(crate::icon::Assets)
        .run(move |cx: &mut App| {
            let prefs = crate::prefs::Prefs::load();
            let launch_at_startup = prefs.launch_at_startup;
            cx.background_spawn(async move {
                if let Err(err) = crate::autostart::apply(launch_at_startup) {
                    eprintln!("{}: autostart: {err:#}", crate::identity::APP_SLUG);
                }
            })
            .detach();
            crate::state::bind_keys(cx, &prefs.shortcuts);
            set_app_menus(cx);

            let state = cx.new(|_| AppState::new(prefs));
            if startup_mode == StartupMode::Interactive {
                open_main_window(state.clone(), false, cx).expect("open main window");
            }
            state.update(cx, |state, cx| {
                state.bootstrap_store(startup_mode == StartupMode::Interactive, cx)
            });

            // Hotkey manager must be created on this GPUI UI thread (Windows
            // win32 loop / macOS main thread). Event recv is forwarded off-thread.
            let rx = desktop::spawn();
            state.update(cx, |state, _| {
                desktop::rebind_globals(&state.prefs.shortcuts);
            });
            state::pump_desktop_events(state.clone(), rx, cx);
            if startup_mode == StartupMode::Interactive {
                cx.activate(true);
            }
        });
}

pub(crate) fn open_main_window(
    state: Entity<AppState>,
    activate: bool,
    cx: &mut App,
) -> anyhow::Result<()> {
    if state.read(cx).main_window.is_some() {
        return Ok(());
    }
    let bounds = Bounds::centered(None, size(px(800.), px(560.)), cx);
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                ..Default::default()
            }),
            focus: activate,
            app_id: Some(APP_ID.into()),
            window_min_size: Some(size(px(520.), px(400.))),
            window_background: gpui::WindowBackgroundAppearance::Opaque,
            // GNOME leaves server-decorated X11 clients unmapped when
            // its mutter-x11-frames helper dies. Own the Linux frame.
            #[cfg(target_os = "linux")]
            window_decorations: Some(WindowDecorations::Client),
            ..Default::default()
        },
        {
            let state = state.clone();
            move |window, cx| cx.new(|cx| MainWindow::new(state, window, cx))
        },
    )?;
    state.update(cx, |state, cx| {
        state.main_window = Some(handle);
        state.main_window_opening = false;
        state.main_window_visible = true;
        state.boot_selected(cx);
    });
    if activate {
        handle.update(cx, |_, window, _| window.activate_window())?;
    }
    Ok(())
}

fn pin_display_vulkan() {
    if std::env::var_os("VK_DRIVER_FILES").is_some()
        || std::env::var_os("VK_ICD_FILENAMES").is_some()
    {
        return;
    }
    let candidates = [
        "/usr/share/vulkan/icd.d/nvidia_icd.json",
        "/etc/vulkan/icd.d/nvidia_icd.json",
        "/usr/share/vulkan/icd.d/intel_icd.json",
        "/usr/share/vulkan/icd.d/radeon_icd.json",
    ];
    if let Some(path) = candidates
        .iter()
        .copied()
        .find(|p| std::path::Path::new(p).exists())
    {
        std::env::set_var("VK_DRIVER_FILES", path);
        std::env::set_var("VK_ICD_FILENAMES", path);
        eprintln!("{}: pin Vulkan ICD {path}", crate::identity::APP_SLUG);
    }
}

fn set_app_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: APP_NAME.into(),
            disabled: false,
            items: vec![
                MenuItem::action("Snip", Capture),
                MenuItem::action("Upload Image…", UploadImage),
                MenuItem::action("Paste Image", PasteSnip),
                MenuItem::action("Draw Snip", StartDraw),
                MenuItem::action("Copy", CopyExport),
                MenuItem::action("Open DOCX", OpenDocx),
                MenuItem::separator(),
                MenuItem::action("Settings", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit", QuitApp),
            ],
        },
        Menu {
            name: "Edit".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Delete Snip", DeleteSelected),
                MenuItem::action("Retry OCR", RetryOcr),
            ],
        },
        Menu {
            name: "View".into(),
            disabled: false,
            items: vec![MenuItem::action("Toggle Markdown / LaTeX", ToggleFormat)],
        },
    ]);
}
