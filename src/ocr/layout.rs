//! PP-DocLayoutV2 layout detection: preprocess, postprocess, and overlap
//! filtering. Faithful port of openocr/tools/infer_doc_onnx.py
//! (LayoutDetectorONNX) and openocr/tools/utils/opendoc_onnx_utils/utils.py
//! (filter_overlap_boxes).

use anyhow::{anyhow, Result};
use fast_image_resize::FilterType;
use ort::session::Session;
use ort::value::Tensor;

use super::imgops::{self, RgbImg};

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

    // Resize original to exactly 800x800 (aspect-distorting, cv2 INTER_LINEAR).
    let resized = imgops::resize(image, 800, 800, FilterType::Bilinear)?;

    // NCHW f32, /255 only.
    const SIDE: usize = 800;
    const PLANE: usize = SIDE * SIDE;
    let mut blob = vec![0f32; 3 * PLANE];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let o = (y * SIDE + x) * 3;
            let i = y * SIDE + x;
            blob[i] = resized.data[o] as f32 / 255.0;
            blob[PLANE + i] = resized.data[o + 1] as f32 / 255.0;
            blob[2 * PLANE + i] = resized.data[o + 2] as f32 / 255.0;
        }
    }

    let image_t = Tensor::from_array((vec![1i64, 3, 800, 800], blob))?;
    if session.inputs().iter().any(|i| i.name() == "im_shape") {
        return Err(anyhow!(
            "layout.onnx is not the freeze-fold ship (unexpected im_shape input)"
        ));
    }
    let out_name = session
        .outputs()
        .first()
        .ok_or_else(|| anyhow!("layout model has no outputs"))?
        .name()
        .to_string();
    let outputs = session.run(ort::inputs! { "image" => image_t })?;

    let (shape, data) = outputs[out_name.as_str()].try_extract_tensor::<f32>()?;
    // V2 freeze-fold: [N,8] = (label, score, x1, y1, x2, y2, order, unused) in 800-space.
    if shape.len() != 2 || shape[1] != 8 {
        return Err(anyhow!("unexpected layout output shape {:?}", shape));
    }
    let n = shape[0] as usize;

    let mut boxes: Vec<DetBox> = Vec::new();
    for i in 0..n {
        let row = &data[i * 8..(i + 1) * 8];
        let score = row[1];
        if score <= threshold {
            continue;
        }
        let class_id = row[0] as i64;
        let label = LABEL_MAP
            .get(class_id as usize)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("class_{}", class_id));
        let x1 = row[2] * orig_w / 800.0;
        let y1 = row[3] * orig_h / 800.0;
        let x2 = row[4] * orig_w / 800.0;
        let y2 = row[5] * orig_h / 800.0;
        let coord = [
            x1.clamp(0.0, orig_w),
            y1.clamp(0.0, orig_h),
            x2.clamp(0.0, orig_w),
            y2.clamp(0.0, orig_h),
        ];
        boxes.push(DetBox {
            label,
            coord,
            order: row[6],
            score,
        });
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
