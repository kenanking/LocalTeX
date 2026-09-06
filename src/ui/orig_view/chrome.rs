use gpui::{Context, SharedString, div, prelude::*, px, rgb, rgba, svg};
use uuid::Uuid;

use super::super::main_window::MainWindow;
use super::super::theme;
use super::super::widgets::{ButtonElement, IconKind, Tooltip};

pub(crate) fn hud_pill() -> gpui::Div {
    div()
        .h(px(26.))
        .px(px(10.))
        .rounded_full()
        .flex()
        .items_center()
        .bg(theme::hud_pill())
        .border_1()
        .border_color(rgba(0xffffff14))
        .text_color(rgb(0xececef))
        .text_xs()
        .whitespace_nowrap()
}

pub(crate) fn orig_hud_disc(
    id: impl Into<SharedString>,
    icon: IconKind,
    tooltip: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl gpui::IntoElement {
    let id = id.into();
    div()
        .id(id.clone())
        .accessibility_id(id)
        .role(gpui::Role::Button)
        .aria_label(tooltip)
        .focusable()
        .tab_stop(true)
        .size(px(28.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::hud_pill())
        .border_1()
        .border_color(rgba(0xffffff14))
        .cursor_pointer()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(Tooltip::text(tooltip))
        .on_click(on_click)
        .hover(|d| d.bg(theme::hud_pill_hover()))
        .child(
            svg()
                .path(icon.asset_path())
                .size(px(14.))
                .text_color(rgb(0xffffff)),
        )
}

/// Compact light disc for the source-editor toolbar on paper, not the overlay HUD.
pub(crate) fn source_hud_disc(
    id: impl Into<SharedString>,
    icon: IconKind,
    tooltip: &'static str,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl gpui::IntoElement {
    let id = id.into();
    let element = div()
        .id(id.clone())
        .accessibility_id(id)
        .role(gpui::Role::Button)
        .aria_label(tooltip)
        .when(enabled, |d| d.focusable().tab_stop(true))
        .size(px(20.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(Tooltip::text(tooltip))
        .when(enabled, |d| d.on_click(on_click))
        .hover(|d| d.bg(rgb(theme::ACCENT_SOFT_FILL)))
        .child(
            svg()
                .path(icon.asset_path())
                .size(px(12.))
                .text_color(rgb(theme::TEXT)),
        );
    ButtonElement { element, enabled }
}

/// Outlined 24px disc, sits outside the source HUD pill (Done).
pub(crate) fn source_done_disc(
    id: impl Into<SharedString>,
    icon: IconKind,
    tooltip: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl gpui::IntoElement {
    let id = id.into();
    div()
        .id(id.clone())
        .accessibility_id(id)
        .role(gpui::Role::Button)
        .aria_label(tooltip)
        .focusable()
        .tab_stop(true)
        .size(px(24.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(theme::BG_RAISED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .cursor_pointer()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(Tooltip::text(tooltip))
        .on_click(on_click)
        .hover(|d| {
            d.bg(rgb(theme::ACCENT_SOFT_FILL))
                .border_color(rgb(theme::ACCENT_BORDER))
        })
        .child(
            svg()
                .path(icon.asset_path())
                .size(px(13.))
                .text_color(rgb(theme::TEXT)),
        )
}

pub(crate) fn source_hud_bar() -> gpui::Div {
    div()
        .h(px(24.))
        .px(px(4.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(0.))
        .bg(rgb(theme::BG_RAISED))
        .border_1()
        .border_color(rgb(theme::BORDER))
}

pub(crate) fn source_hud_sep() -> gpui::Div {
    div().w(px(1.)).h(px(10.)).mx(px(2.)).bg(rgb(theme::BORDER))
}

pub(crate) fn orig_action_capsule(
    doc_id: Uuid,
    copy_flashed: bool,
    reveal_enabled: bool,
    id_prefix: &'static str,
    cx: &mut Context<MainWindow>,
) -> impl gpui::IntoElement + use<> {
    let entity = cx.entity();
    div()
        .id(SharedString::from(format!("{id_prefix}-capsule")))
        .h(px(28.))
        .w(px(86.))
        .px(px(3.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(0.))
        .bg(theme::hud_pill())
        .border_1()
        .border_color(rgba(0xffffff47))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(orig_capsule_btn(
            SharedString::from(format!("{id_prefix}-copy")),
            if copy_flashed {
                IconKind::Check
            } else {
                IconKind::Copy
            },
            "Copy image",
            true,
            {
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.state.update(cx, |s, cx| s.copy_original(doc_id, cx));
                    });
                }
            },
        ))
        .child(orig_capsule_sep())
        .child(orig_capsule_btn(
            SharedString::from(format!("{id_prefix}-save")),
            IconKind::Save,
            "Save as",
            true,
            {
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.state
                            .update(cx, |s, cx| s.save_original_as(doc_id, cx));
                    });
                }
            },
        ))
        .child(orig_capsule_sep())
        .child(orig_capsule_btn(
            SharedString::from(format!("{id_prefix}-reveal")),
            IconKind::Folder,
            if reveal_enabled {
                "Show in folder"
            } else {
                "Image isn't saved yet"
            },
            reveal_enabled,
            {
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.state.update(cx, |s, cx| s.reveal_original(doc_id, cx));
                    });
                }
            },
        ))
}

fn orig_capsule_sep() -> gpui::Div {
    div().w(px(1.)).h(px(12.)).bg(rgba(0xffffff38))
}

fn orig_capsule_btn(
    id: SharedString,
    icon: IconKind,
    label: &'static str,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl gpui::IntoElement {
    div()
        .id(id)
        .w(px(26.))
        .h(px(22.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(Tooltip::text(label))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|d| d.bg(rgba(0xffffff24)))
                .on_click(on_click)
        })
        .when(!enabled, |d| d.opacity(0.38))
        .child(
            svg()
                .path(icon.asset_path())
                .size(px(14.))
                .text_color(rgb(0xffffff)),
        )
}
