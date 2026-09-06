use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Context, CursorStyle, MouseButton, MouseDownEvent, ScrollHandle, SharedString,
    UniformListScrollHandle, Window, div, img, prelude::*, px, rgb, uniform_list,
};
use uuid::Uuid;

use super::main_window::MainWindow;
use super::scroll::{
    ScrollAxis, ScrollThumbDrag, ScrollbarTone, overlay_chrome_hovered, overlay_pointer_in_pane,
    overlay_scrollbar,
};
use super::theme;
use super::widgets::{
    IconBtnSize, IconKind, icon_btn_sized, missing_image_slot, section_label, seg_item, segmented,
};
use super::window_drag::WindowDrag;
use crate::cache::{ROW_HEIGHT_PX, THUMB_VIEWPORT_MULT};
use crate::doc::ImageSlot;
use crate::library::DATE_PRESETS;

pub(crate) const SIDEBAR_MIN: f32 = 208.0;
pub(crate) const SIDEBAR_MAX: f32 = 420.0;
pub(crate) const SIDEBAR_RAIL: f32 = 52.0;
pub(crate) const SIDEBAR_AUTO_COLLAPSE: f32 = 720.0;
const SIDEBAR_HYSTERESIS: f32 = 24.0;
pub(crate) const SIDEBAR_AUTO_EXPAND: f32 = SIDEBAR_AUTO_COLLAPSE + SIDEBAR_HYSTERESIS;

const COLLAPSED_ROW_H: f32 = 48.0;
const COLLAPSED_THUMB_W: f32 = 40.0;
const COLLAPSED_THUMB_H: f32 = 28.0;

#[derive(Clone, Copy)]
pub(crate) struct SidebarFold {
    pub collapsed: bool,
    last_w: f32,
}

impl SidebarFold {
    pub fn seed(win_w: f32, pinned: bool) -> Self {
        Self {
            collapsed: pinned || win_w < SIDEBAR_AUTO_COLLAPSE,
            last_w: win_w,
        }
    }

    pub fn on_resize(&mut self, now_w: f32, pinned: bool) -> bool {
        let next =
            if pinned || (self.last_w >= SIDEBAR_AUTO_COLLAPSE && now_w < SIDEBAR_AUTO_COLLAPSE) {
                true
            } else if self.last_w < SIDEBAR_AUTO_EXPAND && now_w >= SIDEBAR_AUTO_EXPAND {
                false
            } else {
                self.collapsed
            };
        self.last_w = now_w;
        if next == self.collapsed {
            return false;
        }
        self.collapsed = next;
        true
    }

    pub fn toggle_pin(&mut self, win_w: f32) -> bool {
        if self.collapsed {
            self.collapsed = false;
            self.last_w = win_w;
            false
        } else {
            self.collapsed = true;
            true
        }
    }

    pub fn width(self, has_docs: bool, expanded: f32) -> f32 {
        if has_docs && self.collapsed {
            SIDEBAR_RAIL
        } else {
            expanded.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
        }
    }

    #[cfg(test)]
    fn at(last_w: f32, collapsed: bool) -> Self {
        Self { collapsed, last_w }
    }
}

pub(crate) struct HistoryPane {
    pub fold: SidebarFold,
    pub scroll: UniformListScrollHandle,
    pub thumb: Rc<RefCell<Option<ScrollThumbDrag>>>,
    pub hover: bool,
}

impl HistoryPane {
    pub fn new(win_w: f32, pinned: bool) -> Self {
        Self {
            fold: SidebarFold::seed(win_w, pinned),
            scroll: UniformListScrollHandle::new(),
            thumb: Rc::new(RefCell::new(None)),
            hover: false,
        }
    }

    pub fn scroll_base(&self) -> ScrollHandle {
        self.scroll.0.borrow().base_handle.clone()
    }
}

impl MainWindow {
    pub(crate) fn render_history(
        &self,
        n_docs: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let collapsed = self.history.fold.collapsed;
        let (date_preset, width, ids, selected) = {
            let state = self.state.read(cx);
            let width = self.history.fold.width(true, state.prefs.sidebar_width);
            (
                state.date_preset(),
                width,
                state.visible_snapshot(),
                state.selected(),
            )
        };
        let mut list = div()
            .id("history")
            .role(gpui::Role::ListBox)
            .aria_label("Snip history")
            .key_context("SnipList")
            .track_focus(&self.snip_list_focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.snip_list_focus, cx);
                }),
            )
            .relative()
            .w(px(width))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BG))
            .border_r_1()
            .border_color(rgb(theme::BORDER));

        list = list.child(self.render_history_header(n_docs, collapsed, cx));

        if !collapsed {
            list = list.child(div().px_2().pb_1().child(self.search.clone()));

            let state_ent = self.state.clone();
            list = list.child(div().px_2().pb_2().child(segmented(DATE_PRESETS.map(
                |(id, label, preset)| {
                    let state = state_ent.clone();
                    seg_item(id, label, date_preset == preset, move |_, cx| {
                        state.update(cx, |s, cx| s.set_date_preset(preset, cx));
                    })
                },
            ))));
        }

        let has_rows = !ids.is_empty();
        let rows = if ids.is_empty() {
            div()
                .id("history-rows")
                .flex_1()
                .min_h_0()
                .when(!collapsed, |d| {
                    d.px_2()
                        .pt_4()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child("No snips in this range")
                })
                .into_any_element()
        } else {
            let media = self.media.cache.clone();
            let state_ent = self.state.clone();
            let list_focus = self.snip_list_focus.clone();
            let thumb_keep = self.media.thumb_keep.clone();
            let window_ent = cx.entity();
            let scroll = self.history.scroll.clone();
            uniform_list(
                "history-rows",
                ids.len(),
                move |range: Range<usize>, _window: &mut Window, cx: &mut App| {
                    let vis = (range.end - range.start).max(1);
                    let pad = vis.saturating_mul(THUMB_VIEWPORT_MULT.saturating_sub(1)) / 2;
                    let keep_start = range.start.saturating_sub(pad);
                    let keep_end = (range.end + pad).min(ids.len());
                    *thumb_keep.borrow_mut() = ids[keep_start..keep_end].to_vec();
                    let entity = window_ent.clone();
                    cx.defer(move |cx| {
                        entity.update(cx, |this, cx| this.schedule_media_gc(cx));
                    });
                    let mut need_thumbs = Vec::new();
                    {
                        let state = state_ent.read(cx);
                        for &id in &ids[keep_start..keep_end] {
                            let Some(doc) = state.doc(id) else {
                                continue;
                            };
                            if media.borrow().thumb(id).is_some() {
                                continue;
                            }
                            if !doc.thumb_jpeg.is_empty()
                                || doc.image.pixels().is_some()
                                || !matches!(doc.image, crate::doc::ImageSlot::Missing)
                            {
                                need_thumbs.push(id);
                            }
                        }
                    }
                    if !need_thumbs.is_empty() {
                        let entity = window_ent.clone();
                        cx.defer(move |cx| {
                            entity.update(cx, |this, cx| this.ensure_thumbs(&need_thumbs, cx));
                        });
                    }
                    let state = state_ent.read(cx);
                    range
                        .map(|idx| {
                            let id = ids[idx];
                            history_row(
                                id,
                                selected == Some(id),
                                collapsed,
                                state,
                                media.borrow().thumb(id),
                                list_focus.clone(),
                                state_ent.clone(),
                            )
                        })
                        .collect()
                },
            )
            .track_scroll(&scroll)
            .h_full()
            .into_any_element()
        };

        let view = cx.entity_id();
        let thumb_visible = self.history.hover
            || self
                .history
                .thumb
                .borrow()
                .as_ref()
                .is_some_and(|d| d.vertical);
        let scroll_base = self.history.scroll_base();
        let mut wrap = div()
            .id("history-rows-wrap")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .px(px(if collapsed { 6. } else { 8. }))
            .pb(px(if collapsed { 4. } else { 8. }))
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                let next = overlay_chrome_hovered(
                    *hovered,
                    overlay_pointer_in_pane(
                        &this.history.scroll_base(),
                        ScrollAxis::Vertical,
                        window.mouse_position(),
                    ),
                );
                if this.history.hover != next {
                    this.history.hover = next;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(move |_, _, cx| {
                cx.notify(view);
            })
            .child(rows);

        if has_rows {
            wrap = wrap.child(overlay_scrollbar(
                "history-scroll-thumb",
                ScrollAxis::Vertical,
                &scroll_base,
                &self.history.thumb,
                thumb_visible,
                ScrollbarTone::Subtle,
            ));
        }

        list = list.child(wrap);

        if !collapsed {
            // Drag handle on the border; moves land on the window root (main.rs
            // render) so the drag survives leaving this 8px strip.
            list = list.child(
                div()
                    .id("sidebar-resize")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-4.))
                    .w(px(8.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.window_drag = Some(WindowDrag::Sidebar {
                                start_x: f32::from(ev.position.x),
                                start_w: width,
                            });
                            cx.stop_propagation();
                        }),
                    ),
            );
        }

        list
    }

    fn render_history_header(
        &self,
        n_docs: usize,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let entity = cx.entity();
        let chevron = icon_btn_sized(
            "sidebar-toggle",
            if collapsed {
                IconKind::Expand
            } else {
                IconKind::Collapse
            },
            if collapsed {
                "Expand snips"
            } else {
                "Collapse sidebar"
            },
            false,
            true,
            IconBtnSize {
                hit: px(22.),
                glyph: px(14.),
                kbd: None,
            },
            move |window, cx| {
                entity.update(cx, |this, cx| this.toggle_sidebar(window, cx));
            },
        );

        div()
            .flex()
            .items_center()
            .h(px(36.))
            .when(collapsed, |d| d.justify_center())
            .when(!collapsed, |d| d.px_3().gap_1())
            .when(!collapsed, |d| {
                d.child(section_label("Snips")).child(div().flex_1()).child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .child(SharedString::from(n_docs.to_string())),
                )
            })
            .child(chevron)
    }
}

fn history_row(
    id: Uuid,
    selected: bool,
    collapsed: bool,
    state: &crate::state::AppState,
    thumb: Option<std::sync::Arc<gpui::RenderImage>>,
    list_focus: gpui::FocusHandle,
    state_ent: gpui::Entity<crate::state::AppState>,
) -> gpui::AnyElement {
    let doc = state.doc(id);
    let missing = doc.is_some_and(|d| matches!(d.image, ImageSlot::Missing)) && thumb.is_none();
    let title = doc.map(|d| d.first_line()).unwrap_or_default();
    let age = doc.map(|d| d.age_label()).unwrap_or_default();
    let (row_h, thumb_w, thumb_h) = if collapsed {
        (COLLAPSED_ROW_H, COLLAPSED_THUMB_W, COLLAPSED_THUMB_H)
    } else {
        (ROW_HEIGHT_PX, 56.0, 40.0)
    };

    div()
        .id(SharedString::from(id.to_string()))
        .role(gpui::Role::ListBoxOption)
        .aria_label(title.clone())
        .aria_selected(selected)
        .when(selected, |d| d.aria_active_descendant())
        .w_full()
        .h(px(row_h))
        .overflow_hidden()
        .flex()
        .items_center()
        .when(!collapsed, |d| d.gap_2().px_2())
        .when(collapsed, |d| d.justify_center())
        .rounded_md()
        .cursor_pointer()
        .when(selected, |d| d.bg(theme::accent_soft()))
        .when(!selected, |d| d.hover(|d| d.bg(theme::row_hover())))
        .on_a11y_action(gpui::AccessibleAction::Click, {
            let state_ent = state_ent.clone();
            let list_focus = list_focus.clone();
            move |_, window, cx| {
                window.focus(&list_focus, cx);
                state_ent.update(cx, |s, cx| s.select(id, cx));
            }
        })
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            window.focus(&list_focus, cx);
            state_ent.update(cx, |s, cx| s.select(id, cx));
        })
        .child(if missing {
            missing_image_slot(px(thumb_w), px(thumb_h))
        } else {
            div()
                .w(px(thumb_w))
                .h(px(thumb_h))
                .rounded_sm()
                .bg(rgb(theme::BG_SUNKEN))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .overflow_hidden()
                .when_some(thumb, |d, img_data| {
                    d.child(
                        img(img_data)
                            .w(px(thumb_w))
                            .h(px(thumb_h))
                            .object_fit(gpui::ObjectFit::Cover),
                    )
                })
        })
        .when(!collapsed, |d| {
            d.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(rgb(theme::TEXT))
                            .child(SharedString::from(title)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::MUTED))
                            .child(SharedString::from(age)),
                    ),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_matches_scheme_a() {
        assert!(
            !SidebarFold::seed(800.0, false).collapsed,
            "seed at 800 unpinned = expanded"
        );
        assert!(
            SidebarFold::seed(700.0, false).collapsed,
            "seed at 700 unpinned = collapsed"
        );
        assert!(
            SidebarFold::seed(800.0, true).collapsed,
            "seed pinned always collapsed"
        );
        assert!(SidebarFold::seed(700.0, true).collapsed);
    }

    #[test]
    fn resize_hysteresis() {
        let mut f = SidebarFold::at(800.0, false);
        assert!(f.on_resize(700.0, false), "800→700 unpinned collapses");
        assert!(f.collapsed);

        let mut f = SidebarFold::at(700.0, true);
        assert!(
            f.on_resize(800.0, false),
            "700→800 unpinned expands (crosses 744)"
        );
        assert!(!f.collapsed);

        let mut f = SidebarFold::at(700.0, true);
        assert!(
            !f.on_resize(743.0, false),
            "700→743 stays collapsed (must pass 744)"
        );
        assert!(f.collapsed);
        assert!(f.on_resize(744.0, false));
        assert!(!f.collapsed);

        let mut f = SidebarFold::at(730.0, true);
        assert!(
            !f.on_resize(710.0, false),
            "730→710 already collapsed stays collapsed"
        );
        assert!(f.collapsed);

        let mut f = SidebarFold::at(730.0, false);
        assert!(
            f.on_resize(710.0, false),
            "730→710 already expanded folds across 720, no bounce expand"
        );
        assert!(f.collapsed);

        let mut f = SidebarFold::at(800.0, false);
        assert!(f.on_resize(700.0, true), "pinned ignores resize");
        assert!(f.collapsed);
        assert!(!f.on_resize(800.0, true));
        assert!(f.collapsed);

        let mut f = SidebarFold::at(700.0, false);
        assert!(
            !f.on_resize(700.0, false),
            "manual expand while narrow: same-size resize keeps expanded"
        );
        assert!(!f.collapsed);
    }

    #[test]
    fn toggle_expand_while_narrow_does_not_snap_shut() {
        let mut f = SidebarFold::seed(700.0, false);
        assert!(f.collapsed);
        assert!(!f.toggle_pin(700.0));
        assert!(!f.collapsed);
        assert!(!f.on_resize(700.0, false));
        assert!(!f.collapsed);
    }

    #[test]
    fn width_uses_rail_only_when_docs_and_collapsed() {
        let expanded = SidebarFold::seed(800.0, false);
        assert_eq!(expanded.width(true, 232.0), 232.0);
        assert_eq!(expanded.width(false, 232.0), 232.0);
        assert_eq!(expanded.width(false, 500.0), SIDEBAR_MAX);

        let collapsed = SidebarFold::seed(700.0, false);
        assert_eq!(collapsed.width(true, 232.0), SIDEBAR_RAIL);
        assert_eq!(collapsed.width(false, 232.0), 232.0);
    }
}
