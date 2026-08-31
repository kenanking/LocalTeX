use gpui::{div, prelude::*, Entity};

use super::super::widgets::settings_group;
use super::bool_row;
use crate::prefs::{Prefs, WindowCloseAction};
use crate::state::AppState;

pub(super) fn general_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            "Capture",
            vec![
                bool_row(
                    &state,
                    "pref-hide",
                    "Hide window while snipping",
                    "Restore when recognition finishes.",
                    prefs.hide_on_capture,
                    |p, v| p.hide_on_capture = v,
                ),
                bool_row(
                    &state,
                    "pref-orig",
                    "Show original after recognize",
                    "Snip image above the recognized document.",
                    prefs.show_original,
                    |p, v| p.show_original = v,
                ),
                bool_row(
                    &state,
                    "pref-copy",
                    "Copy result automatically",
                    "Copies the last text format you used for this kind of snip, or the primary format.",
                    prefs.autocopy,
                    |p, v| p.autocopy = v,
                ),
            ],
        ))
        .child(settings_group(
            "Window",
            vec![bool_row(
                &state,
                "pref-close-min",
                "Minimize on close",
                "Keeps the tray and Ctrl+Shift+S. Off quits; Ctrl+Q always quits.",
                prefs.close_action == WindowCloseAction::Minimize,
                |p, v| {
                    p.close_action = if v {
                        WindowCloseAction::Minimize
                    } else {
                        WindowCloseAction::Quit
                    };
                },
            )],
        ))
}
