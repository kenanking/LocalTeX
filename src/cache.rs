//! GPU / CPU media lifetimes. `RenderImage` has no Drop in gpui 0.2 —
//! atlas tiles leak until `App::drop_image`. SVG `Image` assets need
//! `ImageSource::remove_asset`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui::{App, Image, ImageFormat, ImageSource, RenderImage};
use image::RgbaImage;
use uuid::Uuid;

use crate::imgutil;

pub const PIXEL_BUDGET: u64 = 192 * 1024 * 1024;
pub const PIXEL_MAX_ENTRIES: usize = 2;
pub const GPU_FULL_MAX: usize = 3;
pub const MATH_IMG_MAX: usize = 512;
pub const THUMB_VIEWPORT_MULT: usize = 3;
pub const ROW_HEIGHT_PX: f32 = 56.0;

pub fn pixel_bytes(img: &RgbaImage) -> u64 {
    u64::from(img.width()).saturating_mul(u64::from(img.height())) * 4
}

/// Overlay film cells are every visible snip. A sidebar viewport must not
/// drop those GPU thumbs while the gallery is open.
pub fn thumb_retain_ids(overlay_open: bool, history_keep: &[Uuid], visible: &[Uuid]) -> Vec<Uuid> {
    if overlay_open || history_keep.is_empty() {
        visible.to_vec()
    } else {
        history_keep.to_vec()
    }
}

fn svg_key(svg: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    svg.hash(&mut h);
    h.finish()
}

struct MathSlot {
    image: Arc<Image>,
}

pub struct MediaCache {
    thumbs: HashMap<Uuid, Arc<RenderImage>>,
    fulls: HashMap<Uuid, Arc<RenderImage>>,
    full_order: VecDeque<Uuid>,
    math: HashMap<u64, MathSlot>,
    math_order: VecDeque<u64>,
}

impl MediaCache {
    pub fn new() -> Self {
        Self {
            thumbs: HashMap::new(),
            fulls: HashMap::new(),
            full_order: VecDeque::new(),
            math: HashMap::new(),
            math_order: VecDeque::new(),
        }
    }

    pub fn thumb(&self, id: Uuid) -> Option<Arc<RenderImage>> {
        self.thumbs.get(&id).cloned()
    }

    pub fn full(&self, id: Uuid) -> Option<Arc<RenderImage>> {
        self.fulls.get(&id).cloned()
    }

    pub fn put_thumb(&mut self, id: Uuid, render: Arc<RenderImage>) {
        self.thumbs.entry(id).or_insert(render);
    }

    pub fn decode_thumb(jpeg: &[u8], pixels: Option<&RgbaImage>) -> Option<Arc<RenderImage>> {
        imgutil::jpeg_to_render(jpeg).or_else(|| {
            pixels.map(|p| {
                let thumb = imgutil::thumbnail(p, 56, 40);
                imgutil::rgba_to_render(&thumb)
            })
        })
    }

    pub fn ensure_full(&mut self, id: Uuid, pixels: &RgbaImage) -> Arc<RenderImage> {
        if let Some(existing) = self.fulls.get(&id) {
            return existing.clone();
        }
        let render = imgutil::gpu_display_image(pixels);
        self.fulls.insert(id, render.clone());
        self.full_order.retain(|x| *x != id);
        self.full_order.push_back(id);
        render
    }

    pub fn math_image(&mut self, svg: &str, _cx: &mut App) -> Arc<Image> {
        let key = svg_key(svg);
        if let Some(slot) = self.math.get(&key) {
            self.math_order.retain(|k| *k != key);
            self.math_order.push_back(key);
            return slot.image.clone();
        }
        let image = Arc::new(Image::from_bytes(ImageFormat::Svg, svg.as_bytes().to_vec()));
        self.math.insert(
            key,
            MathSlot {
                image: image.clone(),
            },
        );
        self.math_order.push_back(key);
        image
    }

    pub fn retain_thumbs(&mut self, keep: impl Iterator<Item = Uuid>, cx: &mut App) {
        let keep: std::collections::HashSet<Uuid> = keep.collect();
        let drop: Vec<Uuid> = self
            .thumbs
            .keys()
            .copied()
            .filter(|id| !keep.contains(id))
            .collect();
        for id in drop {
            if let Some(img) = self.thumbs.remove(&id) {
                cx.drop_image(img, None);
            }
        }
    }

    pub fn retain_fulls(
        &mut self,
        keep: impl Iterator<Item = Uuid>,
        pin: Option<Uuid>,
        cx: &mut App,
    ) {
        let keep: std::collections::HashSet<Uuid> = keep.collect();
        let drop: Vec<Uuid> = self
            .fulls
            .keys()
            .copied()
            .filter(|id| !keep.contains(id))
            .collect();
        for id in drop {
            self.drop_full(id, cx);
        }
        while self.fulls.len() > GPU_FULL_MAX {
            let extra = self.full_order.iter().copied().find(|id| Some(*id) != pin);
            let Some(id) = extra else {
                break;
            };
            self.drop_full(id, cx);
        }
    }

    pub fn trim_math(&mut self, cx: &mut App) {
        while self.math.len() > MATH_IMG_MAX {
            let Some(old) = self.math_order.pop_front() else {
                break;
            };
            self.evict_math(old, cx);
        }
    }

    fn drop_full(&mut self, id: Uuid, cx: &mut App) {
        self.full_order.retain(|x| *x != id);
        if let Some(img) = self.fulls.remove(&id) {
            cx.drop_image(img, None);
        }
    }

    fn evict_math(&mut self, key: u64, cx: &mut App) {
        if let Some(slot) = self.math.remove(&key) {
            ImageSource::Image(slot.image).remove_asset(cx);
        }
    }
}

impl Default for MediaCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_keeps_visible_thumbs_outside_history_viewport() {
        let a = Uuid::nil();
        let b = Uuid::from_u128(1);
        let visible = vec![a, b];
        let keep = thumb_retain_ids(true, &[a], &visible);
        assert!(
            keep.contains(&b),
            "film thumbs outside the sidebar viewport must survive GC"
        );
        assert_eq!(thumb_retain_ids(false, &[a], &visible), vec![a]);
        assert_eq!(thumb_retain_ids(false, &[], &visible), visible);
    }

    #[test]
    fn pixel_bytes_is_rgba() {
        let img = RgbaImage::new(10, 20);
        assert_eq!(pixel_bytes(&img), 10 * 20 * 4);
    }
}
