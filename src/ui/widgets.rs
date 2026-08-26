use std::sync::Arc;
use std::sync::OnceLock;

use gpui::{div, img, prelude::*, px, rgb, App, Image, ImageFormat, SharedString, Window};

use super::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Snip,
    Upload,
    Draw,
    Delete,
    Settings,
}

fn icon_svg(kind: IconKind) -> &'static [u8] {
    match kind {
        IconKind::Snip => {
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" fill="none" stroke="#1a1a1a" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" viewBox="0 0 24 24"><path stroke-dasharray="3 2.4" d="M7 4h10a3 3 0 0 1 3 3v10a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3z"/></svg>"##
        }
        IconKind::Upload => {
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" fill="none" stroke="#1a1a1a" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" viewBox="0 0 24 24"><rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 15 5-5 4 4 3-3 6 6"/><path d="M16 5v5M16 5l-2.2 2.2M16 5l2.2 2.2"/></svg>"##
        }
        IconKind::Draw => {
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" fill="none" stroke="#1a1a1a" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" viewBox="0 0 24 24"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4Z"/></svg>"##
        }
        IconKind::Delete => {
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" fill="none" stroke="#1a1a1a" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" viewBox="0 0 24 24"><path d="M4 7h16"/><path d="M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/><path d="M7 7l1 12a2 2 0 0 0 2 2h4a2 2 0 0 0 2-2l1-12"/></svg>"##
        }
        IconKind::Settings => {
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" fill="none" stroke="#1a1a1a" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.2a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.2a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.2a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9c.3.6.9 1 1.5 1H21a2 2 0 1 1 0 4h-.2a1.7 1.7 0 0 0-1.5 1Z"/></svg>"##
        }
    }
}

pub fn app_icon_image() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();
    ICON.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            crate::icon::APP_ICON_SVG.to_vec(),
        ))
    })
    .clone()
}

fn icon_image(kind: IconKind) -> Arc<Image> {
    static CACHE: OnceLock<[Arc<Image>; 5]> = OnceLock::new();
    let all = CACHE.get_or_init(|| {
        [
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                icon_svg(IconKind::Snip).to_vec(),
            )),
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                icon_svg(IconKind::Upload).to_vec(),
            )),
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                icon_svg(IconKind::Draw).to_vec(),
            )),
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                icon_svg(IconKind::Delete).to_vec(),
            )),
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                icon_svg(IconKind::Settings).to_vec(),
            )),
        ]
    });
    all[kind as usize].clone()
}

pub fn icon_btn(
    id: impl Into<SharedString>,
    kind: IconKind,
    active: bool,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    on_hover: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    div()
        .id(id)
        .size(px(32.))
        .rounded_md()
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |d| d.cursor_pointer())
        .when(!enabled, |d| d.opacity(0.38))
        .when(active, |d| d.bg(theme::accent_soft()))
        .when(enabled && !active, |d| {
            d.hover(|d| d.bg(theme::row_hover()))
        })
        .when(enabled, |d| {
            d.on_click(move |_, window, cx| on_click(window, cx))
        })
        .on_hover(move |hovered, window, cx| on_hover(*hovered, window, cx))
        .child(
            img(icon_image(kind))
                .size(px(18.))
                .object_fit(gpui::ObjectFit::Contain),
        )
}

pub fn checkbox(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    checked: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .py_1()
        .cursor_pointer()
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(
            div()
                .size(px(16.))
                .rounded_sm()
                .border_1()
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |d| {
                    d.bg(rgb(theme::ACCENT))
                        .border_color(rgb(theme::ACCENT))
                        .text_color(rgb(theme::ON_ACCENT))
                        .text_xs()
                        .child("✓")
                })
                .when(!checked, |d| {
                    d.bg(rgb(theme::BG_RAISED)).border_color(rgb(theme::BORDER))
                }),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(theme::TEXT))
                .child(label.into()),
        )
}

pub fn btn(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    filled: bool,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    let label = label.into();
    div()
        .id(id)
        .px_3()
        .h(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .text_sm()
        .when(enabled, |d| d.cursor_pointer())
        .when(!enabled, |d| d.opacity(0.45))
        .when(filled && enabled, |d| {
            d.bg(rgb(theme::ACCENT)).text_color(rgb(theme::ON_ACCENT))
        })
        .when(!filled, |d| {
            d.bg(rgb(theme::BG_RAISED))
                .text_color(rgb(theme::TEXT))
                .border_1()
                .border_color(rgb(theme::BORDER))
        })
        .when(enabled, |d| d.hover(|d| d.opacity(0.88)))
        .when(enabled, |d| {
            d.on_click(move |_, window, cx| on_click(window, cx))
        })
        .child(label)
}

pub fn ghost_btn(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    let label = label.into();
    div()
        .id(id)
        .px_2()
        .h(px(24.))
        .flex()
        .items_center()
        .rounded_sm()
        .text_xs()
        .text_color(rgb(theme::MUTED))
        .when(enabled, |d| {
            d.cursor_pointer().hover(|d| d.bg(theme::row_hover()))
        })
        .when(!enabled, |d| d.opacity(0.45))
        .when(enabled, |d| {
            d.on_click(move |_, window, cx| on_click(window, cx))
        })
        .child(label)
}

pub fn kbd_chip(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px_2()
        .h(px(22.))
        .flex()
        .items_center()
        .rounded_sm()
        .bg(rgb(theme::BG_SUNKEN))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .text_xs()
        .font_family("monospace")
        .text_color(rgb(theme::MUTED))
        .child(label.into())
}

pub fn segmented(
    left_id: impl Into<SharedString>,
    left_label: impl Into<SharedString>,
    left_active: bool,
    left_click: impl Fn(&mut Window, &mut App) + 'static,
    right_id: impl Into<SharedString>,
    right_label: impl Into<SharedString>,
    right_active: bool,
    right_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .p_1()
        .rounded_md()
        .bg(rgb(theme::BG_SUNKEN))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .child(seg_item(left_id, left_label, left_active, left_click))
        .child(seg_item(right_id, right_label, right_active, right_click))
}

pub fn segmented3(
    a_id: impl Into<SharedString>,
    a_label: impl Into<SharedString>,
    a_active: bool,
    a_click: impl Fn(&mut Window, &mut App) + 'static,
    b_id: impl Into<SharedString>,
    b_label: impl Into<SharedString>,
    b_active: bool,
    b_click: impl Fn(&mut Window, &mut App) + 'static,
    c_id: impl Into<SharedString>,
    c_label: impl Into<SharedString>,
    c_active: bool,
    c_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .p_1()
        .rounded_md()
        .bg(rgb(theme::BG_SUNKEN))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .child(seg_item(a_id, a_label, a_active, a_click))
        .child(seg_item(b_id, b_label, b_active, b_click))
        .child(seg_item(c_id, c_label, c_active, c_click))
}

fn seg_item(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id.into())
        .px_2()
        .h(px(24.))
        .flex()
        .items_center()
        .rounded_sm()
        .text_xs()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(rgb(theme::BG_RAISED)).text_color(rgb(theme::TEXT))
        })
        .when(!active, |d| {
            d.text_color(rgb(theme::MUTED))
                .hover(|d| d.text_color(rgb(theme::TEXT)))
        })
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(label.into())
}

pub fn status_dot(kind: theme::StatusKind) -> impl IntoElement {
    div()
        .size(px(7.))
        .rounded_full()
        .bg(theme::status_color(kind))
}

pub fn section_label(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(rgb(theme::MUTED))
        .child(text.into())
}
