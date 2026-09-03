//! PP-DocLayoutV2 layout detection: preprocess, postprocess, and overlap
//! filtering. Faithful port of openocr/tools/infer_doc_onnx.py
//! (LayoutDetectorONNX) and openocr/tools/utils/opendoc_onnx_utils/utils.py
//! (filter_overlap_boxes).

use anyhow::{anyhow, Result};
use fast_image_resize::FilterType;
use ort::session::Session;
use ort::value::Tensor;

use super::imgops::{self, RgbImg};

const RAW_DYNAMIC_OUTPUTS: [&str; 3] = ["logits", "pred_boxes", "order_logits"];

const LABEL_MAP: [&str; 25] = [
    "abstract",
    "algorithm",
    "aside_text",
    "chart",
    "content",
    "display_formula",
    "doc_title",
    "figure_title",
    "footer",
    "footer_image",
    "footnote",
    "formula_number",
    "header",
    "header_image",
    "image",
    "inline_formula",
    "number",
    "paragraph_title",
    "reference",
    "reference_content",
    "seal",
    "table",
    "text",
    "vertical_text",
    "vision_footnote",
];

pub(super) const IMAGE_LABELS: [&str; 4] = ["image", "header_image", "footer_image", "seal"];

#[derive(Clone, Debug)]
pub struct Region {
    pub label: String,   // suffixed, e.g. "text_01"
    pub coord: [f32; 4], // x1, y1, x2, y2 in original image coords
    pub img: Option<RgbImg>,
    pub score: f32,
}

pub struct LayoutResult {
    pub regions: Vec<Region>,
    pub layout_s: f64,
}

pub(super) fn bbox_area(b: &[f32; 4]) -> f32 {
    ((b[2] - b[0]) * (b[3] - b[1])).abs()
}

/// calculate_overlap_ratio from utils.py.
fn overlap_ratio(a: &[f32; 4], b: &[f32; 4], mode: &str) -> f32 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = ix * iy;
    let area_a = bbox_area(a);
    let area_b = bbox_area(b);
    let ref_area = match mode {
        "union" => area_a + area_b - inter,
        "small" => area_a.min(area_b),
        "large" => area_a.max(area_b),
        _ => unreachable!(),
    };
    if ref_area == 0.0 {
        0.0
    } else {
        inter / ref_area
    }
}

struct DetBox {
    label: String,
    coord: [f32; 4],
    order: f32,
    score: f32,
}

fn dynamic_input_size(image: &RgbImg) -> (usize, usize) {
    if image.w as f32 / image.h.max(1) as f32 >= 4.0 {
        (320, 1280)
    } else {
        (800, 800)
    }
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-80.0, 80.0)).exp())
}

struct TensorView<'a> {
    shape: &'a [i64],
    data: &'a [f32],
}

fn raw_dynamic_boxes(
    logits: TensorView<'_>,
    boxes: TensorView<'_>,
    order: TensorView<'_>,
    original_size: (f32, f32),
) -> Result<Vec<DetBox>> {
    let (orig_h, orig_w) = original_size;
    let logits_shape = logits.shape;
    if logits_shape.len() != 3 || logits_shape[0] != 1 {
        return Err(anyhow!("unexpected dynamic logits shape {logits_shape:?}"));
    }
    let queries = logits_shape[1] as usize;
    let classes = logits_shape[2] as usize;
    if boxes.shape != [1, queries as i64, 4] || order.shape != [1, queries as i64, queries as i64] {
        return Err(anyhow!(
            "inconsistent dynamic layout shapes logits={logits_shape:?} boxes={:?} order={:?}",
            boxes.shape,
            order.shape
        ));
    }

    let mut votes = vec![0.0f32; queries];
    for (column, vote) in votes.iter_mut().enumerate() {
        for row in 0..column {
            *vote += sigmoid(order.data[row * queries + column]);
        }
        for row in (column + 1)..queries {
            *vote += 1.0 - sigmoid(order.data[column * queries + row]);
        }
    }
    let mut pointers: Vec<usize> = (0..queries).collect();
    pointers.sort_by(|&a, &b| votes[a].total_cmp(&votes[b]));
    let mut order = vec![0usize; queries];
    for (rank, query) in pointers.into_iter().enumerate() {
        order[query] = rank;
    }

    let mut ranked: Vec<(f32, usize)> = logits
        .data
        .iter()
        .enumerate()
        .map(|(index, &value)| (sigmoid(value), index))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    ranked.truncate(queries);

    Ok(ranked
        .into_iter()
        .map(|(score, flat_index)| {
            let query = flat_index / classes;
            let class_id = flat_index % classes;
            let offset = query * 4;
            let (cx, cy, width, height) = (
                boxes.data[offset],
                boxes.data[offset + 1],
                boxes.data[offset + 2],
                boxes.data[offset + 3],
            );
            DetBox {
                label: LABEL_MAP
                    .get(class_id)
                    .map(|value| (*value).to_string())
                    .unwrap_or_else(|| format!("class_{class_id}")),
                coord: [
                    (cx - width * 0.5) * orig_w,
                    (cy - height * 0.5) * orig_h,
                    (cx + width * 0.5) * orig_w,
                    (cy + height * 0.5) * orig_h,
                ],
                order: order[query] as f32,
                score,
            }
        })
        .collect())
}

/// filter_overlap_boxes from utils.py.
fn filter_overlap_boxes(boxes: Vec<DetBox>) -> Vec<DetBox> {
    let mut boxes: Vec<DetBox> = boxes
        .into_iter()
        .filter(|b| b.label != "reference")
        .collect();
    let mut dropped: Vec<bool> = vec![false; boxes.len()];
    for i in 0..boxes.len() {
        for j in (i + 1)..boxes.len() {
            if dropped[i] || dropped[j] {
                continue;
            }
            let r = overlap_ratio(&boxes[i].coord, &boxes[j].coord, "small");
            if r > 0.7 {
                let ai = bbox_area(&boxes[i].coord);
                let aj = bbox_area(&boxes[j].coord);
                if (boxes[i].label == "image" || boxes[j].label == "image")
                    && boxes[i].label != boxes[j].label
                {
                    continue;
                }
                if ai >= aj {
                    dropped[j] = true;
                } else {
                    dropped[i] = true;
                }
            }
        }
    }
    let mut out = Vec::new();
    for (i, b) in boxes.drain(..).enumerate() {
        if !dropped[i] {
            out.push(b);
        }
    }
    out
}

/// Run layout detection on the (already inverted) RGB image.
/// Returns regions sorted by reading order with crops.
pub fn detect(session: &mut Session, image: &RgbImg, threshold: f32) -> Result<LayoutResult> {
    let t0 = std::time::Instant::now();
    let orig_h = image.h as f32;
    let orig_w = image.w as f32;

    let output_names: Vec<String> = session
        .outputs()
        .iter()
        .map(|o| o.name().to_string())
        .collect();
    let raw_dynamic = RAW_DYNAMIC_OUTPUTS
        .iter()
        .all(|name| output_names.iter().any(|output| output == name));
    let (target_h, target_w) = if raw_dynamic {
        dynamic_input_size(image)
    } else {
        (800, 800)
    };
    let scale_h = target_h as f32 / orig_h;
    let scale_w = target_w as f32 / orig_w;

    let resized = imgops::resize(
        image,
        target_w as u32,
        target_h as u32,
        FilterType::Bilinear,
    )?;

    // NCHW f32, /255 only.
    let plane = target_h * target_w;
    let mut blob = vec![0f32; 3 * plane];
    for y in 0..target_h {
        for x in 0..target_w {
            let o = (y * target_w + x) * 3;
            let i = y * target_w + x;
            blob[i] = resized.data[o] as f32 / 255.0;
            blob[plane + i] = resized.data[o + 1] as f32 / 255.0;
            blob[2 * plane + i] = resized.data[o + 2] as f32 / 255.0;
        }
    }

    let image_t = Tensor::from_array((vec![1i64, 3, target_h as i64, target_w as i64], blob))?;
    let out_name = session
        .outputs()
        .first()
        .ok_or_else(|| anyhow!("layout model has no outputs"))?
        .name()
        .to_string();
    let frozen = !raw_dynamic && !session.inputs().iter().any(|i| i.name() == "im_shape");
    let outputs = if frozen || raw_dynamic {
        session.run(ort::inputs! { "image" => image_t })?
    } else {
        let im_shape = Tensor::from_array((vec![1i64, 2], vec![target_h as f32, target_w as f32]))?;
        let scale_factor = Tensor::from_array((vec![1i64, 2], vec![scale_h, scale_w]))?;
        session.run(ort::inputs! {
            "im_shape" => im_shape,
            "image" => image_t,
            "scale_factor" => scale_factor
        })?
    };

    let mut boxes = if raw_dynamic {
        let (logits_shape, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
        let (boxes_shape, raw_boxes) = outputs["pred_boxes"].try_extract_tensor::<f32>()?;
        let (order_shape, order_logits) = outputs["order_logits"].try_extract_tensor::<f32>()?;
        raw_dynamic_boxes(
            TensorView {
                shape: logits_shape,
                data: logits,
            },
            TensorView {
                shape: boxes_shape,
                data: raw_boxes,
            },
            TensorView {
                shape: order_shape,
                data: order_logits,
            },
            (orig_h, orig_w),
        )?
    } else {
        let (shape, data) = outputs[out_name.as_str()].try_extract_tensor::<f32>()?;
        if shape.len() != 2 || (shape[1] != 8 && shape[1] != 7) {
            return Err(anyhow!("unexpected layout output shape {:?}", shape));
        }
        let stride = shape[1] as usize;
        let mut boxes = Vec::new();
        for row in data.chunks_exact(stride) {
            if row[1] <= threshold {
                continue;
            }
            let class_id = row[0] as usize;
            let (mut x1, mut y1, mut x2, mut y2) = (row[2], row[3], row[4], row[5]);
            if frozen {
                x1 *= orig_w / target_w as f32;
                x2 *= orig_w / target_w as f32;
                y1 *= orig_h / target_h as f32;
                y2 *= orig_h / target_h as f32;
            }
            boxes.push(DetBox {
                label: LABEL_MAP
                    .get(class_id)
                    .map(|value| (*value).to_string())
                    .unwrap_or_else(|| format!("class_{class_id}")),
                coord: [x1, y1, x2, y2],
                order: row[6],
                score: row[1],
            });
        }
        boxes
    };

    boxes.retain(|b| b.score > threshold);
    for b in &mut boxes {
        b.coord = [
            b.coord[0].clamp(0.0, orig_w),
            b.coord[1].clamp(0.0, orig_h),
            b.coord[2].clamp(0.0, orig_w),
            b.coord[3].clamp(0.0, orig_h),
        ];
    }

    let mut boxes = filter_overlap_boxes(boxes);
    boxes.sort_by(|a, b| {
        a.order
            .partial_cmp(&b.order)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut regions: Vec<Region> = Vec::new();
    for (idx, b) in boxes.iter().enumerate() {
        let label = format!("{}_{:02}", b.label, idx + 1);
        let img = imgops::crop(image, b.coord[0], b.coord[1], b.coord[2], b.coord[3]);
        regions.push(Region {
            label,
            coord: b.coord,
            img,
            score: b.score,
        });
    }

    Ok(LayoutResult {
        regions,
        layout_s: t0.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_size_uses_wide_bucket_at_ratio_four() {
        let regular = RgbImg {
            w: 399,
            h: 100,
            data: vec![],
        };
        let wide = RgbImg {
            w: 400,
            h: 100,
            data: vec![],
        };
        assert_eq!(dynamic_input_size(&regular), (800, 800));
        assert_eq!(dynamic_input_size(&wide), (320, 1280));
    }
}
