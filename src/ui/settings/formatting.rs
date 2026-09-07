use gpui::{Entity, div, prelude::*};

use super::super::widgets::{seg_item, setting_row, settings_group};
use super::{patch_prefs, picker};
use crate::i18n::t;
use crate::prefs::{BlockDelim, InlineDelim, Prefs, ReadingWidth};
use crate::state::AppState;

pub(super) fn formatting_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            t("settings.math_delimiters"),
            vec![
                setting_row(
                    t("settings.inline_math"),
                    t("settings.inline_math_hint"),
                    picker(
                        150.,
                        [
                            seg_item(
                                "pref-inl-d",
                                "$ … $",
                                prefs.inline_delim == InlineDelim::Dollar,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.inline_delim = InlineDelim::Dollar
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-inl-p",
                                "\\( … \\)",
                                prefs.inline_delim == InlineDelim::Paren,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.inline_delim = InlineDelim::Paren
                                        })
                                    }
                                },
                            ),
                        ],
                    ),
                )
                .into_any_element(),
                setting_row(
                    t("settings.display_math"),
                    t("settings.display_math_hint"),
                    picker(
                        210.,
                        [
                            seg_item(
                                "pref-blk-d",
                                "$$",
                                prefs.block_delim == BlockDelim::Dollars,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.block_delim = BlockDelim::Dollars
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-blk-b",
                                "\\[ \\]",
                                prefs.block_delim == BlockDelim::Brackets,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.block_delim = BlockDelim::Brackets
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-blk-e",
                                "equation*",
                                prefs.block_delim == BlockDelim::Equation,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.block_delim = BlockDelim::Equation
                                        })
                                    }
                                },
                            ),
                        ],
                    ),
                )
                .into_any_element(),
            ],
        ))
        .child(settings_group(
            t("settings.preview"),
            vec![
                setting_row(
                    t("settings.reading_width"),
                    t("settings.reading_width_hint"),
                    picker(
                        210.,
                        [
                            seg_item(
                                "pref-read-n",
                                t("settings.width_narrow"),
                                prefs.reading_width == ReadingWidth::Narrow,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.reading_width = ReadingWidth::Narrow
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-read-m",
                                t("settings.width_medium"),
                                prefs.reading_width == ReadingWidth::Medium,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.reading_width = ReadingWidth::Medium
                                        })
                                    }
                                },
                            ),
                            seg_item(
                                "pref-read-w",
                                t("settings.width_wide"),
                                prefs.reading_width == ReadingWidth::Wide,
                                {
                                    let state = state.clone();
                                    move |_, cx| {
                                        patch_prefs(&state, cx, |p| {
                                            p.reading_width = ReadingWidth::Wide
                                        })
                                    }
                                },
                            ),
                        ],
                    ),
                )
                .into_any_element(),
            ],
        ))
}
