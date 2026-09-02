//! Online handwriting math (ink2tex): stroke traces → 12-dim features →
//! ONNX encoder / decoder_step greedy decode → bare LaTeX.
//!
//! Port of ocr-pipeline `inktex.rs` minus InkML parse and eval helpers.

use std::borrow::Cow;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};

pub const MAX_LEN: usize = 150;
pub const ENCODER_ONNX: &str = "encoder.onnx";
pub const DECODER_ONNX: &str = "decoder_step.onnx";
pub const VOCAB_JSON: &str = "vocab.json";
pub const INK_FILES: [&str; 3] = [ENCODER_ONNX, DECODER_ONNX, VOCAB_JSON];

/// Nominal UI frame length used to spread burst-timestamped points.
const FRAME_MS: f32 = 16.7;

/// De-burst timestamps: pointer events are processed in event-loop batches,
/// so a fast burst of points shares one `Instant`-derived millisecond while
/// the pen kept moving. The model's dynamics columns (dt/speed/acc) are
/// sensitive to that quantization (~6pt EM in simulation), so spread each
/// maximal run of equal-timestamp points evenly over one frame.
///
/// Grouping is global across strokes (order preserved); each run is spread
/// forward over `min(FRAME_MS, gap to the next distinct timestamp)` so the
/// result is strictly increasing by construction. Points with distinct
/// timestamps are untouched.
pub fn deburst(traces: &mut [Vec<[f32; 3]>]) {
    #[derive(Clone, Copy)]
    struct Pt {
        stroke: usize,
        idx: usize,
        t: f32,
    }
    let mut flat: Vec<Pt> = Vec::new();
    for (s, tr) in traces.iter().enumerate() {
        for (i, p) in tr.iter().enumerate() {
            flat.push(Pt {
                stroke: s,
                idx: i,
                t: p[2],
            });
        }
    }
    let mut g = 0;
    while g < flat.len() {
        let mut h = g + 1;
        while h < flat.len() && flat[h].t == flat[g].t {
            h += 1;
        }
        if h - g > 1 {
            let n = (h - g) as f32;
            let base = flat[g].t;
            let span = match flat.get(h) {
                Some(next) if next.t - base < FRAME_MS => next.t - base,
                _ => FRAME_MS,
            };
            for (k, r) in flat[g..h].iter().enumerate() {
                traces[r.stroke][r.idx][2] = base + (k as f32 + 1.0) / (n + 1.0) * span;
            }
        }
        g = h;
    }
}

pub mod features {
    pub const FEATURES: usize = 12;
    const EPS: f32 = 1e-6;

    /// Row-major `(N, 12)` f32 buffer; empty input yields an empty buffer.
    pub fn extract(traces: &[Vec<[f32; 3]>]) -> Vec<f32> {
        let traces: Vec<&Vec<[f32; 3]>> = traces.iter().filter(|t| !t.is_empty()).collect();
        let n: usize = traces.iter().map(|t| t.len()).sum();
        if n == 0 {
            return Vec::new();
        }

        let (mut xmin, mut ymin) = (f32::MAX, f32::MAX);
        let (mut xmax, mut ymax) = (f32::MIN, f32::MIN);
        let (mut t0, mut t1) = (f32::MAX, f32::MIN);
        for tr in &traces {
            for p in tr.iter() {
                xmin = xmin.min(p[0]);
                ymin = ymin.min(p[1]);
                xmax = xmax.max(p[0]);
                ymax = ymax.max(p[1]);
                t0 = t0.min(p[2]);
                t1 = t1.max(p[2]);
            }
        }
        let xy_range = (xmax - xmin).max(ymax - ymin) + EPS;
        let t_span = (t1 - t0) + EPS;
        let expr_ymin = 0.0f32;
        let expr_yspan = (ymax - ymin) / xy_range + EPS;

        let mut rows: Vec<[f32; FEATURES]> = Vec::with_capacity(n);
        for tr in &traces {
            let m = tr.len();
            let mut xyt = Vec::with_capacity(m);
            for p in tr.iter() {
                xyt.push([
                    (p[0] - xmin) / xy_range,
                    (p[1] - ymin) / xy_range,
                    (p[2] - t0) / t_span,
                ]);
            }
            let mut t_ymin = f32::MAX;
            let mut t_ymax = f32::MIN;
            for q in &xyt {
                t_ymin = t_ymin.min(q[1]);
                t_ymax = t_ymax.max(q[1]);
            }
            let y_center = 0.5 * (t_ymin + t_ymax);
            let y_span = t_ymax - t_ymin;
            let y_center_rel = (y_center - expr_ymin) / expr_yspan;
            let y_span_rel = y_span / expr_yspan;

            let mut dxyt = vec![[0f32; 3]; m];
            for i in 1..m {
                for c in 0..3 {
                    dxyt[i][c] = xyt[i][c] - xyt[i - 1][c];
                }
            }
            let mut dist = vec![0f32; m];
            let mut speed = vec![0f32; m];
            let mut ux = vec![0f32; m];
            let mut uy = vec![0f32; m];
            for i in 0..m {
                dist[i] = dxyt[i][0].hypot(dxyt[i][1]);
                speed[i] = if dxyt[i][2] > EPS {
                    dist[i] / dxyt[i][2]
                } else {
                    0.0
                };
                if dist[i] > EPS {
                    ux[i] = dxyt[i][0] / dist[i];
                    uy[i] = dxyt[i][1] / dist[i];
                }
            }
            let mut curve = vec![0f32; m];
            for i in 1..m {
                let cross = ux[i - 1] * uy[i] - uy[i - 1] * ux[i];
                let dot = ux[i - 1] * ux[i] + uy[i - 1] * uy[i];
                let dtheta = cross.atan2(dot);
                curve[i] = if dist[i] > EPS { dtheta / dist[i] } else { 0.0 };
            }
            let mut acc = vec![0f32; m];
            for i in 1..m {
                let dspeed = speed[i] - speed[i - 1];
                acc[i] = if dxyt[i][2] > EPS {
                    dspeed / dxyt[i][2]
                } else {
                    0.0
                };
            }

            for i in 0..m {
                rows.push([
                    xyt[i][0],
                    xyt[i][1],
                    xyt[i][2],
                    dxyt[i][0],
                    dxyt[i][1],
                    dxyt[i][2],
                    speed[i],
                    curve[i],
                    acc[i],
                    if i == 0 { 1.0 } else { 0.0 },
                    y_center_rel,
                    y_span_rel,
                ]);
            }
        }

        let mut flat = vec![0f32; n * FEATURES];
        for c in 3..=8 {
            let mut mean = 0f32;
            for r in rows.iter() {
                mean += r[c];
            }
            mean /= n as f32;
            let mut var = 0f32;
            for r in rows.iter() {
                let d = r[c] - mean;
                var += d * d;
            }
            let std = (var / n as f32).sqrt() + EPS;
            for (i, r) in rows.iter_mut().enumerate() {
                let v = ((r[c] - mean) / std).clamp(-5.0, 5.0);
                flat[i * FEATURES + c] = if v.is_finite() { v } else { 0.0 };
            }
        }
        for (i, r) in rows.iter().enumerate() {
            for c in [0usize, 1, 2, 9, 10, 11] {
                let v = r[c];
                flat[i * FEATURES + c] = if v.is_finite() { v } else { 0.0 };
            }
        }
        flat
    }
}

struct Vocab {
    id_to_token: Vec<String>,
    sos: i64,
    eos: i64,
    pad: i64,
}

impl Vocab {
    fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read vocab {}", path.display()))?;
        let data: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&raw)?;
        let mut id_to_token: Vec<String> = Vec::new();
        for (_cat, ts) in data.iter() {
            match ts {
                serde_json::Value::String(s) => id_to_token.push(s.clone()),
                serde_json::Value::Array(arr) => {
                    for t in arr {
                        id_to_token.push(t.as_str().unwrap_or("").to_string());
                    }
                }
                _ => {}
            }
        }
        let mut token_to_id = std::collections::HashMap::with_capacity(id_to_token.len());
        for (i, t) in id_to_token.iter().enumerate() {
            token_to_id.insert(t.clone(), i as i64);
        }
        let find = |key: &str| -> Result<i64> {
            token_to_id
                .get(key)
                .copied()
                .ok_or_else(|| anyhow!("vocab missing {}", key))
        };
        Ok(Self {
            sos: find("<SOS>")?,
            eos: find("<EOS>")?,
            pad: find("<PAD>")?,
            id_to_token,
        })
    }

    fn decode_ids(&self, ids: &[i64]) -> String {
        let mut out = String::new();
        for &id in ids {
            if id == self.eos {
                break;
            }
            if id == self.pad || id == self.sos {
                continue;
            }
            if let Some(t) = self.id_to_token.get(id as usize) {
                out.push_str(t);
            }
        }
        out
    }
}

pub struct InkTex {
    encoder: Session,
    decoder: Session,
    vocab: Vocab,
    num_layers: usize,
    num_heads: usize,
    head_dim: usize,
}

pub struct RecognizeOut {
    pub text: String,
    pub encode_s: f64,
    pub decode_s: f64,
}

impl InkTex {
    pub fn load(models_dir: &Path, intra_threads: usize, spinning: bool) -> Result<Self> {
        let encoder = super::build_session(
            &models_dir.join(ENCODER_ONNX),
            intra_threads,
            spinning,
            true,
        )?;
        let decoder = super::build_session(
            &models_dir.join(DECODER_ONNX),
            intra_threads,
            spinning,
            true,
        )?;
        let vocab = Vocab::load(&models_dir.join(VOCAB_JSON))?;

        let mut num_layers = 0usize;
        let (mut num_heads, mut head_dim) = (0usize, 0usize);
        for inp in decoder.inputs() {
            if inp.name() == "self_k" {
                if let ort::value::ValueType::Tensor { shape, .. } = inp.dtype() {
                    if shape.len() == 5 {
                        num_layers = shape[0] as usize;
                        num_heads = shape[2] as usize;
                        head_dim = shape[4] as usize;
                    }
                }
            }
        }
        if num_layers == 0 || num_heads == 0 || head_dim == 0 {
            bail!("decoder self_k input shape not statically known");
        }
        Ok(Self {
            encoder,
            decoder,
            vocab,
            num_layers,
            num_heads,
            head_dim,
        })
    }

    pub fn recognize(&mut self, traces: &[Vec<[f32; 3]>]) -> Result<RecognizeOut> {
        let t_start = std::time::Instant::now();
        let flat = features::extract(traces);
        let n_points = flat.len() / features::FEATURES;
        if n_points == 0 {
            bail!("empty ink");
        }
        let src =
            Tensor::from_array((vec![1i64, n_points as i64, features::FEATURES as i64], flat))?;
        let src_lengths = Tensor::from_array((vec![1i64], vec![n_points as i64]))?;

        let mut enc_out = self
            .encoder
            .run(ort::inputs! { "src" => src, "src_lengths" => src_lengths })?;
        let mem_mask = enc_out
            .remove("mem_mask")
            .ok_or_else(|| anyhow!("encoder missing mem_mask"))?;
        let mem_k = enc_out
            .remove("mem_k")
            .ok_or_else(|| anyhow!("encoder missing mem_k"))?;
        let mem_v = enc_out
            .remove("mem_v")
            .ok_or_else(|| anyhow!("encoder missing mem_v"))?;
        let encode_s = t_start.elapsed().as_secs_f64();

        let (l, h, d) = (
            self.num_layers as i64,
            self.num_heads as i64,
            self.head_dim as i64,
        );
        let mut self_k: DynValue =
            Tensor::from_array((vec![l, 1, h, 0, d], Vec::<f32>::new()))?.into();
        let mut self_v: DynValue =
            Tensor::from_array((vec![l, 1, h, 0, d], Vec::<f32>::new()))?.into();

        let mut generated: Vec<i64> = vec![self.vocab.sos];
        let mut decode_s = 0.0f64;
        for _ in 0..(MAX_LEN - 1) {
            let current = *generated.last().unwrap();
            let tgt_last = Tensor::from_array((vec![1i64, 1], vec![current]))?;
            let step = Tensor::from_array((vec![1i64], vec![(generated.len() - 1) as i64]))?;
            let inputs: Vec<(Cow<'_, str>, SessionInputValue<'_>)> = vec![
                (Cow::from("tgt_last"), tgt_last.into()),
                (Cow::from("step"), step.into()),
                (Cow::from("self_k"), (&self_k).into()),
                (Cow::from("self_v"), (&self_v).into()),
                (Cow::from("mem_k"), (&mem_k).into()),
                (Cow::from("mem_v"), (&mem_v).into()),
                (Cow::from("memory_key_padding_mask"), (&mem_mask).into()),
            ];
            let t0 = std::time::Instant::now();
            let mut out = self.decoder.run(inputs)?;
            decode_s += t0.elapsed().as_secs_f64();

            let next = {
                let (_shape, logits) = out["logits"].try_extract_tensor::<f32>()?;
                argmax(logits)
            };
            self_k = out
                .remove("self_k_out")
                .ok_or_else(|| anyhow!("missing self_k_out"))?;
            self_v = out
                .remove("self_v_out")
                .ok_or_else(|| anyhow!("missing self_v_out"))?;
            generated.push(next);
            if next == self.vocab.eos {
                break;
            }
        }

        Ok(RecognizeOut {
            text: self.vocab.decode_ids(&generated),
            encode_s,
            decode_s,
        })
    }
}

fn argmax(xs: &[f32]) -> i64 {
    let mut best = 0i64;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in xs.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best = i as i64;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_empty_is_empty() {
        assert!(features::extract(&[]).is_empty());
        assert!(features::extract(&[vec![]]).is_empty());
    }

    #[test]
    fn extract_single_point_is_start_flag() {
        let flat = features::extract(&[vec![[10.0, 20.0, 0.0]]]);
        assert_eq!(flat.len(), features::FEATURES);
        assert_eq!(flat[9], 1.0);
        assert!((flat[0] - 0.0).abs() < 1e-5);
        assert!((flat[1] - 0.0).abs() < 1e-5);
        assert!((flat[2] - 0.0).abs() < 1e-5);
    }

    #[test]
    fn extract_two_points_marks_second_not_start() {
        let flat = features::extract(&[vec![[0.0, 0.0, 0.0], [10.0, 0.0, 100.0]]]);
        assert_eq!(flat.len(), 24);
        assert_eq!(flat[9], 1.0);
        assert_eq!(flat[21], 0.0);
    }

    #[test]
    fn deburst_leaves_distinct_timestamps_alone() {
        let mut traces = vec![vec![[0.0, 0.0, 0.0], [1.0, 0.0, 8.0], [2.0, 0.0, 16.0]]];
        deburst(&mut traces);
        assert_eq!(
            traces[0].iter().map(|p| p[2]).collect::<Vec<_>>(),
            vec![0.0, 8.0, 16.0]
        );
    }

    #[test]
    fn deburst_spreads_burst_within_one_frame() {
        let mut traces = vec![vec![
            [0.0, 0.0, 100.0],
            [1.0, 0.0, 100.0],
            [2.0, 0.0, 100.0],
            [3.0, 0.0, 116.7],
        ]];
        deburst(&mut traces);
        let ts: Vec<f32> = traces[0].iter().map(|p| p[2]).collect();
        assert!(ts[0] > 100.0 && ts[0] < ts[1] && ts[1] < ts[2]);
        assert!(ts[2] < 116.7, "spread must stay before the next distinct t");
        assert!((ts[2] - 100.0 - 3.0 / 4.0 * 16.7).abs() < 1e-4);
    }

    #[test]
    fn deburst_respects_short_gap_to_next_point() {
        let mut traces = vec![vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 5.0]]];
        deburst(&mut traces);
        let ts: Vec<f32> = traces[0].iter().map(|p| p[2]).collect();
        assert!(ts[0] < ts[1] && ts[1] < 5.0, "spread must fit the 5ms gap");
    }

    #[test]
    fn deburst_groups_across_stroke_boundary_in_order() {
        let mut traces = vec![
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 100.0]],
            vec![[2.0, 0.0, 100.0], [3.0, 0.0, 116.7]],
        ];
        deburst(&mut traces);
        assert!(
            traces[0][1][2] < traces[1][0][2],
            "cross-stroke burst must keep paint order"
        );
    }

    #[test]
    fn vocab_key_order_assigns_special_ids() {
        let dir = std::env::temp_dir().join(format!("localtex-vocab-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("vocab.json"),
            r#"{"special":["<PAD>","<SOS>","<EOS>","<UNK>"],"letters":["a"]}"#,
        )
        .unwrap();
        let v = Vocab::load(&dir.join("vocab.json")).unwrap();
        assert_eq!(v.pad, 0);
        assert_eq!(v.sos, 1);
        assert_eq!(v.eos, 2);
        assert_eq!(v.decode_ids(&[1, 4, 2]), "a");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
