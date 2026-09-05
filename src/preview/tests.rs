use super::*;

fn dummy_derived(id: Uuid) -> DocDerived {
    DocDerived {
        id,
        revision: 1,
        dpr: 1.0,
        inline_delim: InlineDelim::Dollar,
        block_delim: BlockDelim::Dollars,
        content_font: ContentFontSize::Medium,
        preview: Arc::from([]),
        copy_rows: Vec::new(),
    }
}

#[test]
fn derived_rejects_another_document_and_older_revision() {
    let id = Uuid::new_v4();
    let other = Uuid::new_v4();
    let prefs = Prefs::default();
    assert!(dummy_derived(id).matches(id, 1, 1.0, &prefs));
    assert!(!dummy_derived(id).matches(other, 1, 1.0, &prefs));
    assert!(!dummy_derived(id).matches(id, 2, 1.0, &prefs));
}

#[test]
fn invalid_display_preserves_engine_error_and_other_blocks() {
    let source = "Before\n\n$$\\frac{$$\n\nAfter\n\n$$x+1$$";
    let blocks = crate::source::parse_source(source, &Prefs::default()).unwrap();
    let preview = document_preview_with_dpr(&blocks, 1.0, ContentFontSize::Medium);
    let engine_error = latex_to_math_with_dpr(r"\frac{", MathStyle::Display, 1.0)
        .unwrap_err()
        .to_string();
    assert!(preview.iter().any(|block| matches!(block, PreviewBlock::Error { source, error } if source.contains(r"\frac{") && error == &engine_error)));
    assert!(preview
        .iter()
        .any(|block| matches!(block, PreviewBlock::Display { .. })));
    assert_eq!(
        preview
            .iter()
            .filter(|block| matches!(block, PreviewBlock::Paragraph(_)))
            .count(),
        2
    );
}

#[test]
fn invalid_inline_preserves_source_and_engine_error() {
    let segs = inline_preview(r"Before $\frac{$ after");
    assert!(segs.iter().any(|seg| matches!(seg, InlineSeg::Error { source, error } if source == r"$\frac{$" && error.contains("ratex parse:"))));
}

#[test]
fn should_spawn_derived_retries_after_busy() {
    assert!(!should_spawn_derived(true, false, true));
    assert!(should_spawn_derived(true, false, false));
    assert!(!should_spawn_derived(true, true, false));
    assert!(!should_spawn_derived(false, false, false));
}

#[test]
fn derived_matches_false_when_content_font_differs() {
    let id = Uuid::new_v4();
    let derived = dummy_derived(id);
    let mut prefs = Prefs::default();
    assert!(derived.matches(id, 1, 1.0, &prefs));
    prefs.content_font = ContentFontSize::Large;
    assert!(!derived.matches(id, 1, 1.0, &prefs));
}

#[test]
fn renders_simple_formula_svg() {
    let math = latex_to_math("E=mc^2", MathStyle::Display).expect("ratex");
    let svg = &math.svg;
    assert!(
        svg.contains("<svg"),
        "expected svg, got: {}",
        &svg[..svg.len().min(200)]
    );
    assert!(
        !svg.contains("rgba("),
        "usvg cannot paint rgba() fills: {}",
        &svg[..svg.len().min(200)]
    );
    assert!(svg.contains("rgb("));
    let math = latex_to_math("E=mc^2", MathStyle::Display).expect("size");
    assert!(math.width > 0.0 && math.height > 0.0);
    assert!(
        math.height < 48.0,
        "body-size math should not be huge, got h={}",
        math.height
    );
}

#[test]
fn renders_spaced_tokens_aligned() {
    let compact = r"\begin{aligned} {\mathbf{z}}_{v}^{i}=f_{v}^{(0)}({\mathbf{I}}_{i}^{\mathsf{opt}}), \\ \end{aligned}";
    let svg = latex_to_math(compact, MathStyle::Display)
        .expect("compact aligned")
        .svg;
    assert!(svg.contains("<svg"));
    assert!(!svg.contains("rgba("));
    let spaced = r"\begin{aligned} {\mathbf { z } _ { v } ^ { i } } = f _ { v } ^ { ( 0 ) } ( \mathbf { I } _ { i } ^ { \mathsf { o p t } } ) , \\ \end{aligned}";
    latex_to_math(spaced, MathStyle::Display).expect("spaced aligned");
    assert_eq!(
        compact_tex(r"\mathbf { z } _ { v } ^ { i }"),
        r"\mathbf{z}_{v}^{i}"
    );
}

#[test]
fn mixed_text_preview_has_paragraph() {
    let blocks = vec![Block::new(
        BlockKind::Text,
        crate::doc::Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        },
        "Because $c$ and $1-c$.",
    )];
    let preview = document_preview(&blocks);
    assert!(
        preview
            .iter()
            .any(|b| matches!(b, PreviewBlock::Paragraph(_))),
        "expected a paragraph"
    );
}

#[test]
fn title_and_body_are_separate_preview_blocks() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 10,
        h: 10,
    };
    let blocks = vec![
        Block::new(BlockKind::Text, r, "Introduction").with_role(crate::doc::BlockRole::DocTitle),
        Block::new(BlockKind::Text, r, "The method works."),
    ];
    let preview = document_preview(&blocks);
    assert!(
        matches!(
            preview.as_slice(),
            [
                PreviewBlock::Heading {
                    role: crate::doc::BlockRole::DocTitle,
                    ..
                },
                PreviewBlock::Paragraph(_)
            ]
        ),
        "title must not weld to body, got {preview:?}"
    );
}

fn paragraph_texts(preview: &[PreviewBlock]) -> Vec<String> {
    preview
        .iter()
        .filter_map(|b| match b {
            PreviewBlock::Paragraph(segs) => Some(
                segs.iter()
                    .filter_map(|s| match s {
                        InlineSeg::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn stacked_body_texts_are_separate_paragraphs() {
    let top = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 200,
        h: 20,
    };
    let below = crate::doc::Rect {
        x: 0,
        y: 40,
        w: 200,
        h: 20,
    };
    let blocks = vec![
        Block::new(BlockKind::Text, top, "First paragraph of the snip."),
        Block::new(BlockKind::Text, below, "Second paragraph of the snip."),
    ];
    let preview = document_preview(&blocks);
    assert_eq!(
        paragraph_texts(&preview),
        [
            "First paragraph of the snip.".to_string(),
            "Second paragraph of the snip.".to_string(),
        ],
        "stacked OCR body blocks must not weld, got {preview:?}"
    );
}

#[test]
fn editor_unit_rect_body_texts_are_separate_paragraphs() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 1,
        h: 1,
    };
    let blocks = vec![
        Block::new(BlockKind::Text, r, "Edited first paragraph."),
        Block::new(BlockKind::Text, r, "Edited second paragraph."),
    ];
    let preview = document_preview(&blocks);
    assert_eq!(
        paragraph_texts(&preview),
        [
            "Edited first paragraph.".to_string(),
            "Edited second paragraph.".to_string(),
        ],
        "source-parsed body blocks share a unit rect and must still split, got {preview:?}"
    );
}

#[test]
fn short_formula_block_stays_inline_with_neighbors() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 40,
        h: 16,
    };
    let blocks = vec![
        Block::new(BlockKind::Text, r, "Let "),
        Block::new(BlockKind::Formula, r, "E=mc^2"),
        Block::new(BlockKind::Text, r, " denote energy."),
    ];
    let preview = document_preview(&blocks);
    assert_eq!(
        preview.len(),
        1,
        "expected a single paragraph, got {preview:?}"
    );
    match &preview[0] {
        PreviewBlock::Paragraph(segs) => {
            assert!(
                segs.iter().any(|s| matches!(s, InlineSeg::Math { .. })),
                "expected inline math"
            );
        }
        other => panic!("expected paragraph, got {other:?}"),
    }
}

#[test]
fn table_cell_newlines_still_render_inline_math() {
    let segs = inline_preview("SARCLIP\n$\n\\dagger\n$");
    assert!(
        segs.iter().any(|s| matches!(s, InlineSeg::Math { .. })),
        "expected $\\dagger$ to become math, got {segs:?}"
    );
    assert!(
        segs.iter().any(|s| match s {
            InlineSeg::Text(t) => t.contains("SARCLIP"),
            _ => false,
        }),
        "expected SARCLIP text, got {segs:?}"
    );
}

#[test]
fn subscript_formula_is_marked_and_smaller() {
    let segs = inline_preview(r"H$_2$O");
    let sub = segs.iter().find_map(|s| match s {
        InlineSeg::TextScript { glyph, .. } if glyph.kind == ScriptKind::Sub => {
            Some((glyph.svg.height, glyph.tex.as_str()))
        }
        _ => None,
    });
    let Some((sub_h, tex)) = sub else {
        panic!("expected a TextScript subscript, got {segs:?}");
    };
    assert!(tex.contains('2'), "subscript tex {tex}");
    let full = inline_preview("$x$");
    let full_h = full
        .iter()
        .find_map(|s| match s {
            InlineSeg::Math { svg, .. } => Some(svg.height),
            _ => None,
        })
        .expect("inline x");
    assert!(
        sub_h < full_h,
        "subscript {sub_h} should be shorter than inline {full_h}"
    );
}

#[test]
fn orphan_digit_after_letter_is_subscript() {
    let segs = inline_preview("H$2$O");
    assert!(
        segs.iter().any(|s| matches!(
            s,
            InlineSeg::TextScript {
                glyph: ScriptGlyph {
                    kind: ScriptKind::Sub,
                    ..
                },
                ..
            }
        )),
        "expected orphan $2$ after H to be a TextScript subscript, got {segs:?}"
    );
}

#[test]
fn empty_nucleus_script_from_ocr_is_subscript() {
    let segs = inline_preview(r"AP${}_{50}$, and");
    assert!(
        segs.iter().any(|s| matches!(
            s,
            InlineSeg::TextScript {
                glyph: ScriptGlyph {
                    kind: ScriptKind::Sub,
                    ..
                },
                ..
            }
        )),
        "expected {{}}_{{50}} after AP to be a TextScript subscript, got {segs:?}"
    );
}

#[test]
fn document_preview_marks_empty_nucleus_subscripts() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 10,
        h: 10,
    };
    let blocks = vec![Block::new(
        BlockKind::Text,
        r,
        r"reaching 36.02 mAP, 69.60 AP${}_{50}$, and 33.80 AP${}_{75}$.",
    )];
    let preview = document_preview(&blocks);
    let segs = match &preview[0] {
        PreviewBlock::Paragraph(segs) => segs,
        other => panic!("expected paragraph, got {other:?}"),
    };
    let subs: Vec<&str> = segs
        .iter()
        .filter_map(|s| match s {
            InlineSeg::TextScript {
                glyph:
                    ScriptGlyph {
                        kind: ScriptKind::Sub,
                        tex,
                        ..
                    },
                ..
            } => Some(tex.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(subs, ["{}_{50}", "{}_{75}"]);
    assert!(
        segs.iter().any(|s| matches!(
            s,
            InlineSeg::TextScript { nucleus, .. } if nucleus.ends_with("AP")
        )),
        "script must attach to the AP text run, got {segs:?}"
    );
}

#[test]
fn next_script_keeps_leading_space_on_its_nucleus() {
    let segs = inline_preview(r"AP${}_{50}$ by 2.65%, and AP${}_{75}$");
    let nuclei: Vec<&str> = segs
        .iter()
        .filter_map(|s| match s {
            InlineSeg::TextScript { nucleus, .. } => Some(nucleus.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        nuclei.len(),
        2,
        "expected two TextScript atoms, got {segs:?}"
    );
    assert!(
        nuclei[1].starts_with(' '),
        "space after the first script stays on the next nucleus, got {nuclei:?}"
    );
}

#[test]
fn script_glyph_for_orphans_after_letters() {
    assert_eq!(
        script_glyph_for(Some('H'), "2"),
        Some((ScriptKind::Sub, "2".into()))
    );
    assert_eq!(
        script_glyph_for(Some('H'), "_2"),
        Some((ScriptKind::Sub, "2".into()))
    );
    assert_eq!(script_glyph_for(Some(' '), "x"), None);
    assert_eq!(script_glyph_for(None, "2"), None);
    assert_eq!(
        script_glyph_for(Some('n'), "^2"),
        Some((ScriptKind::Super, "2".into()))
    );
}

#[test]
fn orphan_script_body_is_a_short_nucleus() {
    assert!(is_orphan_script_body("2"));
    assert!(is_orphan_script_body("ij"));
    assert!(is_orphan_script_body(r"\mathrm{max}"));
    assert!(!is_orphan_script_body("x+y"));
    assert!(!is_orphan_script_body(r"\frac{1}{2}"));
    assert!(!is_orphan_script_body("E=mc^2"));
}

#[test]
fn section_title_is_heading_not_paragraph() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 10,
        h: 10,
    };
    let blocks = vec![
        Block::new(BlockKind::Text, r, "Method").with_role(crate::doc::BlockRole::SectionTitle),
        Block::new(BlockKind::Text, r, "Details."),
    ];
    let preview = document_preview(&blocks);
    assert!(
        matches!(
            preview.as_slice(),
            [
                PreviewBlock::Heading {
                    role: crate::doc::BlockRole::SectionTitle,
                    ..
                },
                PreviewBlock::Paragraph(_)
            ]
        ),
        "section title must not weld to body, got {preview:?}"
    );
}

#[test]
fn table_list_cell_keeps_item_breaks() {
    let html = "<table><tr><td>- one\n- two</td></tr></table>";
    let table = table::parse_html(html).unwrap();
    let layout =
        table_layout::table_preview_layout(&table, raster_dpr(1.0), ContentFontSize::Medium);
    assert_eq!(layout.cells.len(), 1);
    let lines = segs_lines(&layout.cells[0].segs);
    assert_eq!(lines.len(), 2, "got {lines:?}");
}

#[test]
fn raster_svg_is_larger_than_display_box() {
    let math = latex_to_math("E=mc^2", MathStyle::Display).expect("ratex");
    let file_w = super::attr_pt(&math.svg, "width").expect("svg width");
    assert!(
        file_w > math.width * raster_dpr(1.0) as f32 * 0.9,
        "SVG file should be DPR-scaled, file={file_w} display={}",
        math.width
    );
}

#[test]
fn lone_formula_snip_is_display() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 40,
        h: 16,
    };
    let preview = document_preview(&[Block::new(BlockKind::Formula, r, "E=mc^2")]);
    assert!(
        matches!(preview.as_slice(), [PreviewBlock::Display { .. }]),
        "lone formula snip should be a centered display block, got {preview:?}"
    );
}

#[test]
fn numbered_display_in_prose_stays_display() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 200,
        h: 24,
    };
    let preview = document_preview(&[Block::new(
        BlockKind::Text,
        r,
        r"Hence $$E=mc^2 \tag{1}$$ holds.",
    )]);
    let tagged = preview.iter().find_map(|b| match b {
        PreviewBlock::Display { eqno, .. } => eqno.as_ref().map(|e| e.raw.as_str()),
        _ => None,
    });
    assert_eq!(
        tagged,
        Some("1"),
        "tagged display math should not collapse to inline, got {preview:?}"
    );
}

#[test]
fn display_tag_is_sidecar_not_typeset() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 40,
        h: 16,
    };
    let preview = document_preview(&[Block::new(BlockKind::Formula, r, r"$$E=mc^2 \tag{11}$$")]);
    match preview.as_slice() {
        [PreviewBlock::Display {
            eqno: Some(eqno), ..
        }] => {
            assert_eq!(eqno.raw, "11");
            assert!(
                eqno.math.is_some(),
                "eqno should typeset as display math, not a muted UI label"
            );
        }
        other => panic!("expected tagged display, got {other:?}"),
    }
    let (body, tags) = crate::math::split_display_tag(r"E=mc^2 \tag{11}");
    assert_eq!(tags, vec!["11".to_string()]);
    latex_to_math(&body, MathStyle::Display).expect("body without tag");
    let (body, _) = crate::math::unwrap_formula(
        r"{\rm ACC}=\frac{1}{N}I\left[\hat{y}_{i}=y_{i}\right],\\tag{1}\\
\\",
    );
    let (body, tags) = crate::math::split_display_tag(&body);
    assert_eq!(tags, vec!["1".to_string()]);
    latex_to_math(&body, MathStyle::Display).expect("repaired tag body");
}

#[test]
fn display_formula_block_stays_display_with_neighbors() {
    let r = crate::doc::Rect {
        x: 0,
        y: 0,
        w: 40,
        h: 16,
    };
    let blocks = vec![
        Block::new(BlockKind::Text, r, "We have"),
        Block {
            kind: BlockKind::Formula,
            bbox: r,
            text: "E=mc^2".into(),
            display: true,
            role: crate::doc::BlockRole::Body,
        },
        Block::new(BlockKind::Text, r, "as usual."),
    ];
    let preview = document_preview(&blocks);
    assert!(
        preview
            .iter()
            .any(|b| matches!(b, PreviewBlock::Display { .. })),
        "layout display_formula should stay display even when short, got {preview:?}"
    );
}

#[test]
fn display_style_sum_is_taller_than_text_style() {
    let tex = r"\sum_{i=1}^{n} i";
    let display = latex_to_math(tex, MathStyle::Display).expect("display");
    let inline = latex_to_math(tex, MathStyle::Text).expect("text");
    assert!(
        display.height > inline.height,
        "display limits should sit above/below, h_d={} h_t={}",
        display.height,
        inline.height
    );
}

#[test]
fn preview_layout_merges_header_spans() {
    let html = r#"<table>
<tr>
<th rowspan="2">Method</th>
<th rowspan="2">Image Backbone</th>
<th colspan="3">Image to Text</th>
</tr>
<tr><th>R@1</th><th>R@5</th><th>R@10</th></tr>
<tr><td>OpenCLIP</td><td>ViT-B/32</td><td>11.1</td><td>22.2</td><td>33.3</td></tr>
</table>"#;
    let t = table::parse_html(html).unwrap();
    let lay = table_layout::table_preview_layout(&t, raster_dpr(1.0), ContentFontSize::Medium);
    let method = lay
        .cells
        .iter()
        .find(|c| cell_plain(c) == "Method")
        .expect("Method");
    let backbone = lay
        .cells
        .iter()
        .find(|c| cell_plain(c) == "Image Backbone")
        .expect("Backbone");
    let group = lay
        .cells
        .iter()
        .find(|c| cell_plain(c) == "Image to Text")
        .expect("group");
    let r1 = lay
        .cells
        .iter()
        .find(|c| cell_plain(c) == "R@1")
        .expect("R@1");
    let r5 = lay
        .cells
        .iter()
        .find(|c| cell_plain(c) == "R@5")
        .expect("R@5");
    let r10 = lay
        .cells
        .iter()
        .find(|c| cell_plain(c) == "R@10")
        .expect("R@10");
    assert!(
        !lay.cells.iter().any(|c| c.col == 0 && c.row == 1),
        "rowspan must not leave an empty cell under Method"
    );
    assert!(
        (r1.x - group.x).abs() < 0.51,
        "R@1 left {} vs group {}",
        r1.x,
        group.x
    );
    assert!(
        (group.w - (r1.w + r5.w + r10.w)).abs() < 0.51,
        "group width {} vs R@ sum {}",
        group.w,
        r1.w + r5.w + r10.w
    );
    assert!(
        method.h > r1.h + 1.0,
        "Method should span both header rows ({} vs {})",
        method.h,
        r1.h
    );
    assert!(
        backbone.w > 88.0,
        "content-sized columns, not a uniform 88px: {}",
        backbone.w
    );
    assert!(
        method.header
            && r1.header
            && !lay
                .cells
                .iter()
                .any(|c| cell_plain(c) == "OpenCLIP" && c.header)
    );
    assert!(lay
        .cells
        .iter()
        .find(|c| cell_plain(c) == "11.1")
        .is_some_and(|c| c.numeric));
}

fn cell_plain(c: &PlacedCell) -> String {
    c.segs
        .iter()
        .map(|s| match s {
            InlineSeg::Text(t) => t.as_str(),
            InlineSeg::TextScript { nucleus, .. } => nucleus.as_str(),
            InlineSeg::Math { .. } => "",
            InlineSeg::Error { source, .. } => source.as_str(),
        })
        .collect()
}
