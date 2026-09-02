use std::time::Duration;

use gpui::{Context, Entity, Focusable, Window};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::source_editor::SourceEditor;

pub(crate) struct SourceBinding {
    pub(crate) open: bool,
    pub(crate) editor: Entity<SourceEditor>,
    bound: Option<Uuid>,
    last: String,
    pub(crate) split: f32,
    flush_task: Option<gpui::Task<()>>,
    pub(crate) lang: &'static str,
}

impl SourceBinding {
    pub(crate) fn new(editor: Entity<SourceEditor>) -> Self {
        Self {
            open: false,
            editor,
            bound: None,
            last: String::new(),
            split: 0.5,
            flush_task: None,
            lang: "LaTeX",
        }
    }
}

impl MainWindow {
    pub(crate) fn set_source_open(
        &mut self,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ready = self
            .state
            .read(cx)
            .selected_doc()
            .is_some_and(|d| d.has_ready_blocks());
        if on && !ready {
            return;
        }
        if on == self.source_panel.open {
            if on {
                self.bind_source(cx);
                window.focus(&self.source_panel.editor.focus_handle(cx), cx);
            }
            return;
        }
        if on {
            self.source_panel.open = true;
            self.bind_source(cx);
            window.focus(&self.source_panel.editor.focus_handle(cx), cx);
        } else {
            self.close_source(cx);
            window.focus(&self.snip_list_focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn close_source(&mut self, cx: &mut Context<Self>) {
        self.source_panel.flush_task.take();
        if self.source_panel.open {
            self.flush_source(cx);
        }
        self.source_panel.open = false;
        self.source_panel.bound = None;
    }

    pub(crate) fn bind_source(&mut self, cx: &mut Context<Self>) {
        let id = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                return;
            };
            doc.id
        };
        if self.source_panel.bound == Some(id) {
            return;
        }
        if self.source_panel.bound.is_some() {
            self.flush_source(cx);
        }
        let (prefs, blocks) = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                return;
            };
            (state.prefs.clone(), doc.blocks.clone())
        };
        let text = crate::source::blocks_to_source(&blocks, &prefs);
        self.source_panel.lang =
            crate::source::editor_lang_label(crate::doc::snip_kind(&blocks), &text);
        self.source_panel.last = text.clone();
        self.source_panel.bound = Some(id);
        self.source_panel
            .editor
            .update(cx, |ed, cx| ed.set_text(text, cx));
    }

    pub(crate) fn sync_source_for_selection(&mut self, cx: &mut Context<Self>) {
        if !self.source_panel.open {
            return;
        }
        let ready = self
            .state
            .read(cx)
            .selected_doc()
            .is_some_and(|d| d.has_ready_blocks());
        if !ready {
            self.close_source(cx);
            cx.notify();
            return;
        }
        self.bind_source(cx);
    }

    pub(crate) fn on_source_edit(&mut self, cx: &mut Context<Self>) {
        if !self.source_panel.open {
            return;
        }
        self.source_panel.flush_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(280))
                .await;
            this.update(cx, |this, cx| {
                this.flush_source(cx);
            })
            .ok();
        }));
    }

    fn flush_source(&mut self, cx: &mut Context<Self>) {
        if !self.source_panel.open {
            return;
        }
        let text = self.source_panel.editor.read(cx).text();
        if text == self.source_panel.last {
            return;
        }
        let prefs = self.state.read(cx).prefs.clone();
        let Ok(blocks) = crate::source::parse_source(&text, &prefs) else {
            return;
        };
        let Some(id) = self.source_panel.bound else {
            return;
        };
        self.source_panel.lang =
            crate::source::editor_lang_label(crate::doc::snip_kind(&blocks), &text);
        self.source_panel.last = text;
        self.state
            .update(cx, |state, cx| state.apply_parsed_source(id, blocks, cx));
    }

    pub(crate) fn revert_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.source_panel.bound else {
            return;
        };
        self.state.update(cx, |state, cx| state.revert_ocr(id, cx));
        self.source_panel.bound = None;
        self.bind_source(cx);
        window.focus(&self.source_panel.editor.focus_handle(cx), cx);
        cx.notify();
    }
}
