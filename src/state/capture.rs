use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext, Context, Timer, WindowHandle};
use image::RgbaImage;

use crate::identity::APP_SLUG;
use crate::ingest::IngestSource;

use super::session::Capture;
use super::AppState;

const SHEET_DISMISS_SETTLE: Duration = Duration::from_millis(250);

impl AppState {
    pub fn is_capturing(&self) -> bool {
        self.capture.is_grabbing()
    }

    pub fn capture_error(&self) -> Option<&str> {
        self.capture.error()
    }

    pub(super) fn flash_capture_error(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.capture.set(Capture::Failed(msg.into()));
        let gen = self.capture.gen();
        cx.notify();
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_secs(4)).await;
            let _ = this.update(cx, |this, cx| {
                if this.capture.should_clear_flash(gen) {
                    this.capture.set(Capture::Idle);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn request_capture(&mut self, cx: &mut Context<Self>) {
        if self.capture.is_grabbing() {
            return;
        }
        self.capture.set(Capture::Grabbing);
        cx.notify();
        self.dismiss_main_sheet(cx);
        crate::desktop::prepare_snip_input();

        let hide = self.prefs.hide_on_capture;
        if hide {
            self.hide_main(cx);
        }

        cx.spawn(async move |this, cx| {
            if hide {
                cx.background_spawn(async { crate::desktop::wait_until_iconified() })
                    .await;
            } else {
                cx.background_spawn(async {
                    std::thread::sleep(SHEET_DISMISS_SETTLE);
                })
                .await;
            }
            let picked = cx
                .background_spawn(async {
                    let shot = crate::capture::grab_desktop()?;
                    crate::desktop::select_region(&shot)
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| match picked {
                Ok(Some(crop)) => this.finish_capture(crop, cx),
                Ok(None) => {
                    this.capture.set(Capture::Idle);
                    this.capture.set_reveal_on_main(false);
                    this.restore_after_hide(cx);
                    cx.notify();
                }
                Err(err) => {
                    this.flash_capture_error(err.to_string(), cx);
                    this.restore_after_hide(cx);
                }
            }) {
                eprintln!("{APP_SLUG}: capture task: {err}");
            }
        })
        .detach();
    }

    pub fn finish_capture(&mut self, crop: RgbaImage, cx: &mut Context<Self>) {
        self.capture.set_reveal_on_main(true);
        self.capture.set(Capture::Idle);
        self.ingest(IngestSource::Screen(crop), cx);
    }

    pub fn request_upload(&mut self, cx: &mut Context<Self>) {
        if self.is_capturing() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                .pick_files()
                .await;
            let Some(handles) = picked else {
                return;
            };
            let paths: Vec<PathBuf> = handles.iter().map(|h| h.path().to_path_buf()).collect();
            if let Err(err) = this.update(cx, |this, cx| {
                this.ingest(IngestSource::Files(paths), cx);
            }) {
                eprintln!("{APP_SLUG}: upload task: {err}");
            }
        })
        .detach();
    }

    pub fn request_paste(&mut self, cx: &mut Context<Self>) {
        if self.is_capturing() {
            return;
        }
        // Windows: GPUI skips CF_DIB/CF_BITMAP and can return text when a
        // bitmap is also present. Try every native candidate (PNG may be a
        // stub; DIB/HBITMAP still work).
        let candidates = crate::desktop::read_clipboard_image();
        if !candidates.is_empty() {
            self.spawn_paste_image(
                move || {
                    let mut last = None;
                    for raw in candidates {
                        match crate::desktop::decode_clipboard_image(raw) {
                            Ok(img) => return Ok(img),
                            Err(err) => last = Some(err),
                        }
                    }
                    Err(last.unwrap_or_else(|| anyhow::anyhow!("no clipboard image")))
                },
                cx,
            );
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            self.flash_capture_error("Clipboard is empty — copy an image first", cx);
            return;
        };
        let image_bytes = item.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(img) => Some(img.bytes.clone()),
            _ => None,
        });
        if let Some(bytes) = image_bytes {
            self.spawn_paste_image(
                move || {
                    image::load_from_memory(&bytes)
                        .map(|d| d.to_rgba8())
                        .map_err(|err| anyhow::anyhow!("{err}"))
                },
                cx,
            );
            return;
        }
        let paths = item
            .text()
            .map(|text| clipboard_image_paths(&text))
            .unwrap_or_default();
        if !paths.is_empty() {
            self.ingest(IngestSource::Files(paths), cx);
            return;
        }
        self.flash_capture_error("Nothing to paste — copy an image first", cx);
    }

    fn spawn_paste_image<F>(&mut self, decode: F, cx: &mut Context<Self>)
    where
        F: FnOnce() -> anyhow::Result<image::RgbaImage> + Send + 'static,
    {
        self.capture.set(Capture::Idle);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let decoded = cx.background_spawn(async move { decode() }).await;
            if let Err(err) = this.update(cx, |this, cx| match decoded {
                Ok(img) => this.ingest_pixels(img, cx),
                Err(err) => {
                    eprintln!("{APP_SLUG}: clipboard image: {err}");
                    this.flash_capture_error("Couldn't read that clipboard image", cx);
                }
            }) {
                eprintln!("{APP_SLUG}: paste task: {err}");
            }
        })
        .detach();
    }

    fn hide_main(&mut self, cx: &mut Context<Self>) {
        self.capture.push_hide();
        // EWMH HIDDEN does not nest `handle.update` (in-app Snip runs while the
        // main window is already on GPUI's update stack).
        crate::desktop::iconify_main_window();
        let handle = self.main_window;
        cx.defer(move |cx| {
            if let Some(handle) = handle {
                if let Err(err) = handle.update(cx, |_, window, _| {
                    window.minimize_window();
                }) {
                    eprintln!("{APP_SLUG}: minimize main: {err}");
                }
            }
        });
    }

    pub(super) fn dismiss_main_sheet(&self, cx: &mut Context<Self>) {
        let handle = self.main_window;
        cx.defer(move |cx| {
            if let Some(handle) = handle {
                if let Err(err) = handle.update(cx, |view, _, cx| {
                    view.dismiss_sheet(cx);
                }) {
                    eprintln!("{APP_SLUG}: dismiss sheet: {err}");
                }
            }
        });
    }

    pub(super) fn restore_after_hide(&mut self, cx: &mut Context<Self>) {
        if self.capture.pop_hide() {
            self.restore_main(cx);
        }
    }

    pub(super) fn restore_main(&mut self, cx: &mut Context<Self>) {
        self.capture.force_show();
        crate::desktop::deiconify_main_window();
        let handle = self.main_window;
        cx.defer(move |cx| {
            if let Some(handle) = handle {
                activate_window(handle, cx);
            }
        });
    }
}

fn clipboard_image_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| PathBuf::from(line.strip_prefix("file://").unwrap_or(line)))
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
                    )
                })
        })
        .collect()
}

fn activate_window<V: 'static>(handle: WindowHandle<V>, cx: &mut App) {
    if let Err(err) = handle.update(cx, |_, window, _| {
        window.activate_window();
    }) {
        eprintln!("{APP_SLUG}: activate window: {err}");
    }
}
