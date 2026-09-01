use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext, AsyncApp, Context, WeakEntity};
use image::RgbaImage;
use uuid::Uuid;

use crate::doc::ImageSlot;
use crate::identity::APP_SLUG;
use crate::store::Store;

use super::AppState;

enum OrigPng {
    Encode(Arc<RgbaImage>),
    File {
        store: Arc<Store>,
        id: Uuid,
        fallback: Option<Arc<RgbaImage>>,
    },
}

impl AppState {
    pub fn orig_copy_flashed(&self, id: Uuid) -> bool {
        self.orig_copy_flash == Some(id)
    }

    pub fn can_reveal_original(&self, id: Uuid) -> bool {
        self.store().is_some() && self.library.get(id).is_some_and(|d| d.is_persisted())
    }

    pub fn copy_original(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(job) = self.orig_png_job(id) else {
            self.flash_capture_error("Couldn't copy that image", cx);
            return;
        };
        cx.spawn(async move |this, cx| {
            let bytes = cx
                .background_spawn(async move { load_png_bytes(job) })
                .await;
            finish_orig_work(
                this,
                cx,
                "copy original",
                "Couldn't copy that image",
                |this, cx| {
                    crate::desktop::write_clipboard_png(bytes?, cx)?;
                    this.flash_orig_copy(id, cx);
                    Ok(())
                },
            );
        })
        .detach();
    }

    pub fn save_original_as(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(job) = self.orig_png_job(id) else {
            self.flash_capture_error("Couldn't save that image", cx);
            return;
        };
        let stem = self
            .library
            .get(id)
            .map(|d| png_export_stem(&d.first_line()))
            .unwrap_or_else(|| "snip".to_string());
        cx.spawn(async move |this, cx| {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("PNG", &["png"])
                .set_file_name(format!("{stem}.png"))
                .save_file()
                .await;
            let Some(handle) = picked else {
                return;
            };
            let dest = handle.path().to_path_buf();
            let written = cx
                .background_spawn(async move { save_orig_png(job, &dest) })
                .await;
            finish_orig_work(
                this,
                cx,
                "save original",
                "Couldn't save that image",
                |_, _| written,
            );
        })
        .detach();
    }

    pub fn reveal_original(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.can_reveal_original(id) {
            self.flash_capture_error("Image isn't saved yet", cx);
            return;
        }
        let Some(store) = self.store() else {
            self.flash_capture_error("Image isn't saved yet", cx);
            return;
        };
        cx.spawn(async move |this, cx| {
            let path = cx.background_spawn(async move { store.png_path(id) }).await;
            finish_orig_work(
                this,
                cx,
                "reveal original",
                "Couldn't open that folder",
                |_, cx| {
                    cx.reveal_path(&path?);
                    Ok(())
                },
            );
        })
        .detach();
    }

    fn orig_png_job(&self, id: Uuid) -> Option<OrigPng> {
        let doc = self.library.get(id)?;
        match &doc.image {
            ImageSlot::Missing => None,
            ImageSlot::OnDisk => self.store().map(|store| OrigPng::File {
                store,
                id,
                fallback: None,
            }),
            ImageSlot::Loaded(img) => {
                if doc.is_persisted() {
                    if let Some(store) = self.store() {
                        return Some(OrigPng::File {
                            store,
                            id,
                            fallback: Some(img.clone()),
                        });
                    }
                }
                Some(OrigPng::Encode(img.clone()))
            }
        }
    }

    fn flash_orig_copy(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.orig_copy_flash = Some(id);
        self.orig_copy_flash_gen = self.orig_copy_flash_gen.wrapping_add(1);
        let gen = self.orig_copy_flash_gen;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.orig_copy_flash_gen == gen {
                    this.orig_copy_flash = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

fn finish_orig_work(
    this: WeakEntity<AppState>,
    cx: &mut AsyncApp,
    op: &'static str,
    fail_msg: &'static str,
    work: impl FnOnce(&mut AppState, &mut Context<AppState>) -> anyhow::Result<()>,
) {
    if let Err(err) = this.update(cx, |this, cx| {
        if let Err(err) = work(this, cx) {
            eprintln!("{APP_SLUG}: {op}: {err:#}");
            this.flash_capture_error(fail_msg, cx);
        }
    }) {
        eprintln!("{APP_SLUG}: {op} task: {err}");
    }
}

fn load_png_bytes(job: OrigPng) -> anyhow::Result<Vec<u8>> {
    match job {
        OrigPng::Encode(img) => crate::imgutil::encode_png_fast(img.as_ref()),
        OrigPng::File {
            store,
            id,
            fallback,
        } => match store.png_path(id) {
            Ok(path) => Ok(std::fs::read(path)?),
            Err(_) => {
                let img = fallback.ok_or_else(|| anyhow::anyhow!("png missing"))?;
                crate::imgutil::encode_png_fast(img.as_ref())
            }
        },
    }
}

fn save_orig_png(job: OrigPng, dest: &Path) -> anyhow::Result<()> {
    match job {
        OrigPng::Encode(img) => {
            std::fs::write(dest, crate::imgutil::encode_png_fast(img.as_ref())?)?;
        }
        OrigPng::File {
            store,
            id,
            fallback,
        } => match store.png_path(id) {
            Ok(src) => {
                std::fs::copy(src, dest)?;
            }
            Err(_) => {
                let img = fallback.ok_or_else(|| anyhow::anyhow!("png missing"))?;
                std::fs::write(dest, crate::imgutil::encode_png_fast(img.as_ref())?)?;
            }
        },
    }
    Ok(())
}

fn png_export_stem(first_line: &str) -> String {
    const MAX: usize = 80;
    let mut out = String::new();
    let mut last_space = false;
    for ch in first_line.chars() {
        if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            continue;
        }
        if ch.is_whitespace() {
            if out.is_empty() || last_space {
                continue;
            }
            last_space = true;
            out.push(' ');
        } else {
            last_space = false;
            out.push(ch);
        }
        if out.chars().count() >= MAX {
            break;
        }
    }
    let stem = out.trim();
    if stem.is_empty() {
        "snip".to_string()
    } else {
        stem.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::png_export_stem;

    #[test]
    fn png_export_stem_empty_falls_back() {
        assert_eq!(png_export_stem(""), "snip");
        assert_eq!(png_export_stem("   "), "snip");
        assert_eq!(png_export_stem("/\\:*?\"<>|"), "snip");
    }

    #[test]
    fn png_export_stem_strips_slashes_and_illegal() {
        assert_eq!(png_export_stem("a/b\\c:d"), "abcd");
        assert_eq!(png_export_stem("hello   world"), "hello world");
        assert_eq!(
            png_export_stem(" CAPM expected return "),
            "CAPM expected return"
        );
        assert_eq!(png_export_stem("a\nb"), "ab");
    }

    #[test]
    fn png_export_stem_caps_long_names() {
        let long = "x".repeat(120);
        let stem = png_export_stem(&long);
        assert_eq!(stem.chars().count(), 80);
        assert!(stem.chars().all(|c| c == 'x'));
    }
}
