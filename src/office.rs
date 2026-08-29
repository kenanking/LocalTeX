use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::doc::{unwrap_formula, Block, BlockKind, BlockRole};
use crate::math::{format_eqno, split_display_tag, split_math, MathRun};
use crate::table::{self, Slot};

const MATHML_NS: &str = "http://www.w3.org/1998/Math/MathML";
const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const M_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

/// Presentation MathML for a formula snip. Word pastes this as an equation.
pub fn formula_mathml(latex: &str) -> String {
    let tex = prepare_tex(latex);
    if tex.is_empty() {
        return String::new();
    }
    match latex2mathml::latex_to_mathml(&tex, latex2mathml::DisplayStyle::Block) {
        Ok(mml) => mml,
        Err(_) => format!(
            "<math xmlns=\"{MATHML_NS}\" display=\"block\"><mtext>{}</mtext></math>",
            xml_escape(&tex)
        ),
    }
}

fn prepare_tex(latex: &str) -> String {
    let body = unwrap_formula(latex).0;
    let (tex, _) = split_display_tag(&body);
    unwrap_boxed(tex.trim())
}

fn unwrap_boxed(tex: &str) -> String {
    let mut t = tex.to_string();
    let needle = r"\boxed{";
    while let Some(at) = t.find(needle) {
        let rest = &t[at + needle.len()..];
        let Some((inner, after)) = crate::math::split_braced(rest) else {
            break;
        };
        t = format!("{}{}{}", &t[..at], inner, after);
    }
    t
}

pub fn build_docx(blocks: &[Block]) -> anyhow::Result<Vec<u8>> {
    let document = document_xml(blocks);
    pack_opc(&document)
}

fn pack_opc(document_xml: &str) -> anyhow::Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("[Content_Types].xml", opts)?;
    zip.write_all(CONTENT_TYPES.as_bytes())?;
    zip.start_file("_rels/.rels", opts)?;
    zip.write_all(ROOT_RELS.as_bytes())?;
    zip.start_file("word/_rels/document.xml.rels", opts)?;
    zip.write_all(DOC_RELS.as_bytes())?;
    zip.start_file("word/styles.xml", opts)?;
    zip.write_all(STYLES.as_bytes())?;
    zip.start_file("word/document.xml", opts)?;
    zip.write_all(document_xml.as_bytes())?;
    Ok(zip.finish()?.into_inner())
}

fn document_xml(blocks: &[Block]) -> String {
    let mut body = String::new();
    for block in blocks {
        if block.text.trim().is_empty() {
            continue;
        }
        match block.kind {
            BlockKind::Table => body.push_str(&table_xml(&block.text)),
            BlockKind::Formula => body.push_str(&display_math_paragraph(&block.text)),
            BlockKind::Text => match block.role {
                BlockRole::DocTitle => {
                    body.push_str(&styled_paragraph("Heading1", &block.text));
                }
                BlockRole::SectionTitle => {
                    body.push_str(&styled_paragraph("Heading2", &block.text));
                }
                BlockRole::Caption => {
                    body.push_str(&styled_paragraph("Caption", &block.text));
                }
                BlockRole::Body => body.push_str(&body_paragraph(&block.text)),
            },
        }
    }
    if body.is_empty() {
        body.push_str("<w:p/>");
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W_NS}" xmlns:m="{M_NS}">
<w:body>
{body}
<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720"/></w:sectPr>
</w:body>
</w:document>"#
    )
}

fn styled_paragraph(style: &str, text: &str) -> String {
    format!(
        "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>{}</w:p>",
        inline_runs(text)
    )
}

fn body_paragraph(text: &str) -> String {
    format!("<w:p>{}</w:p>", inline_runs(text))
}

fn display_math_paragraph(latex: &str) -> String {
    let body = unwrap_formula(latex).0;
    let (tex, tags) = split_display_tag(&body);
    let mut inner = omath(&tex, true);
    if let Some(tag) = tags.first() {
        inner.push_str(&text_run(&format!(" {}", format_eqno(tag))));
    }
    format!("<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr>{inner}</w:p>")
}

fn inline_runs(text: &str) -> String {
    let mut out = String::new();
    for run in split_math(text) {
        match run {
            MathRun::Text(s) => {
                if !s.is_empty() {
                    out.push_str(&text_run(&s));
                }
            }
            MathRun::Inline(tex) | MathRun::Display(tex) => {
                out.push_str(&omath(&tex, false));
            }
        }
    }
    if out.is_empty() {
        out.push_str(&text_run(""));
    }
    out
}

fn omath(latex: &str, display: bool) -> String {
    let tex = prepare_tex(latex);
    if tex.is_empty() {
        return String::new();
    }
    let inner = tex2word_math::to_omath(&tex);
    if display {
        format!("<m:oMathPara>{inner}</m:oMathPara>")
    } else {
        inner
    }
}

fn text_run(text: &str) -> String {
    let escaped = xml_escape(text);
    if text.starts_with(' ') || text.ends_with(' ') {
        format!("<w:r><w:t xml:space=\"preserve\">{escaped}</w:t></w:r>")
    } else {
        format!("<w:r><w:t>{escaped}</w:t></w:r>")
    }
}

fn table_xml(html: &str) -> String {
    let Some(parsed) = table::parse_html(html) else {
        return body_paragraph(html);
    };
    let grid = parsed.slot_grid();
    if grid.is_empty() {
        return body_paragraph(html);
    }
    let cols = grid.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
    let col_w = (9000 / cols as i32).max(800);
    let mut out = String::from("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders>");
    for edge in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        out.push_str(&format!(
            "<w:{edge} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"999999\"/>"
        ));
    }
    out.push_str("</w:tblBorders></w:tblPr><w:tblGrid>");
    for _ in 0..cols {
        out.push_str(&format!("<w:gridCol w:w=\"{col_w}\"/>"));
    }
    out.push_str("</w:tblGrid>");
    for row in &grid {
        out.push_str("<w:tr>");
        for slot in row {
            match slot {
                Slot::ColSpan => {}
                Slot::RowSpan => {
                    out.push_str("<w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc>");
                }
                Slot::Origin {
                    text,
                    rowspan,
                    colspan,
                } => {
                    out.push_str("<w:tc><w:tcPr>");
                    if *colspan > 1 {
                        out.push_str(&format!("<w:gridSpan w:val=\"{colspan}\"/>"));
                    }
                    if *rowspan > 1 {
                        out.push_str("<w:vMerge w:val=\"restart\"/>");
                    }
                    out.push_str("</w:tcPr>");
                    out.push_str(&body_paragraph(text));
                    out.push_str("</w:tc>");
                }
            }
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    out
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="26"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="caption"/><w:basedOn w:val="Normal"/><w:rPr><w:i/><w:sz w:val="20"/></w:rPr></w:style>
</w:styles>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Block, BlockKind, BlockRole, Rect};
    use zip::ZipArchive;

    fn rect() -> Rect {
        Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        }
    }

    fn unzip_document(bytes: &[u8]) -> String {
        let mut zip = ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
        let mut file = zip.by_name("word/document.xml").unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut file, &mut s).unwrap();
        s
    }

    #[test]
    fn formula_mathml_is_block_presentation() {
        let mml = formula_mathml(r"E = mc^2");
        assert!(
            mml.contains(r#"xmlns="http://www.w3.org/1998/Math/MathML""#),
            "{mml}"
        );
        assert!(mml.contains(r#"display="block""#), "{mml}");
        assert!(mml.contains("<mi>E</mi>"), "{mml}");
        assert!(mml.contains("<msup>"), "{mml}");
    }

    #[test]
    fn formula_mathml_strips_tag() {
        let mml = formula_mathml(r"x^{2} \tag{11}");
        assert!(!mml.contains("tag"), "{mml}");
        assert!(mml.contains("<msup>"), "{mml}");
    }

    #[test]
    fn formula_mathml_unwraps_boxed() {
        let mml = formula_mathml(r"\boxed{E}=mc^2");
        assert!(!mml.contains("PARSE ERROR"), "{mml}");
        assert!(!mml.contains("boxed"), "{mml}");
        assert!(mml.contains("<mi>E</mi>"), "{mml}");
    }

    #[test]
    fn formula_mathml_escapes_fallback() {
        let mml = formula_mathml(r"a < b & c");
        assert!(!mml.contains("a < b"), "{mml}");
    }

    #[test]
    fn docx_has_omml_and_heading() {
        let blocks = vec![
            Block::new(BlockKind::Text, rect(), "Paper title").with_role(BlockRole::DocTitle),
            Block::new(BlockKind::Text, rect(), r"Let $E=mc^2$ hold."),
            Block::new(BlockKind::Formula, rect(), r"x^{2}+y^{2}=r^{2} \tag{1}"),
        ];
        let bytes = build_docx(&blocks).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("w:val=\"Heading1\""), "{xml}");
        assert!(xml.contains("Paper title"), "{xml}");
        assert!(xml.contains("<m:oMath>"), "{xml}");
        assert!(xml.contains("<m:oMathPara>"), "{xml}");
        assert!(xml.contains("(1)"), "{xml}");
        assert!(xml.contains("&lt;") || xml.contains("Let "), "{xml}");
    }

    #[test]
    fn docx_table_preserves_grid() {
        let html = "<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>$x^2$</td></tr></table>";
        let blocks = vec![Block::new(BlockKind::Table, rect(), html)];
        let xml = unzip_document(&build_docx(&blocks).unwrap());
        assert!(xml.contains("<w:tbl>"), "{xml}");
        assert!(xml.contains("<m:oMath>"), "{xml}");
        assert!(xml.contains(">A</w:t>"), "{xml}");
    }

    #[test]
    fn empty_blocks_still_pack() {
        let bytes = build_docx(&[]).unwrap();
        let xml = unzip_document(&bytes);
        assert!(xml.contains("<w:body>"), "{xml}");
    }
}
