// Explorer (and shortcuts) attach a console to CONSOLE-subsystem PE files.
// `dev-opt` / `release` inherit `debug_assertions = false`, so those builds
// are WINDOWS-subsystem. Default `cargo run` stays CONSOLE so logs still print.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

rust_i18n::i18n!("locales", fallback = "en");

mod actions;
mod autostart;
mod cache;
mod capture;
mod desktop;
mod doc;
mod export;
mod i18n;
mod icon;
mod identity;
mod imgutil;
mod instance;
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
    App, AppContext, Bounds, Entity, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions,
    px, size,
};

use crate::actions::{
    Capture, CopyExport, DeleteSelected, OpenDocx, OpenSettings, PasteSnip, QuitApp, RetryOcr,
    StartDraw, ToggleFormat, ToggleSidebar, UploadImage,
};
use crate::i18n::t;
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
    #[cfg(target_os = "linux")]
    // SAFETY: No application threads exist before the instance service starts.
    unsafe {
        pin_display_vulkan();
    }
    match instance::claim() {
        instance::Claim::AlreadyRunning => {}
        instance::Claim::Primary(seat) => run(seat),
    }
}

#[cfg(all(test, target_os = "windows"))]
#[test]
#[ignore = "interactive Windows UI; requires LOCALTEX_TEST_ROOT"]
fn interactive_windows_ui() {
    let root = std::env::var_os("LOCALTEX_TEST_ROOT").expect("set an isolated LOCALTEX_TEST_ROOT");
    assert!(std::path::Path::new(&root).is_absolute());
    let store = crate::store::Store::open(crate::identity::data_dir()).unwrap();
    if store.list().unwrap().is_empty() {
        let mut doc = crate::doc::Document::pending(std::sync::Arc::new(
            image::RgbaImage::from_pixel(64, 32, image::Rgba([255; 4])),
        ));
        doc.blocks = vec![crate::doc::Block::new(
            crate::doc::BlockKind::Text,
            crate::doc::Rect {
                x: 0,
                y: 0,
                w: 64,
                h: 32,
            },
            "Isolated UI test content",
        )];
        doc.ocr_blocks = doc.blocks.clone();
        doc.status = crate::doc::DocStatus::Ready;
        store.insert_ready(&doc).unwrap();
    }
    drop(store);
    main();
}

fn run(mut seat: instance::Seat) {
    let startup_mode = StartupMode::from_args();
    crate::icon::install_desktop_identity();
    let engine = std::thread::spawn(crate::ocr::Engine::load);
    gpui_platform::application()
        .with_assets(crate::icon::Assets)
        .run(move |cx: &mut App| {
            let prefs = crate::prefs::Prefs::load();
            crate::i18n::set_language(prefs.ui_lang);
            crate::keymap::apply(cx, &prefs.shortcuts);
            set_app_menus(cx);

            let engine = engine.join().expect("model discovery thread");
            let state = cx.new(|_| AppState::new(prefs, engine));
            let quit_state = state.clone();
            cx.on_action(move |_: &QuitApp, cx| {
                quit_state.update(cx, |state, cx| state.request_quit(cx));
            });
            if startup_mode == StartupMode::Interactive {
                open_main_window(state.clone(), cfg!(target_os = "windows"), cx)
                    .expect("open main window");
            }
            state.update(cx, |state, cx| {
                state.start_model_inspect(cx);
                state.bootstrap_store(startup_mode == StartupMode::Interactive, cx);
            });

            // Hotkey manager must be created on this GPUI UI thread (Windows
            // win32 loop / macOS main thread). Event recv is forwarded off-thread.
            let (tx, rx) = desktop::spawn();
            seat.serve(tx);
            seat.hold();
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
    if state.read(cx).has_main_window() {
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
            focus: crate::desktop::window_open_focus(activate),
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
    #[cfg(target_os = "windows")]
    crate::desktop::attach_main_window();
    state.update(cx, |state, cx| {
        state.open_main(handle);
        state.boot_selected(cx);
    });
    if activate {
        crate::desktop::focus_new_main(handle, cx)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
unsafe fn pin_display_vulkan() {
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
        // SAFETY: The caller runs this before starting any application threads.
        unsafe {
            std::env::set_var("VK_DRIVER_FILES", path);
            std::env::set_var("VK_ICD_FILENAMES", path);
        }
        eprintln!("{}: pin Vulkan ICD {path}", crate::identity::APP_SLUG);
    }
}

pub(crate) fn set_app_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: APP_NAME.into(),
            disabled: false,
            items: vec![
                MenuItem::action(t("menu.snip"), Capture),
                MenuItem::action(t("menu.upload_image"), UploadImage),
                MenuItem::action(t("menu.paste_image"), PasteSnip),
                MenuItem::action(t("menu.draw_snip"), StartDraw),
                MenuItem::action(t("menu.copy"), CopyExport),
                MenuItem::action(t("menu.open_docx"), OpenDocx),
                MenuItem::separator(),
                MenuItem::action(t("menu.settings"), OpenSettings),
                MenuItem::separator(),
                MenuItem::action(t("menu.quit"), QuitApp),
            ],
        },
        Menu {
            name: t("menu.edit").into(),
            disabled: false,
            items: vec![
                MenuItem::action(t("menu.delete_snip"), DeleteSelected),
                MenuItem::action(t("menu.retry_ocr"), RetryOcr),
            ],
        },
        Menu {
            name: t("menu.view").into(),
            disabled: false,
            items: vec![
                MenuItem::action(t("menu.toggle_format"), ToggleFormat),
                MenuItem::action(t("menu.toggle_sidebar"), ToggleSidebar),
            ],
        },
    ]);
}
