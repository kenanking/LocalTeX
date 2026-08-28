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
    ship_ok: bool,
}

impl Engine {
    pub fn load() -> Arc<Self> {
        let dir = models_dir();
        let present = ship_present(&dir);
        if present {
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
            ship_ok: present,
        })
    }

    pub fn status(&self) -> EngineStatus {
        if self.ship_ok {
            EngineStatus::Ready
        } else {
            EngineStatus::MissingModels {
                dir: self.dir.clone(),
            }
        }
    }

    pub fn recognize(&self, image: &RgbaImage) -> Result<OcrResult> {
        if !self.ship_ok {
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
    use crate::doc::{BlockKind, BlockRole};
    use crate::math::unwrap_formula;
    use pipeline::{rec_kind, to_doc_block, RecKind};

    #[test]
    fn formula_label_excludes_formula_number() {
        assert_eq!(rec_kind("display_formula"), RecKind::Formula);
        assert_eq!(rec_kind("inline_formula"), RecKind::Formula);
        assert_eq!(rec_kind("formula_number"), RecKind::FormulaNumber);
        assert_eq!(rec_kind("text"), RecKind::Text);
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
            "$$a+b \\tag{1}$$\n\n",
        )
        .expect("numbered");
        assert_eq!(numbered.text, r"a+b \tag{1}");
        assert!(numbered.display);
        let inline = to_doc_block("inline_formula", [0.0, 0.0, 10.0, 10.0], "$x$").expect("inline");
        assert_eq!(inline.text, "x");
        assert!(!inline.display);
        let text = to_doc_block("text", [0.0, 0.0, 10.0, 10.0], "hello").expect("text");
        assert_eq!(text.kind, BlockKind::Text);
        assert_eq!(text.role, BlockRole::Body);
        assert_eq!(text.text, "hello");
        let title = to_doc_block("doc_title", [0.0, 0.0, 10.0, 10.0], "Intro").expect("title");
        assert_eq!(title.role, BlockRole::DocTitle);
        let section =
            to_doc_block("paragraph_title", [0.0, 0.0, 10.0, 10.0], "Method").expect("section");
        assert_eq!(section.role, BlockRole::SectionTitle);
        let cap = to_doc_block("figure_title", [0.0, 0.0, 10.0, 10.0], "Table 1").expect("cap");
        assert_eq!(cap.role, BlockRole::Caption);
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
        assert_eq!(unwrap_formula(r"$$a+b \tag{1}$$").0, r"a+b \tag{1}");
    }

    #[test]
    fn handle_formula_strips_bracket_eqno_like_opendoc() {
        let out = text::handle_formula(
            r"\[{\rm ACC}=\frac{1}{N}I\left[\hat{y}_{i}=y_{i}\right]\] (1)

",
        );
        assert!(
            out.starts_with("$$") && out.contains("ACC"),
            "expected $$ wrap, got {out:?}"
        );
        assert!(
            !out.contains("(1)"),
            "OpenDoc strips \\] (n)\\n\\n before wrap; tags come from layout pairing, got {out:?}"
        );
    }

    #[test]
    fn missing_weights_are_an_error() {
        let engine = Engine {
            dir: PathBuf::from("/no/such/localtex-models"),
            inner: Mutex::new(None),
            ship_ok: false,
        };
        assert!(matches!(
            engine.status(),
            EngineStatus::MissingModels { .. }
        ));
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
        assert!(engine.recognize(&img).is_err());
    }

    #[test]
    fn normalize_text_ligature_fullwidth() {
        assert_eq!(text::normalize_text("ﬁ"), "fi");
        assert_eq!(text::normalize_text("Ａ"), "A");
        assert_eq!(
            text::normalize_text("# heading"),
            "# heading",
            "leading # stays in Block.text; Markdown export escapes it"
        );
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
