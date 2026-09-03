//! Layout → per-region UniRec → LocalTeX `doc::Block`s.
//! Recognition and label postprocess follow OpenDocONNX.__call__; markdown
//! file assembly and figure-token painting are out of scope for this app.
//! `formula_number` regions are paired with `display_formula` and folded
//! into `\tag{N}`. A trailing `(n)` in formula OCR is not promoted without
//! independent layout evidence.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use ort::session::Session;

use super::imgops::{self, RgbImg};
use super::layout::{self, Region, IMAGE_LABELS};
use super::text::{self, IGNORE_LABELS};
use super::unirec::{RecognizeOut, Tokenizer, UniRec};
use super::{build_session, inspect_onnx_pack, PackMetadata};
use crate::doc::{Block, BlockKind, BlockRole, OcrMeta, Rect};
use crate::identity::APP_SLUG;

pub const LAYOUT_ONNX: &str = "layout.onnx";
pub const ENCODER_ONNX: &str = "encoder.onnx";
pub const DECODER_ONNX: &str = "decoder.onnx";
pub const TOKENIZER_JSON: &str = "unirec_tokenizer_mapping.json";

pub const OPENDOC_FILES: [&str; 4] = [LAYOUT_ONNX, ENCODER_ONNX, DECODER_ONNX, TOKENIZER_JSON];

#[derive(Clone)]
pub struct OcrResult {
    pub blocks: Vec<Block>,
    pub meta: Option<OcrMeta>,
}

pub struct Pipeline {
    layout: Session,
    unirec: UniRec,
    pack_metadata: PackMetadata,
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v != "0")
}

impl Pipeline {
    pub fn load(dir: &Path, intra: usize) -> Result<Self> {
        let spinning = env_flag("LOCALTEX_SPINNING");
        eprintln!(
            "{APP_SLUG}: intra_op_threads={intra} spinning={}",
            if spinning { "on" } else { "off" }
        );
        let layout = build_session(&dir.join(LAYOUT_ONNX), intra, spinning, false)?;
        let encoder = build_session(&dir.join(ENCODER_ONNX), intra, spinning, true)?;
        let decoder = build_session(&dir.join(DECODER_ONNX), intra, spinning, true)?;
        let pack_metadata = inspect_onnx_pack(&[
            (&layout, "layout"),
            (&encoder, "unirec_encoder"),
            (&decoder, "unirec_decoder"),
        ]);
        let tokenizer = Tokenizer::load(&dir.join(TOKENIZER_JSON))?;
        let unirec = UniRec::new(encoder, decoder, tokenizer)?;
        eprintln!("{APP_SLUG}: layout dynamic auto · decoder GQA");
        Ok(Self {
            layout,
            unirec,
            pack_metadata,
        })
    }

    pub(super) fn pack_metadata(&self) -> &PackMetadata {
        &self.pack_metadata
    }

    fn recognize_region(&mut self, mut crop: RgbImg, kind: RecKind) -> Result<RecognizeOut> {
        if kind == RecKind::Formula {
            crop = imgops::crop_margin(&crop);
        }
        let mut out = self.unirec.recognize(&crop)?;
        if kind != RecKind::Text || !looks_like_dropped_english_spaces(&crop, &out.text) {
            return Ok(out);
        }

        let retry = self
            .unirec
            .recognize(&imgops::pad_to_unirec_height_128(&crop))?;
        let timing = (
            out.encode_s + retry.encode_s,
            out.decode_s + retry.decode_s,
            out.decode_steps + retry.decode_steps,
        );
        if only_adds_whitespace(&out.text, &retry.text) {
            out = retry;
        }
        out.encode_s = timing.0;
        out.decode_s = timing.1;
        out.decode_steps = timing.2;
        Ok(out)
    }

    pub fn infer(&mut self, image: &mut RgbImg) -> Result<OcrResult> {
        let t0 = std::time::Instant::now();
        imgops::invert_if_dark(image);

        let layout_out = layout::detect(&mut self.layout, image, 0.5)?;
        let regions = layout_out.regions;
        let n_layout = regions.len();
        // Pair before the move: number-block index → display_formula index.
        let tag_pairs = pair_formula_numbers(&regions);

        let mut encode_s = 0.0f64;
        let mut decode_s = 0.0f64;
        let mut decode_steps = 0usize;
        let mut conf = ConfAcc::default();
        let mut pending: Vec<Option<PendingRec>> = Vec::with_capacity(regions.len());

        for region in regions {
            let base = text::base_label(&region.label);
            let kind = rec_kind(base);
            if kind == RecKind::Skip {
                pending.push(None);
                continue;
            }
            let Some(crop) = region.img else {
                pending.push(None);
                continue;
            };
            let out = self.recognize_region(crop, kind)?;
            encode_s += out.encode_s;
            decode_s += out.decode_s;
            decode_steps += out.decode_steps;
            conf.add(out.p_sum, out.p_n, region.score, region.coord);
            pending.push(Some(PendingRec {
                base: base.to_string(),
                coord: region.coord,
                text: postprocess(kind, out.text),
            }));
        }

        for rec in pending.iter_mut().flatten() {
            if rec.base == "display_formula" {
                rec.text = text::remove_unverified_formula_tags(&rec.text);
            }
        }

        let mut tags_for: HashMap<usize, Vec<String>> = HashMap::new();
        let mut consumed = vec![false; pending.len()];
        for (bi, rec) in pending.iter().enumerate() {
            let Some(rec) = rec else {
                continue;
            };
            if let Some(&fj) = tag_pairs.get(&bi) {
                if let Some(tag) = clean_tag(&rec.text) {
                    tags_for.entry(fj).or_default().push(tag);
                    consumed[bi] = true;
                }
            }
        }
        for (fj, tags) in &tags_for {
            if let Some(Some(rec)) = pending.get_mut(*fj) {
                rec.text = text::inject_tags(&rec.text, tags);
            }
        }

        let mut blocks = Vec::new();
        for (i, rec) in pending.into_iter().enumerate() {
            if consumed[i] {
                continue;
            }
            let Some(rec) = rec else {
                continue;
            };
            // Unpaired numbers stay page-chrome (not a Text block).
            if rec.base == "formula_number" {
                continue;
            }
            if let Some(block) = to_doc_block(&rec.base, rec.coord, &rec.text) {
                blocks.push(block);
            }
        }

        let elapsed_s = t0.elapsed().as_secs_f32();
        let meta = conf.finish().map(|confidence| OcrMeta {
            elapsed_s,
            confidence,
        });

        eprintln!(
            "{APP_SLUG}: OpenDoc {}/{} regions in {:.2}s (layout {:.2}s encode {:.2}s decode {:.2}s / {} steps{})",
            blocks.len(),
            n_layout,
            elapsed_s,
            layout_out.layout_s,
            encode_s,
            decode_s,
            decode_steps,
            meta.map(|m| format!(", conf {:.2}", m.confidence))
                .unwrap_or_default()
        );
        Ok(OcrResult { blocks, meta })
    }
}

fn looks_like_dropped_english_spaces(crop: &RgbImg, text: &str) -> bool {
    if crop.w.saturating_mul(2) <= crop.h.saturating_mul(15)
        || text.chars().any(char::is_whitespace)
    {
        return false;
    }
    let latin = text.bytes().filter(u8::is_ascii_alphabetic).count();
    latin >= 20 && latin * 5 >= text.chars().count() * 4
}

fn only_adds_whitespace(baseline: &str, candidate: &str) -> bool {
    candidate.chars().any(char::is_whitespace)
        && candidate
            .chars()
            .filter(|c| !c.is_whitespace())
            .eq(baseline.chars())
}

fn postprocess(kind: RecKind, mut text: String) -> String {
    text = match kind {
        RecKind::Table => text::handle_table(&text),
        RecKind::Formula => text::handle_formula(&text),
        _ => text::handle_text(&text),
    };
    text = text::truncate_repetitive_content(&text);
    text = text::normalize_math_delimiters(&text);
    if kind == RecKind::FormulaNumber && text.contains('$') {
        text = text.replace('$', "");
    }
    if kind == RecKind::Table {
        let html = text::convert_otsl_to_html(&text);
        if !html.is_empty() {
            text = html;
        }
    }
    text
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RecKind {
    Text,
    Formula,
    Table,
    FormulaNumber,
    Skip,
}

pub(super) fn rec_kind(base: &str) -> RecKind {
    if IMAGE_LABELS.contains(&base) || IGNORE_LABELS.contains(&base) {
        RecKind::Skip
    } else if base.contains("table") {
        RecKind::Table
    } else if base == "formula_number" {
        RecKind::FormulaNumber
    } else if base.contains("formula") {
        RecKind::Formula
    } else {
        RecKind::Text
    }
}

fn role_for(base: &str) -> BlockRole {
    match base {
        "doc_title" => BlockRole::DocTitle,
        "paragraph_title" => BlockRole::SectionTitle,
        "figure_title" => BlockRole::Caption,
        _ => BlockRole::Body,
    }
}

pub(super) fn to_doc_block(base: &str, coord: [f32; 4], text: &str) -> Option<Block> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let kind = match rec_kind(base) {
        RecKind::Formula => BlockKind::Formula,
        RecKind::Table => BlockKind::Table,
        _ => BlockKind::Text,
    };
    let mut block = Block::new(kind, bbox_to_rect(coord), text).with_role(role_for(base));
    if kind == BlockKind::Formula {
        let (body, from_dollars) = crate::math::unwrap_formula(&block.text);
        if body.is_empty() {
            return None;
        }
        block.text = body;
        block.display = from_dollars
            || base.contains("display_formula")
            || crate::math::is_display_body(&block.text);
    } else if kind == BlockKind::Text {
        block.text = crate::math::canonicalize_mixed_text(&block.text);
        if block.text.is_empty() {
            return None;
        }
    }
    block.promote_html_table();
    Some(block)
}

fn region_area(coord: [f32; 4]) -> f32 {
    layout::bbox_area(&coord).max(1.0)
}

#[derive(Default)]
struct ConfAcc {
    token_sum: f32,
    token_n: usize,
    layout_wsum: f32,
    layout_w: f32,
}

impl ConfAcc {
    fn add(&mut self, p_sum: f32, p_n: usize, score: f32, coord: [f32; 4]) {
        self.token_sum += p_sum;
        self.token_n += p_n;
        let area = region_area(coord);
        self.layout_wsum += score * area;
        self.layout_w += area;
    }

    fn finish(self) -> Option<f32> {
        let token = (self.token_n > 0).then(|| self.token_sum / self.token_n as f32);
        let layout = (self.layout_w > 0.0).then(|| self.layout_wsum / self.layout_w);
        mix_confidence(token, layout)
    }
}

const TOKEN_CONF_WEIGHT: f32 = 0.8;
const LAYOUT_CONF_WEIGHT: f32 = 0.2;
const FORMULA_NUMBER_MIN_SCORE: f32 = 0.65;
const FORMULA_NUMBER_Y_BAND: f32 = 0.25;
const FORMULA_NUMBER_MAX_WIDTH_RATIO: f32 = 0.20;
const FORMULA_NUMBER_MAX_OVERLAP_RATIO: f32 = 0.70;

pub(super) fn mix_confidence(token: Option<f32>, layout: Option<f32>) -> Option<f32> {
    match (token, layout) {
        (Some(t), Some(l)) => {
            Some((TOKEN_CONF_WEIGHT * t + LAYOUT_CONF_WEIGHT * l).clamp(0.0, 1.0))
        }
        (Some(t), None) => Some(t.clamp(0.0, 1.0)),
        (None, Some(l)) => Some(l.clamp(0.0, 1.0)),
        (None, None) => None,
    }
}

struct PendingRec {
    base: String,
    coord: [f32; 4],
    text: String,
}

fn pair_formula_numbers(regions: &[Region]) -> HashMap<usize, usize> {
    let formulas: Vec<usize> = regions
        .iter()
        .enumerate()
        .filter(|(_, r)| text::base_label(&r.label) == "display_formula")
        .map(|(i, _)| i)
        .collect();
    let mut pairs = HashMap::new();
    for (i, r) in regions.iter().enumerate() {
        if text::base_label(&r.label) != "formula_number" {
            continue;
        }
        if r.score < FORMULA_NUMBER_MIN_SCORE {
            continue;
        }
        let yc = (r.coord[1] + r.coord[3]) * 0.5;
        let nw = (r.coord[2] - r.coord[0]).max(1.0);
        let nxc = (r.coord[0] + r.coord[2]) * 0.5;
        let mut best: Option<(usize, f32)> = None;
        for &j in &formulas {
            let f = &regions[j].coord;
            let fw = (f[2] - f[0]).max(1.0);
            if nw > fw * FORMULA_NUMBER_MAX_WIDTH_RATIO {
                continue;
            }
            let band = (f[3] - f[1]) * FORMULA_NUMBER_Y_BAND;
            if yc < f[1] - band || yc > f[3] + band {
                continue;
            }
            let overlap = (f[2].min(r.coord[2]) - f[0].max(r.coord[0])).max(0.0);
            if overlap / nw > FORMULA_NUMBER_MAX_OVERLAP_RATIO {
                continue;
            }
            let separation = if nxc >= (f[0] + f[2]) * 0.5 {
                (r.coord[0] - f[2]).max(0.0)
            } else {
                (f[0] - r.coord[2]).max(0.0)
            };
            let rank = separation + overlap * 0.25;
            if best.is_none_or(|(_, previous)| rank < previous) {
                best = Some((j, rank));
            }
        }
        if let Some((j, _)) = best {
            pairs.insert(i, j);
        }
    }
    pairs
}

/// `(11)` / `$$(11)$$` → `11`. None if the crop is not a tag.
fn clean_tag(text: &str) -> Option<String> {
    let mut t = text.trim().to_string();
    for pat in ["$$", "\\(", "\\)", "\\[", "\\]", "\\\\", "$"] {
        t = t.replace(pat, "");
    }
    let t = t
        .trim()
        .trim_matches(|c| c == '(' || c == ')' || c == '[' || c == ']')
        .trim()
        .to_string();
    text::eqno_payload(&t)
}

fn bbox_to_rect(coord: [f32; 4]) -> Rect {
    let x0 = coord[0].max(0.0).round() as u32;
    let y0 = coord[1].max(0.0).round() as u32;
    let x1 = coord[2].max(0.0).round() as u32;
    let y1 = coord[3].max(0.0).round() as u32;
    Rect::from_points(x0, y0, x1, y1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(label: &str, coord: [f32; 4]) -> Region {
        Region {
            label: label.into(),
            coord,
            img: None,
            score: 1.0,
        }
    }

    #[test]
    fn rec_kind_classifies_labels() {
        assert_eq!(rec_kind("text"), RecKind::Text);
        assert_eq!(rec_kind("doc_title"), RecKind::Text);
        assert_eq!(rec_kind("display_formula"), RecKind::Formula);
        assert_eq!(rec_kind("inline_formula"), RecKind::Formula);
        assert_eq!(rec_kind("formula_number"), RecKind::FormulaNumber);
        assert_eq!(rec_kind("table"), RecKind::Table);
        assert_eq!(rec_kind("image"), RecKind::Skip);
        assert_eq!(rec_kind("header"), RecKind::Skip);
    }

    #[test]
    fn mix_confidence_weights() {
        assert_eq!(mix_confidence(Some(1.0), None), Some(1.0));
        assert_eq!(mix_confidence(None, Some(0.4)), Some(0.4));
        assert!((mix_confidence(Some(1.0), Some(0.0)).unwrap() - TOKEN_CONF_WEIGHT).abs() < 1e-6);
        assert_eq!(mix_confidence(Some(0.5), Some(0.5)), Some(0.5));
        assert_eq!(mix_confidence(None, None), None);
    }

    #[test]
    fn space_retry_is_narrow_and_content_preserving() {
        let crop = RgbImg::blank(1650, 91, 255);
        let padded = imgops::pad_to_unirec_height_128(&crop);
        assert_eq!((padded.w, padded.h), (1650, 220));
        assert!(looks_like_dropped_english_spaces(
            &crop,
            "Itaddressesstructuralvariabilityandsemanticentanglement"
        ));
        assert!(!looks_like_dropped_english_spaces(
            &crop,
            "It addresses structural variability and semantic entanglement"
        ));
        assert!(only_adds_whitespace(
            "Itaddressesstructuralvariability",
            "It addresses structural variability"
        ));
        assert!(!only_adds_whitespace(
            "Itaddressesstructuralvariability",
            "It addresses structure variability"
        ));
    }

    #[test]
    fn pair_formula_numbers_same_line() {
        let regions = vec![
            region("display_formula_01", [10.0, 10.0, 110.0, 40.0]),
            region("formula_number_02", [120.0, 14.0, 138.0, 36.0]),
            region("formula_number_03", [120.0, 80.0, 138.0, 100.0]),
        ];
        let pairs = pair_formula_numbers(&regions);
        assert_eq!(pairs.get(&1), Some(&0));
        assert!(
            !pairs.contains_key(&2),
            "below-band number must stay unpaired"
        );
    }

    #[test]
    fn clean_tag_accepts_parenthesized_digits() {
        assert_eq!(clean_tag("$$(11)$$").as_deref(), Some("11"));
        assert_eq!(clean_tag("(2.1)").as_deref(), Some("2.1"));
        assert_eq!(clean_tag("hello"), None);
    }

    #[test]
    fn inject_tags_then_unwrap_yields_tag() {
        let wrapped = text::handle_formula("a+b");
        let tagged = text::inject_tags(&wrapped, &["1".into()]);
        let block =
            to_doc_block("display_formula", [0.0, 0.0, 10.0, 10.0], &tagged).expect("formula");
        assert_eq!(block.text, r"a+b \tag{1}");
        assert!(block.display);
    }

    #[test]
    fn layout_tag_replaces_unverified_inline_number() {
        let wrapped = text::handle_formula("\\[a+b\\] (1)\n\n");
        assert!(!wrapped.contains(r"\tag{"));
        let cleaned = text::remove_unverified_formula_tags(&wrapped);
        let tagged = text::inject_tags(&cleaned, &["1".into()]);
        assert_eq!(
            tagged.matches(r"\tag{1}").count(),
            1,
            "layout pairing should be the only source of the eqno, got {tagged:?}"
        );
    }

    #[test]
    fn inject_tags_does_not_treat_tag11_as_tag1() {
        let wrapped = "$$a \\tag{11}$$\n\n";
        let tagged = text::inject_tags(wrapped, &["1".into()]);
        let core = tagged.trim_end();
        let body = core.strip_suffix("$$").expect("display wrap");
        let inner = body.strip_prefix("$$").unwrap_or(body);
        let (_, tags) = crate::math::split_display_tag(inner);
        assert_eq!(tags, vec!["11".to_string(), "1".to_string()]);
    }
}
