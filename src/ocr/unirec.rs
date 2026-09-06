//! UniRec-0.1B encoder/decoder inference: image processor, tokenizer,
//! encoder run, and greedy autoregressive decode with KV cache fed by
//! reference (zero per-token copies of cross/past tensors).

use std::borrow::Cow;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use fast_image_resize::FilterType;
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};

use super::imgops::{self, RgbImg};
use super::text::clean_special_tokens;

type KvCache = Vec<(DynValue, DynValue)>;

const BOS: i64 = 0;
const EOS: i64 = 2;
const PAD: i64 = 1;
const MAX_LEN: usize = 2048;

// ---------------------------------------------------------------------------
// Tokenizer (decode-only; mirrors SimpleTokenizer + clean_special_tokens)

pub struct Tokenizer {
    /// None = id absent from the mapping (decodes as `<unk_{id}>`).
    id_to_token: Vec<Option<String>>,
}

impl Tokenizer {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read tokenizer mapping {}", path.display()))?;
        let v: serde_json::Value = serde_json::from_str(&raw)?;
        let vocab_size = v
            .get("vocab_size")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| anyhow!("missing vocab_size"))? as usize;
        let map = v
            .get("id_to_token")
            .and_then(|x| x.as_object())
            .ok_or_else(|| anyhow!("missing id_to_token"))?;
        let mut id_to_token = vec![None; vocab_size];
        for (k, val) in map {
            let id: usize = k.parse().with_context(|| format!("bad token id {}", k))?;
            let tok = val.as_str().unwrap_or("").to_string();
            if id >= id_to_token.len() {
                id_to_token.resize(id + 1, None);
            }
            id_to_token[id] = Some(tok);
        }
        Ok(Self { id_to_token })
    }

    pub fn decode(&self, ids: &[i64]) -> String {
        let mut text = String::new();
        for &id in ids {
            match self.id_to_token.get(id as usize).and_then(|t| t.as_ref()) {
                Some(tok) => text.push_str(tok),
                None => text.push_str(&format!("<unk_{}>", id)),
            }
        }
        text
    }
}

// ---------------------------------------------------------------------------
// Image processor (SimpleImageProcessor): max-side downscale, 64-alignment,
// bicubic resize, /255, (x-0.5)/0.5.

fn calculate_target_size(w: u32, h: u32) -> (u32, u32) {
    let max_w = 960.0f64;
    let max_h = 1408.0f64;
    let aspect = w as f64 / h as f64;
    let (nw, nh) = if w > 960 || h > 1408 {
        if (max_w / max_h) >= aspect {
            ((max_h * aspect) as i64 as u32, 1408u32)
        } else {
            (960u32, (max_w / aspect) as i64 as u32)
        }
    } else {
        (w, h)
    };
    let fw = ((nw / 64) * 64).max(64);
    let fh = ((nh / 64) * 64).max(64);
    (fw, fh)
}

/// Returns [1,3,H,W] f32 tensor data.
fn preprocess_crop(img: &RgbImg) -> Result<(Vec<i64>, Vec<f32>)> {
    let (tw, th) = calculate_target_size(img.w, img.h);
    let resized = imgops::resize(img, tw, th, FilterType::CatmullRom)?;
    let (w, h) = (resized.w as usize, resized.h as usize);
    let mut blob = vec![0f32; 3 * w * h];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 3;
            blob[y * w + x] = resized.data[o] as f32 / 127.5 - 1.0;
            blob[w * h + y * w + x] = resized.data[o + 1] as f32 / 127.5 - 1.0;
            blob[2 * w * h + y * w + x] = resized.data[o + 2] as f32 / 127.5 - 1.0;
        }
    }
    Ok((vec![1, 3, th as i64, tw as i64], blob))
}

// ---------------------------------------------------------------------------
// Encoder + decoder

pub struct UniRec {
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
    pub num_layers: usize,
    pub num_heads: usize,
    pub head_dim: usize,
    layer_names: Vec<LayerNames>,
}

struct LayerNames {
    cross_kt: String,
    cross_v: String,
    past_key: String,
    past_value: String,
    present_key: String,
    present_value: String,
}

struct EncodedImage {
    /// Per-layer cross-K: kt[l] is [1,6,128,seq] (transposed).
    pub cross_kt: Vec<DynValue>,
    /// Per-layer cross-V: v[l] is [1,6,seq,128].
    pub cross_v: Vec<DynValue>,
}

pub(super) struct RecognizeOut {
    pub text: String,
    pub encode_s: f64,
    pub decode_s: f64,
    pub decode_steps: usize,
    /// Sum of top-1 softmax over generated tokens (BOS/EOS excluded).
    pub p_sum: f32,
    pub p_n: usize,
}

impl UniRec {
    pub fn new(encoder: Session, decoder: Session, tokenizer: Tokenizer) -> Result<Self> {
        // Ship decoder: GQA + per-layer cross_kt_l / cross_v_l.
        if !decoder.inputs().iter().any(|i| i.name() == "cross_kt_0") {
            return Err(anyhow!(
                "decoder is not the GQA ship; expected input cross_kt_0"
            ));
        }
        if !decoder.inputs().iter().any(|i| i.name() == "seqlens_k") {
            return Err(anyhow!(
                "decoder is not the GQA ship; expected input seqlens_k"
            ));
        }
        let mut num_layers = 0usize;
        let mut num_heads = 0usize;
        let mut head_dim = 0usize;
        for inp in decoder.inputs() {
            if let Some(rest) = inp.name().strip_prefix("past_key_") {
                if let Ok(idx) = rest.parse::<usize>() {
                    num_layers = num_layers.max(idx + 1);
                }
                if let ort::value::ValueType::Tensor { shape, .. } = inp.dtype() {
                    // past_key: [batch, heads, dim, past]
                    if shape.len() == 4 && shape[1] > 0 {
                        num_heads = shape[1] as usize;
                    }
                }
            }
        }
        // past_key's head-dim axis is symbolic ("key_head_dim"); read the
        // static head dim from cross_kt_0 [batch, heads, 128, enc_seq].
        if let Some(inp) = decoder.inputs().iter().find(|i| i.name() == "cross_kt_0")
            && let ort::value::ValueType::Tensor { shape, .. } = inp.dtype()
            && shape.len() == 4
            && shape[2] > 0
        {
            head_dim = shape[2] as usize;
        }
        if num_layers == 0 {
            return Err(anyhow!("decoder exposes no past_key_* inputs"));
        }
        // Defensive: verify required I/O names exist.
        let has = |name: &str, inputs: bool| {
            if inputs {
                decoder.inputs().iter().any(|i| i.name() == name)
            } else {
                decoder.outputs().iter().any(|o| o.name() == name)
            }
        };
        for name in ["input_ids", "position_ids"] {
            if !has(name, true) {
                return Err(anyhow!("decoder missing input {}", name));
            }
        }
        for i in 0..num_layers {
            for name in [format!("cross_kt_{}", i), format!("cross_v_{}", i)] {
                if !has(&name, true) {
                    return Err(anyhow!("decoder missing input {}", name));
                }
            }
        }
        for i in 0..num_layers {
            for name in [format!("past_key_{}", i), format!("past_value_{}", i)] {
                if !has(&name, true) {
                    return Err(anyhow!("decoder missing input {}", name));
                }
            }
            for name in [format!("present_key_{}", i), format!("present_value_{}", i)] {
                if !has(&name, false) {
                    return Err(anyhow!("decoder missing output {}", name));
                }
            }
        }
        if !has("logits", false) {
            return Err(anyhow!("decoder missing logits output"));
        }
        for name in ["seqlens_k", "total_seq_len"] {
            if !has(name, true) {
                return Err(anyhow!("GQA decoder missing input {}", name));
            }
        }
        let layer_names = (0..num_layers)
            .map(|i| LayerNames {
                cross_kt: format!("cross_kt_{i}"),
                cross_v: format!("cross_v_{i}"),
                past_key: format!("past_key_{i}"),
                past_value: format!("past_value_{i}"),
                present_key: format!("present_key_{i}"),
                present_value: format!("present_value_{i}"),
            })
            .collect();
        Ok(Self {
            encoder,
            decoder,
            tokenizer,
            num_layers,
            num_heads,
            head_dim,
            layer_names,
        })
    }

    fn encode(&mut self, img: &RgbImg) -> Result<(EncodedImage, f64)> {
        let t0 = std::time::Instant::now();
        let (shape, blob) = preprocess_crop(img)?;
        let pixel_values = Tensor::from_array((shape, blob))?;
        let mut outputs = self
            .encoder
            .run(ort::inputs! { "pixel_values" => pixel_values })?;
        let cross_k = outputs
            .remove("cross_k")
            .ok_or_else(|| anyhow!("encoder missing cross_k output"))?;
        let cross_v = outputs
            .remove("cross_v")
            .ok_or_else(|| anyhow!("encoder missing cross_v output"))?;
        // One-time per block: split [6,1,6,seq,128] into per-layer
        // tensors; K additionally transposed to [1,6,128,seq].
        let (k_shape, k_data) = cross_k.try_extract_tensor::<f32>()?;
        let (v_shape, v_data) = cross_v.try_extract_tensor::<f32>()?;
        if k_shape.len() != 5 || v_shape.len() != 5 {
            return Err(anyhow!(
                "unexpected cross shape {:?} / {:?}",
                k_shape,
                v_shape
            ));
        }
        let layers = k_shape[0] as usize;
        let heads = k_shape[2] as usize;
        let seq = k_shape[3] as usize;
        let dim = k_shape[4] as usize;
        if layers != self.num_layers || heads != self.num_heads || dim != self.head_dim {
            return Err(anyhow!(
                "cross shape {:?} mismatches decoder config L{} H{} D{}",
                k_shape,
                self.num_layers,
                self.num_heads,
                self.head_dim
            ));
        }
        let mut kt = Vec::with_capacity(layers);
        let mut vv = Vec::with_capacity(layers);
        for l in 0..layers {
            // kt[l][0][h][d][s] = cross_k[l][0][h][s][d]
            let mut buf = vec![0f32; heads * dim * seq];
            for h in 0..heads {
                for s in 0..seq {
                    let src_base = (((l * heads + h) * seq) + s) * dim;
                    let dst_base = (h * dim) * seq + s;
                    for d in 0..dim {
                        buf[dst_base + d * seq] = k_data[src_base + d];
                    }
                }
            }
            kt.push(
                Tensor::from_array((vec![1i64, heads as i64, dim as i64, seq as i64], buf))?.into(),
            );
            // v[l] is a contiguous layer slice: [1,6,seq,128]
            let start = l * heads * seq * dim;
            let slice = v_data[start..start + heads * seq * dim].to_vec();
            vv.push(
                Tensor::from_array((vec![1i64, heads as i64, seq as i64, dim as i64], slice))?
                    .into(),
            );
        }
        let enc = EncodedImage {
            cross_kt: kt,
            cross_v: vv,
        };
        Ok((enc, t0.elapsed().as_secs_f64()))
    }

    /// One greedy decode step. Returns (next_token, new_past, step_time_s, p_top1).
    fn decode_step(
        &mut self,
        token: i64,
        past_len: usize,
        enc: &EncodedImage,
        past: &[(DynValue, DynValue)],
    ) -> Result<(i64, KvCache, f64, f32)> {
        let input_ids = Tensor::from_array((vec![1i64, 1], vec![token]))?;
        let position_ids = Tensor::from_array((vec![1i64, 1], vec![PAD + 1 + past_len as i64]))?;

        let mut inputs: Vec<(Cow<'_, str>, SessionInputValue<'_>)> = ort::inputs! {
            "input_ids" => input_ids,
            "position_ids" => position_ids,
        };
        for (i, names) in self.layer_names.iter().enumerate() {
            inputs.push((
                Cow::Borrowed(names.cross_kt.as_str()),
                (&enc.cross_kt[i]).into(),
            ));
            inputs.push((
                Cow::Borrowed(names.cross_v.as_str()),
                (&enc.cross_v[i]).into(),
            ));
        }
        for ((k, v), names) in past.iter().zip(&self.layer_names) {
            inputs.push((Cow::Borrowed(names.past_key.as_str()), k.into()));
            inputs.push((Cow::Borrowed(names.past_value.as_str()), v.into()));
        }
        // s = 1 decode step: last-key index = past, total = past + 1.
        let seqlens_k = Tensor::from_array((vec![1i64], vec![past_len as i32]))?;
        let total_seq_len = Tensor::from_array((vec![1i64], vec![past_len as i32 + 1]))?;
        inputs.push((Cow::from("seqlens_k"), seqlens_k.into()));
        inputs.push((Cow::from("total_seq_len"), total_seq_len.into()));

        let t0 = std::time::Instant::now();
        let mut outputs = self.decoder.run(inputs)?;
        let step_s = t0.elapsed().as_secs_f64();

        let (next, p_top1) = {
            let (_shape, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
            let (idx, p) = softmax_top1(logits);
            (idx as i64, p)
        };

        let mut new_past = Vec::with_capacity(self.num_layers);
        for names in &self.layer_names {
            let pk = outputs
                .remove(&names.present_key)
                .ok_or_else(|| anyhow!("missing {}", names.present_key))?;
            let pv = outputs
                .remove(&names.present_value)
                .ok_or_else(|| anyhow!("missing {}", names.present_value))?;
            new_past.push((pk, pv));
        }
        Ok((next, new_past, step_s, p_top1))
    }

    /// Full greedy generation for one block image.
    pub fn recognize(&mut self, img: &RgbImg) -> Result<RecognizeOut> {
        let (enc, encode_s) = self.encode(img)?;

        let mut past: Vec<(DynValue, DynValue)> = Vec::with_capacity(self.num_layers);
        for _ in 0..self.num_layers {
            let empty: Vec<f32> = Vec::new();
            let (h, d) = (self.num_heads as i64, self.head_dim as i64);
            // GQA past_key / past_value: [1, heads, 0, dim].
            let k: DynValue = Tensor::from_array((vec![1i64, h, 0, d], empty.clone()))?.into();
            let v: DynValue = Tensor::from_array((vec![1i64, h, 0, d], empty))?.into();
            past.push((k, v));
        }

        let mut generated: Vec<i64> = vec![BOS];
        let mut decode_s = 0.0f64;
        let mut steps = 0usize;
        let mut p_sum = 0.0f32;
        let mut p_n = 0usize;
        for step in 0..(MAX_LEN - 1) {
            let current = *generated.last().unwrap();
            let (next, new_past, dt, p) = self.decode_step(current, step, &enc, &past)?;
            past = new_past;
            decode_s += dt;
            steps += 1;
            generated.push(next);
            if next == EOS {
                break;
            }
            p_sum += p;
            p_n += 1;
        }

        let raw = self.tokenizer.decode(&generated);
        let text = clean_special_tokens(&raw);
        Ok(RecognizeOut {
            text,
            encode_s,
            decode_s,
            decode_steps: steps,
            p_sum,
            p_n,
        })
    }
}

/// Numerically stable softmax of the argmax: p = 1 / Σ exp(x − max).
pub(super) fn softmax_top1(logits: &[f32]) -> (usize, f32) {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in logits.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    if !best_v.is_finite() {
        return (best, 0.0);
    }
    let mut sum = 0.0f32;
    for &v in logits {
        sum += (v - best_v).exp();
    }
    let p = if sum > 0.0 {
        (1.0 / sum).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (best, p)
}

#[cfg(test)]
mod tests {
    use super::softmax_top1;

    #[test]
    fn softmax_top1_peaks_at_known_index() {
        let (idx, p) = softmax_top1(&[1.0, 10.0, 1.0]);
        assert_eq!(idx, 1);
        assert!(p > 0.99, "p={p}");
        let (idx, p) = softmax_top1(&[0.0, 0.0, 0.0]);
        assert_eq!(idx, 0);
        assert!((p - 1.0 / 3.0).abs() < 1e-5, "p={p}");
    }
}
