use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use image::RgbaImage;

use crate::identity::{models_dir, APP_SLUG};

mod imgops;
mod layout;
mod pipeline;
mod text;
mod unirec;

use imgops::RgbImg;
use pipeline::{Pipeline, SHIP_FILES};

pub use pipeline::OcrResult;

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

    pub fn recognize(&self, image: &RgbaImage) -> Result<OcrResult> {
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
    let intra = default_intra();
    eprintln!("{APP_SLUG}: loading OpenDoc ship ({intra} intra-op threads)");
    let pipeline = Pipeline::load(dir, intra)?;
    eprintln!("{APP_SLUG}: loaded PP-DocLayoutV2 + UniRec-0.1B");
    Ok(pipeline)
}

/// ORT binds thread pools at session commit. Default: at most half the
/// cores, clamped to [2, 4] — decode is bandwidth-bound; 4 vs 8 costs ~5%
/// latency and keeps the desktop responsive. Override with
/// `LOCALTEX_INTRA_THREADS` (or `OPENDOC_INTRA_THREADS`).
fn default_intra() -> usize {
    for key in ["LOCALTEX_INTRA_THREADS", "OPENDOC_INTRA_THREADS"] {
        if let Ok(v) = std::env::var(key) {
            if let Ok(n) = v.parse::<usize>() {
                return n.max(1);
            }
        }
    }
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cores / 2).clamp(2, 4)
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
    use crate::math::unwrap_formula;
    use pipeline::{is_formula, to_doc_block};

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
        assert!(formula.display);
        let numbered = to_doc_block(
            "display_formula",
            [0.0, 0.0, 10.0, 10.0],
            "$$a+b$$ (1)",
        )
        .expect("numbered");
        assert_eq!(numbered.text, r"a+b \tag{1}");
        assert!(numbered.display);
        let inline = to_doc_block("inline_formula", [0.0, 0.0, 10.0, 10.0], "$x$").expect("inline");
        assert_eq!(inline.text, "x");
        assert!(!inline.display);
        let text = to_doc_block("text", [0.0, 0.0, 10.0, 10.0], "hello").expect("text");
        assert_eq!(text.kind, BlockKind::Text);
        assert_eq!(text.text, "hello");
        let table = to_doc_block(
            "table",
            [0.0, 0.0, 10.0, 10.0],
            "<table><tr><td>a</td></tr></table>",
        )
        .expect("table");
        assert_eq!(table.kind, BlockKind::Table);
        assert!(to_doc_block("text", [0.0, 0.0, 10.0, 10.0], "  ").is_none());
    }

    #[test]
    fn strip_display_wrappers() {
        assert_eq!(unwrap_formula("$$a+b$$\n\n").0, "a+b");
        assert_eq!(unwrap_formula("$x$").0, "x");
        assert_eq!(unwrap_formula("$$a+b$$ (1)").0, r"a+b \tag{1}");
        assert_eq!(unwrap_formula("$$a+b$$\n(2.1)").0, r"a+b \tag{2.1}");
    }

    #[test]
    fn handle_formula_folds_bracket_eqno_as_tag_not_linebreak() {
        let out = text::handle_formula(
            r"\[{\rm ACC}=\frac{1}{N}I\left[\hat{y}_{i}=y_{i}\right]\] (1)

",
        );
        assert!(
            out.contains(r"\tag{1}"),
            "expected \\tag command, got {out:?}"
        );
        assert!(
            !out.contains(r"\\tag{"),
            "\\tag is a line-break plus 'tag', got {out:?}"
        );
        assert!(
            !out.ends_with('\\'),
            "trailing \\\\ after \\tag breaks display math, got {out:?}"
        );
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
