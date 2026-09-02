use gpui::{div, prelude::*, Entity};

use super::super::widgets::{seg_item, setting_row, settings_group};
use super::{patch_prefs, picker};
use crate::prefs::{BlockDelim, InlineDelim, Prefs, ReadingWidth};
use crate::state::AppState;

pub(super) fn formatting_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            "Math delimiters",
            vec![
                setting_row(
                    "Inline math",
                    "Wraps inline formulas in Markdown and mixed LaTeX export.",
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
                    "Display math",
                    "Wraps longer formulas in document export. Snip copy also offers equation.",
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
            "Preview",
            vec![setting_row(
                "Reading width",
                "Caps the copy column in a wide window. Tables and display math can still scroll.",
                picker(
                    210.,
                    [
                        seg_item(
                            "pref-read-n",
                            "Narrow",
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
                            "Medium",
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
                            "Wide",
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
            .into_any_element()],
        ))
}
