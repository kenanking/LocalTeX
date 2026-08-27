//! Layout → per-region UniRec → LocalTeX `doc::Block`s.
//! Recognition and label postprocess follow OpenDocONNX.__call__; markdown
//! file assembly and figure-token painting are out of scope for this app.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

use super::imgops::{self, RgbImg};
use super::layout::{self, IMAGE_LABELS};
use super::text::{self, IGNORE_LABELS};
use super::unirec::{Tokenizer, UniRec};
use crate::doc::{Block, BlockKind, OcrMeta, Rect};
use crate::identity::APP_SLUG;

pub const LAYOUT_ONNX: &str = "layout.onnx";
pub const ENCODER_ONNX: &str = "encoder.onnx";
pub const DECODER_ONNX: &str = "decoder.onnx";
pub const TOKENIZER_JSON: &str = "unirec_tokenizer_mapping.json";

pub const SHIP_FILES: [&str; 4] = [LAYOUT_ONNX, ENCODER_ONNX, DECODER_ONNX, TOKENIZER_JSON];

#[derive(Clone)]
pub struct OcrResult {
    pub blocks: Vec<Block>,
    pub meta: Option<OcrMeta>,
}

pub struct Pipeline {
    layout: Session,
    unirec: UniRec,
}

fn build_session(path: &Path, intra: usize, spinning: bool) -> Result<Session> {
    // Builder errors carry the builder for recovery (not Send/Sync); stringify.
    fn e<E: std::fmt::Display>(err: E) -> anyhow::Error {
        anyhow!("{}", err)
    }
    Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(e)?
        .with_intra_threads(intra)
        .map_err(e)?
        .with_inter_threads(1)
        .map_err(e)?
        .with_parallel_execution(false)
        .map_err(e)?
        .with_flush_to_zero()
        .map_err(e)?
        .with_intra_op_spinning(spinning)
        .map_err(e)?
        .with_inter_op_spinning(spinning)
        .map_err(e)?
        .commit_from_file(path)
        .with_context(|| format!("commit session {}", path.display()))
}

fn env_flag(localtex: &str, opendoc: &str) -> bool {
    std::env::var(localtex)
        .or_else(|_| std::env::var(opendoc))
        .is_ok_and(|v| v != "0")
}

impl Pipeline {
    pub fn load(dir: &Path, intra: usize) -> Result<Self> {
        ort::init().with_name("localtex").commit();
        let spinning = env_flag("LOCALTEX_SPINNING", "OPENDOC_SPINNING");
        eprintln!(
            "{APP_SLUG}: intra_op_threads={intra} spinning={}",
            if spinning { "on" } else { "off" }
        );
        let layout = build_session(&dir.join(LAYOUT_ONNX), intra, spinning)?;
        if layout.inputs().iter().any(|i| i.name() == "im_shape") {
            return Err(anyhow!(
                "layout.onnx is not the freeze-fold ship (unexpected im_shape input)"
            ));
        }
        let encoder = build_session(&dir.join(ENCODER_ONNX), intra, spinning)?;
        let decoder = build_session(&dir.join(DECODER_ONNX), intra, spinning)?;
        let tokenizer = Tokenizer::load(&dir.join(TOKENIZER_JSON))?;
        let unirec = UniRec::new(encoder, decoder, tokenizer)?;
        eprintln!("{APP_SLUG}: layout freeze-fold · decoder GQA");
        Ok(Self { layout, unirec })
    }

    pub fn infer(&mut self, image: &mut RgbImg) -> Result<OcrResult> {
        let t0 = std::time::Instant::now();
        imgops::invert_if_dark(image);

        let layout_out = layout::detect(&mut self.layout, image, 0.5)?;
        let n_layout = layout_out.regions.len();
        let mut encode_s = 0.0f64;
        let mut decode_s = 0.0f64;
        let mut decode_steps = 0usize;
        let mut blocks = Vec::new();
        let mut conf = ConfAcc::default();

        for region in layout_out.regions {
            let base = text::base_label(&region.label);
            if IMAGE_LABELS.contains(&base) || IGNORE_LABELS.contains(&base) {
                continue;
            }
            let Some(mut crop) = region.img else {
                continue;
            };
            if is_formula(base) {
                crop = imgops::crop_margin(&crop);
            }

            let out = self.unirec.recognize(&crop)?;
            encode_s += out.encode_s;
            decode_s += out.decode_s;
            decode_steps += out.decode_steps;
            conf.add(out.p_sum, out.p_n, region.score, region.coord);
            let text = postprocess(base, out.text);
            if let Some(block) = to_doc_block(base, region.coord, &text) {
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

fn postprocess(base: &str, mut text: String) -> String {
    text = if base.contains("table") {
        text::handle_table(&text)
    } else if is_formula(base) {
        text::handle_formula(&text)
    } else {
        text::handle_text(&text)
    };
    text = text::truncate_repetitive_content(&text);
    let has_paren = text.contains("\\(") && text.contains("\\)");
    let has_bracket = text.contains("\\[") && text.contains("\\]");
    if has_paren || has_bracket {
        text = text.replace('$', "");
        text = text
            .replace("\\(", " $ ")
            .replace("\\)", " $ ")
            .replace("\\[", " $$ ")
            .replace("\\]", " $$ ");
        if base == "formula_number" {
            text = text.replace('$', "");
        }
    }
    if base.contains("table") {
        let html = text::convert_otsl_to_html(&text);
        if !html.is_empty() {
            text = html;
        }
    }
    text
}

pub(super) fn is_formula(base: &str) -> bool {
    base.contains("formula") && base != "formula_number"
}

pub(super) fn to_doc_block(base: &str, coord: [f32; 4], text: &str) -> Option<Block> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let kind = if is_formula(base) {
        BlockKind::Formula
    } else if base.contains("table") {
        BlockKind::Table
    } else {
        BlockKind::Text
    };
    let text = if kind == BlockKind::Formula {
        strip_math_wrappers(text)
    } else {
        text.to_string()
    };
    if text.is_empty() {
        return None;
    }
    Some(Block::new(kind, bbox_to_rect(coord), text))
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

/// Hybrid document confidence: 0.8 token softmax + 0.2 layout box score.
/// Missing decode tokens fall back to layout only. No evidence → None.
pub(super) fn mix_confidence(token: Option<f32>, layout: Option<f32>) -> Option<f32> {
    match (token, layout) {
        (Some(t), Some(l)) => Some((0.8 * t + 0.2 * l).clamp(0.0, 1.0)),
        (Some(t), None) => Some(t.clamp(0.0, 1.0)),
        (None, Some(l)) => Some(l.clamp(0.0, 1.0)),
        (None, None) => None,
    }
}

fn bbox_to_rect(coord: [f32; 4]) -> Rect {
    let x0 = coord[0].max(0.0).round() as u32;
    let y0 = coord[1].max(0.0).round() as u32;
    let x1 = coord[2].max(0.0).round() as u32;
    let y1 = coord[3].max(0.0).round() as u32;
    Rect::from_points(x0, y0, x1, y1)
}

pub(super) fn strip_math_wrappers(text: &str) -> String {
    let t = text.trim();
    let t = t
        .strip_prefix("$$")
        .and_then(|s| s.strip_suffix("$$"))
        .unwrap_or(t);
    let t = t
        .strip_prefix('$')
        .and_then(|s| s.strip_suffix('$'))
        .unwrap_or(t);
    t.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::mix_confidence;

    #[test]
    fn mix_confidence_weights() {
        assert_eq!(mix_confidence(Some(1.0), None), Some(1.0));
        assert_eq!(mix_confidence(None, Some(0.4)), Some(0.4));
        assert!((mix_confidence(Some(1.0), Some(0.0)).unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(mix_confidence(Some(0.5), Some(0.5)), Some(0.5));
        assert_eq!(mix_confidence(None, None), None);
    }
}
