//! PP-DocLayoutV2 layout detection: preprocess, postprocess, overlap
//! filtering and text-block merging. Faithful port of
//! openocr/tools/infer_doc_onnx.py (LayoutDetectorONNX) and
//! openocr/tools/utils/opendoc_onnx_utils/utils.py (filter_overlap_boxes,
//! merge_blocks).

use anyhow::{anyhow, Result};
use fast_image_resize::FilterType;
use ort::session::Session;
use ort::value::Tensor;

use super::imgops::{self, Align, RgbImg};

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

/// Labels treated as images when use_chart_recognition=True (eval default).
pub(super) const IMAGE_LABELS: [&str; 4] = ["image", "header_image", "footer_image", "seal"];

#[derive(Clone, Debug)]
pub struct Region {
    pub label: String,   // suffixed, e.g. "text_01"
    pub coord: [f32; 4], // x1, y1, x2, y2 in original image coords
    pub img: Option<RgbImg>,
}

pub struct LayoutResult {
    pub regions: Vec<Region>,
    pub layout_s: f64,
}

fn bbox_area(b: &[f32; 4]) -> f32 {
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

/// calculate_projection_overlap_ratio (default mode "union").
fn projection_overlap(a: &[f32; 4], b: &[f32; 4], horizontal: bool, mode: &str) -> f32 {
    let (s, e) = if horizontal { (1, 3) } else { (0, 2) };
    let inter = (a[e].min(b[e]) - a[s].max(b[s])).max(0.0);
    if (a[e].min(b[e]) - a[s].max(b[s])) <= 0.0 {
        return 0.0;
    }
    let ref_width = match mode {
        "union" => a[e].max(b[e]) - a[s].min(b[s]),
        "small" => (a[e] - a[s]).min(b[e] - b[s]),
        "large" => (a[e] - a[s]).max(b[e] - b[s]),
        _ => unreachable!(),
    };
    if ref_width > 0.0 {
        inter / ref_width
    } else {
        0.0
    }
}

struct DetBox {
    label: String,
    coord: [f32; 4],
    order: f32,
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
/// Returns regions sorted by reading order with crops, merged per
/// `merge_blocks` with non_merge_labels = IMAGE_LABELS + ["table"].
pub fn detect(session: &mut Session, image: &RgbImg, threshold: f32) -> Result<LayoutResult> {
    let t0 = std::time::Instant::now();
    let orig_h = image.h as f32;
    let orig_w = image.w as f32;
    let scale_h = 800.0 / orig_h;
    let scale_w = 800.0 / orig_w;

    // Resize original to exactly 800x800 (aspect-distorting, cv2 INTER_LINEAR).
    let resized = imgops::resize(image, 800, 800, FilterType::Bilinear)?;

    // NCHW f32, /255 only.
    let mut blob = vec![0f32; 3 * 800 * 800];
    for y in 0..800usize {
        for x in 0..800usize {
            let o = (y * 800 + x) * 3;
            blob[0 * 800 * 800 + y * 800 + x] = resized.data[o] as f32 / 255.0;
            blob[1 * 800 * 800 + y * 800 + x] = resized.data[o + 1] as f32 / 255.0;
            blob[2 * 800 * 800 + y * 800 + x] = resized.data[o + 2] as f32 / 255.0;
        }
    }

    let im_shape = Tensor::from_array((vec![1i64, 2], vec![800.0f32, 800.0]))?;
    let image_t = Tensor::from_array((vec![1i64, 3, 800, 800], blob))?;
    let scale_factor = Tensor::from_array((vec![1i64, 2], vec![scale_h, scale_w]))?;

    let out_name = session
        .outputs()
        .first()
        .ok_or_else(|| anyhow!("layout model has no outputs"))?
        .name()
        .to_string();
    let outputs = session.run(ort::inputs! {
        "im_shape" => im_shape,
        "image" => image_t,
        "scale_factor" => scale_factor
    })?;

    let (shape, data) = outputs[out_name.as_str()].try_extract_tensor::<f32>()?;
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
        let coord = [
            row[2].clamp(0.0, orig_w),
            row[3].clamp(0.0, orig_h),
            row[4].clamp(0.0, orig_w),
            row[5].clamp(0.0, orig_h),
        ];
        boxes.push(DetBox {
            label,
            coord,
            order: row[6],
        });
    }

    let mut boxes = filter_overlap_boxes(boxes);
    // Stable ascending sort by reading-order value.
    boxes.sort_by(|a, b| {
        a.order
            .partial_cmp(&b.order)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Suffix labels with 1-based order index, then crop.
    let mut blocks: Vec<Region> = Vec::new();
    for (idx, b) in boxes.iter().enumerate() {
        let label = format!("{}_{:02}", b.label, idx + 1);
        let img = imgops::crop(image, b.coord[0], b.coord[1], b.coord[2], b.coord[3]);
        blocks.push(Region {
            label,
            coord: b.coord,
            img,
        });
    }

    // merge_blocks with non_merge_labels = IMAGE_LABELS + ["table"]
    // (use_chart_recognition=True path of OpenDocONNX.__call__).
    let non_merge: Vec<&str> = IMAGE_LABELS.iter().copied().chain(["table"]).collect();
    let regions = merge_blocks(blocks, &non_merge);

    Ok(LayoutResult {
        regions,
        layout_s: t0.elapsed().as_secs_f64(),
    })
}

fn is_aligned(a: f32, b: f32) -> bool {
    (a - b).abs() <= 5.0
}

/// merge_blocks from utils.py, operating on full (suffixed) labels.
fn merge_blocks(blocks: Vec<Region>, non_merge_labels: &[&str]) -> Vec<Region> {
    let n = blocks.len();
    let mut blocks_to_merge: Vec<usize> = Vec::new();
    let mut non_merge_blocks: Vec<bool> = vec![false; n];
    for (idx, b) in blocks.iter().enumerate() {
        if non_merge_labels.contains(&b.label.as_str()) {
            non_merge_blocks[idx] = true;
        } else {
            blocks_to_merge.push(idx);
        }
    }

    // Grouping pass.
    let mut merged_groups: Vec<(Vec<usize>, Vec<Align>)> = Vec::new();
    let mut current_indices: Vec<usize> = Vec::new();
    let mut current_aligns: Vec<Align> = Vec::new();

    for (i, &idx) in blocks_to_merge.iter().enumerate() {
        if current_indices.is_empty() {
            current_indices = vec![idx];
            current_aligns = Vec::new();
            continue;
        }
        let prev_idx = blocks_to_merge[i - 1];
        let prev_bbox = blocks[prev_idx].coord;
        let prev_label = &blocks[prev_idx].label;
        let block_bbox = blocks[idx].coord;
        let block_label = &blocks[idx].label;

        let iou_h = projection_overlap(&block_bbox, &prev_bbox, true, "union");
        let is_cross = iou_h == 0.0
            && block_label == "text"
            && block_label == prev_label
            && block_bbox[0] > prev_bbox[2]
            && block_bbox[1] < prev_bbox[3]
            && (block_bbox[0] - prev_bbox[2])
                < (prev_bbox[2] - prev_bbox[0]).max(block_bbox[2] - block_bbox[0]) * 0.3;
        let overlaps_other = {
            let x1 = prev_bbox[0].min(block_bbox[0]);
            let y1 = prev_bbox[1].min(block_bbox[1]);
            let x2 = prev_bbox[2].max(block_bbox[2]);
            let y2 = prev_bbox[3].max(block_bbox[3]);
            let min_box = [x1, y1, x2, y2];
            let mut found = false;
            for (oidx, other) in blocks.iter().enumerate() {
                if oidx == idx || oidx == prev_idx {
                    continue;
                }
                if overlap_ratio(&min_box, &other.coord, "union") > 0.0 {
                    found = true;
                    break;
                }
            }
            found
        };
        let is_updown_align = iou_h > 0.0
            && block_label == "text"
            && block_label == prev_label
            && block_bbox[3] >= prev_bbox[1]
            && (block_bbox[1] - prev_bbox[3]).abs()
                < (prev_bbox[3] - prev_bbox[1]).max(block_bbox[3] - block_bbox[1]) * 0.5
            && (is_aligned(block_bbox[0], prev_bbox[0]) ^ is_aligned(block_bbox[2], prev_bbox[2]))
            && overlaps_other;

        if is_cross || is_updown_align {
            let align = if is_cross {
                Align::Center
            } else if is_aligned(block_bbox[0], prev_bbox[0]) {
                Align::Left
            } else if is_aligned(block_bbox[2], prev_bbox[2]) {
                Align::Right
            } else {
                Align::Center
            };
            current_indices.push(idx);
            current_aligns.push(align);
        } else {
            merged_groups.push((current_indices.clone(), current_aligns.clone()));
            current_indices = vec![idx];
            current_aligns = Vec::new();
        }
    }
    if !current_indices.is_empty() {
        merged_groups.push((current_indices, current_aligns));
    }

    // group_ranges: (start, end) = (min, max) of indices.
    let group_ranges: Vec<(usize, usize)> = merged_groups
        .iter()
        .map(|(idxs, _)| {
            (
                idxs.iter().copied().min().unwrap_or(0),
                idxs.iter().copied().max().unwrap_or(0),
            )
        })
        .collect();

    let mut result: Vec<Region> = Vec::new();
    let mut used: Vec<bool> = vec![false; n];
    let mut idx = 0usize;
    while idx < n {
        let mut group_found = false;
        for (gi, (start, end)) in group_ranges.iter().enumerate() {
            let (group_indices, aligns) = &merged_groups[gi];
            if idx == *start && group_indices.iter().all(|&i| !used[i]) {
                group_found = true;
                let imgs: Vec<&RgbImg> = group_indices
                    .iter()
                    .filter_map(|&i| blocks[i].img.as_ref())
                    .collect();
                let (w, h) = imgops::calc_merged_wh(&imgs);
                let aspect = if w != 0 {
                    h as f64 / w as f64
                } else {
                    f64::INFINITY
                };
                if aspect >= 3.0 {
                    for &bi in group_indices {
                        result.push(blocks[bi].clone());
                        used[bi] = true;
                    }
                } else {
                    let merged_img = imgops::merge_images(&imgs, aligns);
                    for (j, &bi) in group_indices.iter().enumerate() {
                        let mut b = blocks[bi].clone();
                        if j == 0 {
                            b.img = merged_img.clone();
                        } else {
                            b.img = None;
                        }
                        result.push(b);
                        used[bi] = true;
                    }
                }
                // Insert non-merge blocks strictly inside (start, end).
                for n_idx in (start + 1)..*end {
                    if non_merge_blocks[n_idx] && !used[n_idx] {
                        result.push(blocks[n_idx].clone());
                        used[n_idx] = true;
                    }
                }
                idx = end + 1;
                break;
            }
        }
        if group_found {
            continue;
        }
        if non_merge_blocks[idx] && !used[idx] {
            result.push(blocks[idx].clone());
            used[idx] = true;
        }
        idx += 1;
    }
    result
}
