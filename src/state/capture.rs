use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext, Context, Window, WindowHandle};
use image::RgbaImage;

use super::ingest::{is_ingest_image_path, IngestSource, IMAGE_EXTS};
use super::session::Capture;
use super::{AppState, MainWindowState};
use crate::identity::APP_SLUG;

const SHEET_DISMISS_SETTLE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy)]
struct HidePlan {
    push_hide: bool,
    await_iconify: bool,
}

fn capture_hide_plan(hide_pref: bool, has_window: bool) -> HidePlan {
    HidePlan {
        push_hide: hide_pref,
        await_iconify: hide_pref && has_window,
    }
}

impl AppState {
    pub fn is_capturing(&self) -> bool {
        self.capture.is_grabbing()
    }

    pub fn capture_error(&self) -> Option<&str> {
        self.capture.error()
    }

    pub(super) fn flash_error(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.capture.set(Capture::Failed(msg.into()));
        let gen = self.capture.gen();
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
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
        if self.is_bootstrapping() {
            self.capture_after_bootstrap = true;
            return;
        }
        if self.capture.is_grabbing() {
            return;
        }
        self.capture.set(Capture::Grabbing);
        cx.notify();
        self.dismiss_main_sheet(cx);
        crate::desktop::prepare_snip_input();

        let plan = capture_hide_plan(self.prefs.hide_on_capture, self.has_main_window());
        if plan.push_hide {
            self.capture.push_hide();
        }
        if plan.await_iconify {
            self.iconify_main(cx);
        }

        cx.spawn(async move |this, cx| {
            if plan.await_iconify {
                if let Err(err) = cx
                    .background_spawn(async { crate::desktop::wait_until_iconified() })
                    .await
                {
                    let _ = this.update(cx, |this, cx| {
                        this.flash_error(err.to_string(), cx);
                        this.restore_after_hide(cx);
                    });
                    return;
                }
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
                    this.restore_after_hide(cx);
                    cx.notify();
                }
                Err(err) => {
                    this.flash_error(err.to_string(), cx);
                    this.restore_after_hide(cx);
                }
            }) {
                eprintln!("{APP_SLUG}: capture task: {err}");
            }
        })
        .detach();
    }

    pub fn finish_capture(&mut self, crop: RgbaImage, cx: &mut Context<Self>) {
        self.capture.set(Capture::Idle);
        self.ingest(IngestSource::Screen(crop), cx);
        self.dismiss_main_sheet(cx);
        self.restore_after_hide(cx);
    }

    pub fn request_upload(&mut self, cx: &mut Context<Self>) {
        if self.is_bootstrapping() {
            return;
        }
        if self.is_capturing() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("Images", IMAGE_EXTS)
                .pick_files()
                .await;
            let Some(handles) = picked else {
                return;
            };
            let paths: Vec<PathBuf> = handles.iter().map(|h| h.path().to_path_buf()).collect();
            if let Err(err) = this.update(cx, |this, cx| {
                this.offer_files(paths, cx);
            }) {
                eprintln!("{APP_SLUG}: upload task: {err}");
            }
        })
        .detach();
    }

    pub fn request_paste(&mut self, cx: &mut Context<Self>) {
        if self.is_bootstrapping() {
            return;
        }
        if self.is_capturing() || self.ingest.clipboard_loading {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            self.ingest.clipboard_loading = true;
            cx.spawn(async move |this, cx| {
                let candidates = cx
                    .background_spawn(async { crate::desktop::read_clipboard_image() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.ingest.clipboard_loading = false;
                    if candidates.is_empty() {
                        this.request_paste_fallback(cx);
                    } else {
                        this.decode_native_clipboard(candidates, cx);
                    }
                });
            })
            .detach();
            return;
        }
        #[cfg(not(target_os = "windows"))]
        self.request_paste_fallback(cx);
    }

    #[cfg(target_os = "windows")]
    fn decode_native_clipboard(&mut self, candidates: Vec<Vec<u8>>, cx: &mut Context<Self>) {
        // Windows: GPUI skips CF_DIB/CF_BITMAP and can return text when a
        // bitmap is also present. Try every native candidate (PNG may be a
        // stub; DIB/HBITMAP still work).
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
    }

    fn request_paste_fallback(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            self.flash_error("Clipboard is empty — copy an image first", cx);
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
        self.flash_error("Nothing to paste — copy an image first", cx);
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
                Ok(img) => {
                    this.ingest_pixels(img, cx);
                }
                Err(err) => {
                    eprintln!("{APP_SLUG}: clipboard image: {err}");
                    this.flash_error("Couldn't read that clipboard image", cx);
                }
            }) {
                eprintln!("{APP_SLUG}: paste task: {err}");
            }
        })
        .detach();
    }

    pub(super) fn iconify_main(&mut self, cx: &mut Context<Self>) {
        // EWMH HIDDEN does not nest `handle.update` (in-app Snip runs while the
        // main window is already on GPUI's update stack).
        crate::desktop::iconify_main_window();
        self.defer_minimize(cx, "minimize main");
    }

    fn mark_hidden(&mut self, cx: &mut Context<Self>) {
        self.main_window.set_visible(false);
        self.schedule_hidden_media_release(cx);
        crate::desktop::hide_main_to_tray();
    }

    fn defer_minimize(&self, cx: &mut Context<Self>, fail_label: &'static str) {
        let handle = self.main_window.handle();
        cx.defer(move |cx| {
            if let Some(handle) = handle {
                if let Err(err) = handle.update(cx, |_, window, _| {
                    window.minimize_window();
                }) {
                    eprintln!("{APP_SLUG}: {fail_label}: {err}");
                }
            }
        });
    }

    pub fn minimize_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mark_hidden(cx);
        #[cfg(not(target_os = "windows"))]
        window.minimize_window();
        #[cfg(target_os = "windows")]
        let _ = window;
    }

    pub(crate) fn hide_to_tray(&mut self, cx: &mut Context<Self>) {
        self.mark_hidden(cx);
        #[cfg(not(target_os = "windows"))]
        self.defer_minimize(cx, "hide to tray");
    }

    pub(super) fn dismiss_main_sheet(&self, cx: &mut Context<Self>) {
        let handle = self.main_window.handle();
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
        self.main_window.set_visible(true);
        self.hidden_media_release_task = None;
        if self.main_window.handle().is_none() {
            if self.main_window.is_opening() {
                return;
            }
            self.main_window = MainWindowState::Opening;
            let state = cx.entity();
            cx.defer(move |cx| {
                if let Err(err) = crate::open_main_window(state.clone(), true, cx) {
                    eprintln!("{APP_SLUG}: open main window: {err:#}");
                    state.update(cx, |state, cx| {
                        state.main_window = MainWindowState::Closed;
                        state.flash_error("Couldn't open the main window", cx);
                    });
                }
            });
            return;
        }
        self.boot_selected(cx);
        crate::desktop::deiconify_main_window();
        let handle = self.main_window.handle();
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
        .filter(|path| path.is_file() && is_ingest_image_path(path))
        .collect()
}

fn activate_window<V: 'static>(handle: WindowHandle<V>, cx: &mut App) {
    if let Err(err) = handle.update(cx, |_, window, _| {
        window.activate_window();
    }) {
        eprintln!("{APP_SLUG}: activate window: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_hide_plan_skips_iconify_wait_without_window() {
        let p = capture_hide_plan(true, false);
        assert!(p.push_hide, "still restore/open the window after the snip");
        assert!(!p.await_iconify, "nothing to iconify without a window");
        let p = capture_hide_plan(true, true);
        assert!(p.push_hide && p.await_iconify);
        let p = capture_hide_plan(false, true);
        assert!(!p.push_hide && !p.await_iconify);
    }
}
