use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    div, point, prelude::*, px, rgb, AnyElement, App, Context, Entity, ScrollHandle, Window,
};

use super::scroll::{overlay_scrollbar, ScrollAxis, ScrollThumbDrag, ScrollbarTone};
use super::theme;
use super::widgets::{pill_tab, segmented, setting_row, switch};
use crate::keymap::ShortcutId;
use crate::prefs::Prefs;
use crate::state::AppState;

mod formatting;
mod general;
mod shortcuts;
mod system;

use formatting::formatting_page;
use general::general_page;
use shortcuts::shortcuts_page;
use system::system_page;

pub(crate) use shortcuts::intercept_recording;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Formatting,
    Shortcuts,
    System,
}

const TABS: [(&str, &str, SettingsTab); 4] = [
    ("set-general", "General", SettingsTab::General),
    ("set-formatting", "Formatting", SettingsTab::Formatting),
    ("set-shortcuts", "Shortcuts", SettingsTab::Shortcuts),
    ("set-system", "System", SettingsTab::System),
];

pub struct SettingsPane {
    tab: SettingsTab,
    listen: Option<ShortcutId>,
    scroll: ScrollHandle,
    thumb: Rc<RefCell<Option<ScrollThumbDrag>>>,
    sysmon: crate::sysmon::SysMon,
    sys_snap: crate::sysmon::SysSnapshot,
    sysmon_on: bool,
    visible: bool,
}

impl SettingsPane {
    pub fn new() -> Self {
        Self {
            tab: SettingsTab::General,
            listen: None,
            scroll: ScrollHandle::new(),
            thumb: Rc::new(RefCell::new(None)),
            sysmon: crate::sysmon::SysMon::new(),
            sys_snap: crate::sysmon::SysSnapshot::default(),
            sysmon_on: false,
            visible: false,
        }
    }

    pub fn listening(&self) -> Option<ShortcutId> {
        self.listen
    }

    pub fn set_listen(&mut self, id: Option<ShortcutId>) {
        self.listen = id;
    }

    pub fn dismiss_listen(&mut self) {
        self.listen = None;
        self.hide();
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn reset_scroll(&mut self) {
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.thumb.borrow_mut().take();
    }

    pub(crate) fn scroll_thumb(&self) -> Rc<RefCell<Option<ScrollThumbDrag>>> {
        Rc::clone(&self.thumb)
    }

    fn kick_sysmon(&mut self, cx: &mut Context<Self>) {
        if self.sysmon_on {
            return;
        }
        self.sysmon_on = true;
        cx.spawn(async move |this, cx| {
            let mut ticks = 0u32;
            loop {
                // CPU/memory are cheap Win32 /proc reads. Do not wait on the
                // models/snips walk first — that walk can sit in Defender for
                // a long time and left this tab showing "—" on Windows.
                let still = this
                    .update(cx, |this, cx| {
                        let disk = this.sys_snap.disk.clone();
                        let mut snap = this.sysmon.sample();
                        snap.disk = disk;
                        this.sys_snap = snap;
                        cx.notify();
                        this.tab == SettingsTab::System && this.visible
                    })
                    .unwrap_or(false);
                if !still {
                    break;
                }
                if ticks.is_multiple_of(crate::sysmon::DISK_EVERY_TICKS) {
                    let disk = cx
                        .background_spawn(async move { crate::sysmon::disk_sample() })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.sys_snap.disk = Some(disk);
                        cx.notify();
                    });
                }
                ticks += 1;
                cx.background_executor()
                    .timer(crate::sysmon::SAMPLE_INTERVAL)
                    .await;
            }
            let _ = this.update(cx, |this, _| {
                this.sysmon_on = false;
            });
        })
        .detach();
    }

    pub fn view(
        &mut self,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _ = window;
        self.visible = true;
        if self.tab == SettingsTab::System {
            self.kick_sysmon(cx);
        }

        let prefs = state.read(cx).prefs.clone();
        let tab = self.tab;
        let listen = self.listen;
        let entity = cx.entity();
        let on_tab = {
            let entity = entity.clone();
            move |tab, cx: &mut App| {
                entity.update(cx, |this, cx| {
                    this.tab = tab;
                    this.listen = None;
                    this.reset_scroll();
                    cx.notify();
                });
            }
        };
        let on_listen = {
            let entity = entity.clone();
            move |id, cx: &mut App| {
                entity.update(cx, |this, cx| {
                    this.set_listen(id);
                    cx.notify();
                });
            }
        };

        let mut tabs_row = div()
            .flex()
            .gap_1()
            .p_1()
            .rounded_full()
            .bg(rgb(theme::BG_SUNKEN))
            .border_1()
            .border_color(rgb(theme::BORDER));
        for (id, label, t) in TABS {
            let on_tab = on_tab.clone();
            tabs_row = tabs_row.child(pill_tab(id, label, tab == t, move |_, cx| on_tab(t, cx)));
        }

        div()
            .id("settings")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(theme::BG_RAISED))
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER))
                    .bg(rgb(theme::BG))
                    .child(tabs_row),
            )
            .child({
                let view = entity.entity_id();
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .child(
                        div()
                            .id("settings-body")
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .on_scroll_wheel(move |_, _, cx| cx.notify(view))
                            .flex()
                            .flex_col()
                            .items_center()
                            .p_4()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(600.))
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .when(tab == SettingsTab::General, |d| {
                                        d.child(general_page(state.clone(), &prefs))
                                    })
                                    .when(tab == SettingsTab::Formatting, |d| {
                                        d.child(formatting_page(state.clone(), &prefs))
                                    })
                                    .when(tab == SettingsTab::Shortcuts, |d| {
                                        d.child(shortcuts_page(
                                            state.clone(),
                                            &prefs,
                                            listen,
                                            on_listen.clone(),
                                        ))
                                    })
                                    .when(tab == SettingsTab::System, |d| {
                                        d.child(system_page(state.clone(), &self.sys_snap))
                                    }),
                            ),
                    )
                    .child(div().relative().w(px(12.)).h_full().flex_shrink_0().child(
                        overlay_scrollbar(
                            "settings-y-scroll",
                            ScrollAxis::Vertical,
                            &self.scroll,
                            &self.thumb,
                            true,
                            ScrollbarTone::Default,
                        ),
                    ))
            })
    }
}

fn picker(width: f32, items: impl IntoIterator<Item = AnyElement>) -> impl IntoElement {
    div().w(px(width)).flex_shrink_0().child(segmented(items))
}

fn bool_row(
    state: &Entity<AppState>,
    id: &'static str,
    title: &'static str,
    hint: &'static str,
    value: bool,
    set: fn(&mut Prefs, bool),
) -> AnyElement {
    let state = state.clone();
    setting_row(
        title,
        hint,
        switch(id, value, move |_, cx| {
            state.update(cx, |s, cx| {
                s.update_prefs(cx, |p| set(p, !value));
            });
        }),
    )
    .into_any_element()
}

fn patch_prefs(state: &Entity<AppState>, cx: &mut App, f: impl FnOnce(&mut Prefs)) {
    state.update(cx, |s, cx| s.update_prefs(cx, f));
}
