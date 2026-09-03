use gpui::{App, AppContext, ClipboardItem, Context};
use uuid::Uuid;

use crate::doc::SnipKind;
use crate::export::CopyKind;
use crate::identity::APP_SLUG;

use super::AppState;

impl AppState {
    pub fn copy_selected(&mut self, cx: &mut App) -> Option<(Uuid, CopyKind)> {
        let doc = self.selected_doc()?;
        let id = doc.id;
        let snip = doc.snip_kind();
        let kind = self.prefs.copy_habit.resolve(snip, self.export_fmt);
        let text = doc.text_for(kind, &self.prefs);
        if text.is_empty() {
            return None;
        }
        Self::write_clipboard(text, cx);
        self.record_copy_habit(snip, kind);
        Some((id, kind))
    }

    pub fn copy_chip_payload(&mut self, kind: CopyKind, text: String, cx: &mut App) {
        let Some(doc) = self.selected_doc() else {
            return;
        };
        let snip = doc.snip_kind();
        if text.is_empty() {
            return;
        }
        Self::write_clipboard(text, cx);
        self.record_copy_habit(snip, kind);
    }

    fn record_copy_habit(&mut self, snip: SnipKind, kind: CopyKind) {
        if self.prefs.copy_habit.remember(snip, kind) {
            self.persist_prefs();
        }
    }

    pub fn open_docx_selected(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.selected_doc() else {
            return;
        };
        if !doc.has_ready_blocks() {
            return;
        }
        let blocks = doc.blocks.clone();
        let id = doc.id;
        cx.spawn(async move |this, cx| {
            let built = cx
                .background_spawn(async move {
                    let bytes = crate::office::build_docx(&blocks)?;
                    let path = std::env::temp_dir().join(format!("{APP_SLUG}-{id}.docx"));
                    std::fs::write(&path, bytes)?;
                    anyhow::Ok(path)
                })
                .await;
            match built {
                Ok(path) => {
                    let _ = this.update(cx, |_, cx| {
                        cx.open_with_system(&path);
                    });
                }
                Err(err) => {
                    eprintln!("{APP_SLUG}: open docx: {err:#}");
                    let _ = this.update(cx, |this, cx| {
                        this.flash_error("Couldn't open a Word document", cx);
                    });
                }
            }
        })
        .detach();
    }

    fn write_clipboard(text: String, cx: &mut App) {
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected() else {
            return;
        };
        self.ocr.remove(id);
        self.finish_intake_id(id, false, cx);
        self.library.remove(id);
        self.documents.drop_doc(id);
        if self.library.is_empty() {
            self.ocr.cancel_remaining();
        }
        if let Some(writer) = self.store_writer() {
            if let Err(err) = writer.delete(id) {
                eprintln!("{APP_SLUG}: queue delete snip: {err:#}");
                self.flash_error("Couldn't delete that snip", cx);
            }
        }
        if let Some(id) = self.library.selected() {
            self.ensure_detail(id, cx);
        }
        self.pump_ocr(cx);
        cx.notify();
    }

    pub fn wipe_library(&mut self, cx: &mut Context<Self>) {
        self.ocr.cancel_remaining();
        if let Some(id) = self.ocr.running() {
            self.ocr.remove(id);
        }
        self.intake = None;
        self.documents.clear_docs();
        self.search.bump();
        self.library.clear();
        if let Some(writer) = self.store_writer() {
            if let Err(err) = writer.wipe() {
                eprintln!("{APP_SLUG}: queue wipe library: {err:#}");
                self.flash_error("Couldn't clear the snip library", cx);
            }
        }
        cx.notify();
    }
}
