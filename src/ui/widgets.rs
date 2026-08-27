use gpui::{
    div, img, prelude::*, px, rgb, AnyElement, AnyView, App, Pixels, ScrollHandle, SharedString,
    Window,
};

use super::theme;

/// Hover card for icon chrome. GPUI already owns delay / placement /
/// dismissal via `.tooltip()`; this is only the view it asks for.
/// Adapted from Waku's tooltip surface (no `shadow_md` on gpui 0.2).
pub struct Tooltip {
    label: SharedString,
}

impl Tooltip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }

    pub fn build(self, _window: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| self).into()
    }

    pub fn text(
        label: impl Into<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let label = label.into();
        move |window, cx| Tooltip::new(label.clone()).build(window, cx)
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().pt(px(4.)).pl(px(2.)).child(
            div()
                .px(px(7.))
                .py(px(4.))
                .rounded_md()
                .border_1()
                .border_color(rgb(theme::BORDER))
                .bg(rgb(theme::BG_RAISED))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(self.label.clone()),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Snip,
    Upload,
    Draw,
    Delete,
    Settings,
    Copy,
    Zoom,
}

impl IconKind {
    pub fn asset_path(self) -> &'static str {
        match self {
            Self::Snip => "icons/snip.svg",
            Self::Upload => "icons/upload.svg",
            Self::Draw => "icons/draw.svg",
            Self::Delete => "icons/delete.svg",
            Self::Settings => "icons/settings.svg",
            Self::Copy => "icons/copy.svg",
            Self::Zoom => "icons/zoom.svg",
        }
    }

    #[cfg(test)]
    const ALL: [Self; 7] = [
        Self::Snip,
        Self::Upload,
        Self::Draw,
        Self::Delete,
        Self::Settings,
        Self::Copy,
        Self::Zoom,
    ];
}

pub fn icon_btn(
    id: impl Into<SharedString>,
    kind: IconKind,
    hint: impl Into<SharedString>,
    active: bool,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    let hint = hint.into();
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
        .tooltip(Tooltip::text(hint))
        .child(
            // `img()` + AssetSource. GPUI rasters SVG at 2× the file's
            // width/height; the 96px sources stay sharp when downscaled
            // into this 18px slot. Do not switch to `svg().path` on this
            // NVIDIA/Vulkan host until a filled rect actually paints.
            img(kind.asset_path())
                .size(px(18.))
                .flex_shrink_0()
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
        .items_start()
        .gap_2()
        .py_1()
        .min_w_0()
        .cursor_pointer()
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(
            div()
                .size(px(16.))
                .mt(px(2.))
                .flex_shrink_0()
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
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(rgb(theme::TEXT))
                .whitespace_normal()
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

pub fn segmented(items: impl IntoIterator<Item = AnyElement>) -> impl IntoElement {
    let mut row = div()
        .flex()
        .w_full()
        .min_w_0()
        .p_1()
        .rounded_md()
        .bg(rgb(theme::BG_SUNKEN))
        .border_1()
        .border_color(rgb(theme::BORDER));
    for item in items {
        row = row.child(item);
    }
    row
}

pub fn missing_image_slot(width: Pixels, height: Pixels) -> gpui::Div {
    div()
        .w(width)
        .h(height)
        .rounded_sm()
        .bg(rgb(theme::BG_SUNKEN))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(rgb(theme::MUTED))
        .child("Image missing")
}

pub fn seg_item(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .id(id.into())
                .h(px(24.))
                .w_full()
                .px_2()
                .flex()
                .items_center()
                .justify_center()
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
                .child(label.into()),
        )
        .into_any()
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

pub fn settings_nav_item(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    // Outer `flex_1` has no `.id()`: gpui 0.2 does not grow a stateful
    // (id'd) node. Same wrapper as `seg_item`.
    div().flex_1().min_w_0().h(px(32.)).child(
        div()
            .id(id.into())
            .h(px(32.))
            .w_full()
            .px_3()
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_sm()
            .when(active, |d| {
                d.bg(theme::accent_soft())
                    .text_color(rgb(theme::ACCENT))
                    .font_weight(gpui::FontWeight::MEDIUM)
            })
            .when(!active, |d| {
                d.text_color(rgb(theme::TEXT))
                    .hover(|d| d.bg(theme::row_hover()))
            })
            .on_click(move |_, window, cx| on_click(window, cx))
            .child(label.into()),
    )
}

pub fn settings_card(
    title: impl Into<SharedString>,
    children: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .w_full()
        .min_w_0()
        .rounded_lg()
        .border_1()
        .border_color(rgb(theme::BORDER))
        .bg(rgb(theme::BG))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(theme::TEXT))
                .child(title.into()),
        )
        .child(children)
}

pub fn setting_row(
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    control: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .min_w_0()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(theme::TEXT))
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .whitespace_normal()
                        .child(hint.into()),
                ),
        )
        .child(div().w_full().min_w_0().child(control))
}

/// GPUI 0.2 `overflow_y_scroll` enables wheel scrolling but does not
/// paint a native thumb. Overlay this on a `relative` parent that
/// `track_scroll`s the same handle.
pub fn overlay_y_scrollbar(handle: &ScrollHandle) -> impl IntoElement {
    let max_y: f32 = handle.max_offset().height.into();
    let view_h: f32 = handle.bounds().size.height.into();
    let offset_y: f32 = handle.offset().y.into();
    let show = max_y > 1.0 && view_h > 1.0;
    let thumb_h = if show {
        (view_h * view_h / (view_h + max_y)).clamp(24.0, view_h)
    } else {
        0.0
    };
    let travel = (view_h - thumb_h).max(0.0);
    let top = if max_y > 0.0 {
        (-offset_y / max_y).clamp(0.0, 1.0) * travel
    } else {
        0.0
    };

    if !show {
        return div().into_any();
    }
    div()
        .id("y-scroll-thumb")
        .absolute()
        .top(px(top))
        .right(px(3.))
        .w(px(6.))
        .h(px(thumb_h))
        .rounded_full()
        .bg(theme::scrollbar_thumb())
        .into_any()
}

pub fn copy_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    preview: impl Into<SharedString>,
    copied: bool,
    enabled: bool,
    on_copy: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    let btn_id = SharedString::from(format!("{id}-btn"));
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .h(px(24.))
        .overflow_hidden()
        .pl_3()
        .pr_1()
        .rounded_md()
        .when(copied, |d| d.bg(theme::accent_soft()))
        .when(!copied, |d| d.bg(rgb(theme::BG_SUNKEN)))
        .child(
            div()
                .w(px(110.))
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(label.into()),
        )
        .child(
            // Outer `flex_1` has no `.id()`: gpui 0.2 does not shrink an
            // id'd node, so the preview would wrap to two lines.
            div().flex_1().min_w_0().overflow_hidden().child(
                div()
                    .id(id)
                    .w_full()
                    .overflow_hidden()
                    .font_family("monospace")
                    .text_sm()
                    .line_height(px(18.))
                    .truncate()
                    .text_color(rgb(theme::TEXT))
                    .child(preview.into()),
            ),
        )
        .child(
            div()
                .id(btn_id)
                .size(px(24.))
                .rounded_sm()
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .when(enabled, |d| d.cursor_pointer())
                .when(!enabled, |d| d.opacity(0.38))
                .when(copied, |d| d.bg(theme::accent_soft()))
                .when(enabled && !copied, |d| {
                    d.hover(|d| d.bg(theme::row_hover()))
                })
                .when(enabled, |d| {
                    d.on_click(move |_, window, cx| on_copy(window, cx))
                })
                .tooltip(Tooltip::text(if copied { "Copied" } else { "Copy" }))
                .child(
                    img(IconKind::Copy.asset_path())
                        .size(px(14.))
                        .flex_shrink_0()
                        .object_fit(gpui::ObjectFit::Contain),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::IconKind;
    use crate::icon::Assets;
    use gpui::AssetSource;

    #[test]
    fn every_toolbar_kind_has_an_embedded_asset() {
        for kind in IconKind::ALL {
            let path = kind.asset_path();
            assert!(
                Assets.load(path).expect("load").is_some(),
                "missing embedded icon: {path}"
            );
        }
    }
}
