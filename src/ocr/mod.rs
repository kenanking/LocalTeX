use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use image::RgbaImage;

use crate::doc::Block;
use crate::identity::{models_dir, APP_SLUG};

mod imgops;
mod layout;
mod pipeline;
mod text;
mod unirec;

use imgops::RgbImg;
use pipeline::{Pipeline, SHIP_FILES};

#[derive(Debug, Clone)]
pub enum EngineStatus {
    Ready,
    MissingModels { dir: PathBuf },
}

impl EngineStatus {
    pub fn label(&self) -> String {
        match self {
            EngineStatus::Ready => "Ready".into(),
            EngineStatus::MissingModels { dir } => {
                format!("Models missing · {}", dir.display())
            }
        }
    }
}

/// OpenDoc sessions load on the first snip.
pub struct Engine {
    dir: PathBuf,
    inner: Mutex<Option<Pipeline>>,
}

impl Engine {
    pub fn load() -> Arc<Self> {
        let dir = models_dir();
        if ship_present(&dir) {
            eprintln!(
                "{APP_SLUG}: OpenDoc weights deferred until first snip ({})",
                dir.display()
            );
        } else {
            eprintln!(
                "{APP_SLUG}: OpenDoc ship models missing in {}",
                dir.display()
            );
        }
        Arc::new(Self {
            dir,
            inner: Mutex::new(None),
        })
    }

    pub fn status(&self) -> EngineStatus {
        if ship_present(&self.dir) {
            EngineStatus::Ready
        } else {
            EngineStatus::MissingModels {
                dir: self.dir.clone(),
            }
        }
    }

    pub fn recognize(&self, image: &RgbaImage) -> Result<Vec<Block>> {
        if !ship_present(&self.dir) {
            bail!("OpenDoc ship models missing in {}", self.dir.display());
        }
        let mut guard = self.inner.lock().expect("ocr mutex");
        if guard.is_none() {
            *guard = Some(load_pipeline(&self.dir)?);
        }
        let pipeline = guard.as_mut().expect("ocr just loaded");
        let mut rgb = rgba_to_rgb(image)?;
        pipeline.infer(&mut rgb)
    }
}

fn ship_present(dir: &Path) -> bool {
    SHIP_FILES.iter().all(|name| dir.join(name).is_file())
}

fn load_pipeline(dir: &Path) -> Result<Pipeline> {
    let intra = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
        .clamp(1, 8);
    eprintln!("{APP_SLUG}: loading OpenDoc ship ({intra} intra-op threads)");
    let pipeline = Pipeline::load(dir, intra)?;
    eprintln!("{APP_SLUG}: loaded PP-DocLayoutV2 + UniRec-0.1B");
    Ok(pipeline)
}

fn rgba_to_rgb(image: &RgbaImage) -> Result<RgbImg> {
    let (w, h) = image.dimensions();
    let mut data = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in image.pixels() {
        data.push(px[0]);
        data.push(px[1]);
        data.push(px[2]);
    }
    RgbImg::new(w, h, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::BlockKind;
    use pipeline::{is_formula, strip_math_wrappers, to_doc_block};

    #[test]
    fn formula_label_excludes_formula_number() {
        assert!(is_formula("display_formula"));
        assert!(is_formula("inline_formula"));
        assert!(!is_formula("formula_number"));
        assert!(!is_formula("text"));
    }

    #[test]
    fn recs_map_formula_and_skip_empty() {
        let formula =
            to_doc_block("display_formula", [0.0, 0.0, 10.0, 10.0], "$$a+b$$").expect("formula");
        assert_eq!(formula.kind, BlockKind::Formula);
        assert_eq!(formula.text, "a+b");
        let text = to_doc_block("text", [0.0, 0.0, 10.0, 10.0], "hello").expect("text");
        assert_eq!(text.kind, BlockKind::Text);
        assert_eq!(text.text, "hello");
        assert!(to_doc_block("text", [0.0, 0.0, 10.0, 10.0], "  ").is_none());
    }

    #[test]
    fn strip_display_wrappers() {
        assert_eq!(strip_math_wrappers("$$a+b$$\n\n"), "a+b");
        assert_eq!(strip_math_wrappers("$x$"), "x");
    }

    #[test]
    fn missing_weights_are_an_error() {
        let engine = Engine {
            dir: PathBuf::from("/no/such/localtex-models"),
            inner: Mutex::new(None),
        };
        assert!(matches!(
            engine.status(),
            EngineStatus::MissingModels { .. }
        ));
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
        assert!(engine.recognize(&img).is_err());
    }

    #[test]
    #[ignore]
    fn opendoc_smoke_if_weights_exist() {
        let dir = models_dir();
        if !ship_present(&dir) {
            return;
        }
        let engine = Engine::load();
        let img = RgbaImage::from_pixel(64, 32, image::Rgba([255, 255, 255, 255]));
        let _ = engine.recognize(&img).expect("OpenDoc recognize");
    }
}
