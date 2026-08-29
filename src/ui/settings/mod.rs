use std::cell::RefCell;
use std::rc::Rc;

use gpui::{div, prelude::*, px, rgb, AnyElement, App, Entity, EntityId, ScrollHandle};

use super::scroll::{overlay_scrollbar, ScrollAxis, ScrollThumbDrag, ScrollbarTone};
use super::theme;
use super::widgets::{pill_tab, segmented, setting_row, switch};
use crate::keymap::ShortcutId;
use crate::prefs::Prefs;
use crate::state::AppState;
use crate::sysmon::SysSnapshot;

mod formatting;
mod general;
mod shortcuts;
mod system;

use formatting::formatting_page;
use general::general_page;
use shortcuts::shortcuts_page;
use system::system_page;

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

pub struct SettingsScroll<'a> {
    pub handle: &'a ScrollHandle,
    pub thumb: &'a Rc<RefCell<Option<ScrollThumbDrag>>>,
    pub view: EntityId,
}

pub fn page(
    state: Entity<AppState>,
    tab: SettingsTab,
    snap: &SysSnapshot,
    listen: Option<ShortcutId>,
    on_tab: impl Fn(SettingsTab, &mut App) + Clone + 'static,
    on_listen: impl Fn(Option<ShortcutId>, &mut App) + Clone + 'static,
    scroll: SettingsScroll<'_>,
    cx: &App,
) -> impl IntoElement {
    let prefs = state.read(cx).prefs.clone();

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
            let view = scroll.view;
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
                        .track_scroll(scroll.handle)
                        .on_scroll_wheel(move |_, _, cx| cx.notify(view))
                        .flex()
                        .flex_col()
                        .items_center()
                        .p_4()
                        .child(
                            // Content column caps out so meter bars stay scannable
                            // instead of stretching across a wide window.
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
                                .when(tab == SettingsTab::System, |d| d.child(system_page(snap))),
                        ),
                )
                .child(div().relative().w(px(12.)).h_full().flex_shrink_0().child(
                    overlay_scrollbar(
                        "settings-y-scroll",
                        ScrollAxis::Vertical,
                        scroll.handle,
                        scroll.thumb,
                        true,
                        ScrollbarTone::Default,
                    ),
                ))
        })
}

/// Segmented control pinned to a fixed width (row-right picker).
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
            state.update(cx, |s, cx| s.update_prefs(cx, |p| set(p, !value)));
        }),
    )
    .into_any_element()
}

fn patch_prefs(state: &Entity<AppState>, cx: &mut App, f: impl FnOnce(&mut Prefs)) {
    state.update(cx, |s, cx| s.update_prefs(cx, f));
}
