use gpui::{div, prelude::*, Entity};

use super::super::widgets::{seg_item, setting_row, settings_group};
use super::{patch_prefs, picker};
use crate::doc::ExportFmt;
use crate::prefs::{BlockDelim, InlineDelim, Prefs};
use crate::state::AppState;

pub(super) fn formatting_page(state: Entity<AppState>, prefs: &Prefs) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .child(settings_group(
            "Export",
            vec![setting_row(
                "Primary format",
                "Used for Ctrl+C and auto-copy. Extra formats stay available on each snip.",
                picker(
                    150.,
                    [
                        seg_item(
                            "pref-md",
                            ExportFmt::Markdown.label(),
                            prefs.default_fmt == ExportFmt::Markdown,
                            {
                                let state = state.clone();
                                move |_, cx| {
                                    state.update(cx, |s, cx| {
                                        s.update_prefs(cx, |p| p.default_fmt = ExportFmt::Markdown);
                                        s.set_format(ExportFmt::Markdown, cx);
                                    });
                                }
                            },
                        ),
                        seg_item(
                            "pref-tex",
                            ExportFmt::Latex.label(),
                            prefs.default_fmt == ExportFmt::Latex,
                            {
                                let state = state.clone();
                                move |_, cx| {
                                    state.update(cx, |s, cx| {
                                        s.update_prefs(cx, |p| p.default_fmt = ExportFmt::Latex);
                                        s.set_format(ExportFmt::Latex, cx);
                                    });
                                }
                            },
                        ),
                    ],
                ),
            )
            .into_any_element()],
        ))
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
}
