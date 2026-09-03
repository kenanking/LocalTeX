use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

use crate::identity::{models_dir, APP_SLUG};

mod imgops;
pub(crate) mod inktex;
mod layout;
mod model_info;
mod onnx_meta;
mod pipeline;
mod text;
mod unirec;

use crate::doc::{Block, BlockKind, OcrMeta, Rect};
use imgops::RgbImg;
use inktex::{InkTex, INK_FILES};
use pipeline::{Pipeline, OPENDOC_FILES};

use model_info::{inspect_stamps, FileStamp, PackReport};
#[cfg(test)]
use model_info::{reconcile_pack, ModelManifestState};
pub use model_info::{ModelInfo, ModelRuntimeState};
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

struct Sessions {
    page: Option<Pipeline>,
    ink: Option<InkTex>,
}

/// OpenDoc and inktex sessions load on first use.
pub struct Engine {
    dir: PathBuf,
    model_info: Mutex<ModelInfo>,
    inner: Mutex<Sessions>,
    opendoc_ok: bool,
    ink_ok: bool,
    usage_generation: AtomicU64,
}

impl Engine {
    pub fn load() -> Arc<Self> {
        let dir = models_dir();
        let model_info = ModelInfo::read(dir.clone());
        let opendoc_ok = model_info.opendoc_available();
        let ink_ok = model_info.handwriting_available();
        if opendoc_ok {
            eprintln!(
                "{APP_SLUG}: OpenDoc weights deferred until first snip ({})",
                opendoc_dir(&dir).display()
            );
        } else {
            eprintln!(
                "{APP_SLUG}: OpenDoc models missing in {}",
                opendoc_dir(&dir).display()
            );
        }
        if ink_ok {
            eprintln!(
                "{APP_SLUG}: handwriting weights deferred until first drawing ({})",
                handwriting_dir(&dir).display()
            );
        } else {
            eprintln!(
                "{APP_SLUG}: handwriting models missing in {}",
                handwriting_dir(&dir).display()
            );
        }
        Arc::new(Self {
            dir,
            model_info: Mutex::new(model_info),
            inner: Mutex::new(Sessions {
                page: None,
                ink: None,
            }),
            opendoc_ok,
            ink_ok,
            usage_generation: AtomicU64::new(0),
        })
    }

    pub fn status(&self) -> EngineStatus {
        if self.opendoc_ok {
            EngineStatus::Ready
        } else {
            EngineStatus::MissingModels {
                dir: self.dir.clone(),
            }
        }
    }

    pub fn model_info(&self) -> ModelInfo {
        self.model_info.lock().expect("model info mutex").clone()
    }

    /// Read ONNX `metadata_props` from disk. Does not open an ORT session.
    pub fn inspect_file_metadata(&self) {
        if self.opendoc_ok {
            let pack = opendoc_dir(&self.dir);
            let metadata = onnx_meta::inspect_onnx_files(&[
                (&pack.join(pipeline::LAYOUT_ONNX), "layout"),
                (&pack.join(pipeline::ENCODER_ONNX), "unirec_encoder"),
                (&pack.join(pipeline::DECODER_ONNX), "unirec_decoder"),
            ]);
            self.model_info
                .lock()
                .expect("model info mutex")
                .set_opendoc_metadata(metadata);
        }
        if self.ink_ok {
            let pack = handwriting_dir(&self.dir);
            let metadata = onnx_meta::inspect_onnx_files(&[
                (&pack.join(inktex::ENCODER_ONNX), "inktex_encoder"),
                (&pack.join(inktex::DECODER_ONNX), "inktex_decoder_step"),
            ]);
            self.model_info
                .lock()
                .expect("model info mutex")
                .set_handwriting_metadata(metadata);
        }
    }

    pub fn recognize(&self, image: &RgbaImage) -> Result<OcrResult> {
        if !self.opendoc_ok {
            bail!(
                "OpenDoc models missing in {}",
                opendoc_dir(&self.dir).display()
            );
        }
        self.usage_generation.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.inner.lock().expect("ocr mutex");
        if guard.page.is_none() {
            let pipeline = load_pipeline(&self.dir)?;
            self.model_info
                .lock()
                .expect("model info mutex")
                .set_opendoc_metadata(pipeline.pack_metadata().clone());
            guard.page = Some(pipeline);
        }
        let pipeline = guard.page.as_mut().expect("ocr just loaded");
        let mut rgb = rgba_to_rgb(image)?;
        let result = pipeline.infer(&mut rgb);
        drop(rgb);
        drop(guard);
        trim_process_heap();
        result
    }

    pub fn recognize_ink(
        &self,
        traces: &[Vec<[f32; 3]>],
        image_size: (u32, u32),
    ) -> Result<OcrResult> {
        if !self.ink_ok {
            bail!(
                "Handwriting models missing in {}",
                handwriting_dir(&self.dir).display()
            );
        }
        self.usage_generation.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.inner.lock().expect("ocr mutex");
        if guard.ink.is_none() {
            let ink = load_ink(&self.dir)?;
            self.model_info
                .lock()
                .expect("model info mutex")
                .set_handwriting_metadata(ink.pack_metadata().clone());
            guard.ink = Some(ink);
        }
        let ink = guard.ink.as_mut().expect("inktex just loaded");
        let mut traces = traces.to_vec();
        inktex::deburst(&mut traces);
        let result = ink.recognize(&traces).map(|out| {
            ink_formula_result(out.text, image_size, (out.encode_s + out.decode_s) as f32)
        });
        drop(traces);
        drop(guard);
        trim_process_heap();
        result
    }

    #[cfg(test)]
    fn sessions_are_empty(&self) -> bool {
        let guard = self.inner.lock().expect("ocr mutex");
        guard.page.is_none() && guard.ink.is_none()
    }

    pub fn usage_generation(&self) -> u64 {
        self.usage_generation.load(Ordering::Relaxed)
    }

    /// Drop lazily loaded sessions only if no newer inference started.
    /// Session destruction can be expensive, so callers run this off the UI thread.
    pub fn release_if_idle(&self, generation: u64) -> bool {
        if self.usage_generation() != generation {
            return false;
        }
        let mut guard = self.inner.lock().expect("ocr mutex");
        if self.usage_generation() != generation {
            return false;
        }
        let loaded = guard.page.is_some() || guard.ink.is_some();
        guard.page = None;
        guard.ink = None;
        drop(guard);
        if loaded {
            trim_process_heap();
        }
        loaded
    }
}

/// Return allocator pages made idle by OCR to the kernel. ORT work and session
/// destruction both run off the UI thread; non-glibc targets need no analogue.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn trim_process_heap() {
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn trim_process_heap() {}

fn pack_present(dir: &Path, files: &[&str]) -> bool {
    files.iter().all(|name| dir.join(name).is_file())
}

fn ensure_ort() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        ort::init().with_name("localtex").commit();
    });
}

pub(crate) fn build_session(
    path: &Path,
    intra: usize,
    spinning: bool,
    memory_pattern: bool,
) -> Result<Session> {
    ensure_ort();
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
        .with_memory_pattern(memory_pattern)
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

pub(crate) fn inspect_onnx_pack(sessions: &[(&Session, &str)]) -> PackReport {
    let mut stamps = Vec::with_capacity(sessions.len());
    for (session, expected) in sessions {
        let Ok(metadata) = session.metadata() else {
            return metadata_mismatch("ONNX metadata is unreadable");
        };
        stamps.push((
            FileStamp {
                schema: metadata.custom("localtex.metadata_schema"),
                pack_id: metadata.custom("localtex.pack_id"),
                component: metadata.custom("localtex.component"),
            },
            *expected,
        ));
    }
    inspect_stamps(&stamps)
}

fn metadata_mismatch(detail: &str) -> PackReport {
    PackReport {
        state: ModelRuntimeState::Mismatch,
        pack_id: None,
        detail: Some(detail.into()),
    }
}

fn opendoc_dir(models: &Path) -> PathBuf {
    models.join("opendoc")
}

fn handwriting_dir(models: &Path) -> PathBuf {
    models.join("handwriting")
}

fn load_pipeline(dir: &Path) -> Result<Pipeline> {
    let intra = default_intra();
    let pack = opendoc_dir(dir);
    eprintln!(
        "{APP_SLUG}: loading OpenDoc ({intra} intra-op threads, {})",
        pack.display()
    );
    let pipeline = Pipeline::load(&pack, intra)?;
    eprintln!("{APP_SLUG}: loaded PP-DocLayoutV2 + UniRec-0.1B");
    Ok(pipeline)
}

fn load_ink(dir: &Path) -> Result<InkTex> {
    let intra = default_intra();
    let spinning = std::env::var("LOCALTEX_SPINNING").is_ok_and(|v| v != "0");
    let ink_dir = handwriting_dir(dir);
    eprintln!(
        "{APP_SLUG}: loading handwriting inktex ({intra} intra-op threads, {})",
        ink_dir.display()
    );
    let ink = InkTex::load(&ink_dir, intra, spinning)?;
    eprintln!("{APP_SLUG}: loaded inktex encoder + decoder-step");
    Ok(ink)
}

fn ink_formula_result(text: String, (w, h): (u32, u32), elapsed_s: f32) -> OcrResult {
    let mut block = Block::new(
        BlockKind::Formula,
        Rect {
            x: 0,
            y: 0,
            w: w.max(1),
            h: h.max(1),
        },
        text,
    );
    block.display = true;
    OcrResult {
        blocks: vec![block],
        meta: Some(OcrMeta {
            elapsed_s,
            confidence: 1.0,
        }),
    }
}

/// ORT binds thread pools at session commit. Default: at most half the
/// cores, clamped to [2, 4] — decode is bandwidth-bound; 4 vs 8 costs ~5%
/// latency and keeps the desktop responsive. Override with
/// `LOCALTEX_INTRA_THREADS`.
fn default_intra() -> usize {
    if let Ok(v) = std::env::var("LOCALTEX_INTRA_THREADS") {
        if let Ok(n) = v.parse::<usize>() {
            return n.max(1);
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
    fn text_region_with_html_table_becomes_table_kind() {
        let block = to_doc_block(
            "text",
            [0.0, 0.0, 10.0, 10.0],
            "<table><tr><td>a</td></tr></table>",
        )
        .expect("block");
        assert_eq!(block.kind, BlockKind::Table);
    }

    #[test]
    fn strip_display_wrappers() {
        assert_eq!(unwrap_formula("$$a+b$$\n\n").0, "a+b");
        assert_eq!(unwrap_formula("$x$").0, "x");
        assert_eq!(unwrap_formula(r"$$a+b \tag{1}$$").0, r"a+b \tag{1}");
    }

    #[test]
    fn handle_formula_keeps_trailing_number_unverified() {
        let out = text::handle_formula(
            r"\[{\rm ACC}=\frac{1}{N}I\left[\hat{y}_{i}=y_{i}\right]\] (1)

",
        );
        assert!(
            out.starts_with("$$") && out.contains("ACC"),
            "expected $$ wrap, got {out:?}"
        );
        assert!(!out.contains(r"\tag{"), "text alone cannot prove an eqno");
        assert!(out.contains("(1)"));
    }

    #[test]
    fn handle_formula_does_not_promote_spaced_trailing_paren() {
        let out = text::handle_formula("a+b (2.1)");
        assert!(!out.contains(r"\tag{"));
        let leave = text::handle_formula("f(1)");
        assert!(
            !leave.contains(r"\tag{"),
            "f(1) is math, not an eqno, got {leave:?}"
        );
    }

    #[test]
    fn packs_live_beside_models_root() {
        let root = PathBuf::from("/models");
        assert_eq!(opendoc_dir(&root), root.join("opendoc"));
        assert_eq!(handwriting_dir(&root), root.join("handwriting"));
    }

    #[test]
    fn missing_weights_are_an_error() {
        let dir = PathBuf::from("/no/such/localtex-models");
        let engine = Engine {
            model_info: Mutex::new(ModelInfo::read(dir.clone())),
            dir,
            inner: Mutex::new(Sessions {
                page: None,
                ink: None,
            }),
            opendoc_ok: false,
            ink_ok: false,
            usage_generation: AtomicU64::new(0),
        };
        assert!(matches!(
            engine.status(),
            EngineStatus::MissingModels { .. }
        ));
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
        assert!(engine.recognize(&img).is_err());
        let traces = vec![vec![[0.0, 0.0, 0.0], [1.0, 0.0, 1.0]]];
        assert!(engine.recognize_ink(&traces, (8, 8)).is_err());
    }

    #[test]
    fn model_info_reads_pack_versions_without_loading_sessions() {
        let dir = std::env::temp_dir().join(format!("localtex-model-info-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            br#"{"packs":{"opendoc":"open-v2","handwriting":"ink-v1"}}"#,
        )
        .unwrap();

        let info = ModelInfo::read(dir);
        assert_eq!(info.manifest(), ModelManifestState::Loaded);
        assert_eq!(info.opendoc_pack(), Some("open-v2"));
        assert_eq!(info.handwriting_pack(), Some("ink-v1"));
        assert!(!info.opendoc_available());
        assert!(!info.handwriting_available());
        assert_eq!(info.opendoc_runtime(), ModelRuntimeState::Declared);
        assert_eq!(info.handwriting_runtime(), ModelRuntimeState::Declared);
    }

    #[test]
    fn present_packs_start_as_checking() {
        let dir =
            std::env::temp_dir().join(format!("localtex-model-checking-{}", std::process::id()));
        let opendoc = dir.join("opendoc");
        let ink = dir.join("handwriting");
        std::fs::create_dir_all(&opendoc).unwrap();
        std::fs::create_dir_all(&ink).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            br#"{"packs":{"opendoc":"open-v2","handwriting":"ink-v1"}}"#,
        )
        .unwrap();
        for name in OPENDOC_FILES {
            std::fs::write(opendoc.join(name), []).unwrap();
        }
        for name in INK_FILES {
            std::fs::write(ink.join(name), []).unwrap();
        }
        let info = ModelInfo::read(dir.clone());
        assert_eq!(info.opendoc_runtime(), ModelRuntimeState::Checking);
        assert_eq!(info.handwriting_runtime(), ModelRuntimeState::Checking);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn inspect_file_metadata_verifies_installed_packs_without_sessions() {
        let dir = models_dir();
        if !pack_present(&opendoc_dir(&dir), &OPENDOC_FILES)
            && !pack_present(&handwriting_dir(&dir), &INK_FILES)
        {
            return;
        }
        let engine = Engine::load();
        assert!(engine.sessions_are_empty());
        engine.inspect_file_metadata();
        assert!(engine.sessions_are_empty());
        let info = engine.model_info();
        if pack_present(&opendoc_dir(&dir), &OPENDOC_FILES) {
            assert_eq!(info.opendoc_runtime(), ModelRuntimeState::Verified);
        }
        if pack_present(&handwriting_dir(&dir), &INK_FILES) {
            assert_eq!(info.handwriting_runtime(), ModelRuntimeState::Verified);
        }
    }

    #[test]
    fn file_metadata_reads_stamped_layout_without_ort() {
        let path = models_dir().join("opendoc").join(pipeline::LAYOUT_ONNX);
        if !path.is_file() {
            return;
        }
        let map = onnx_meta::read_onnx_metadata(&path).expect("read layout metadata");
        assert_eq!(
            map.get("localtex.metadata_schema").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            map.get("localtex.component").map(String::as_str),
            Some("layout")
        );
        assert!(map.contains_key("localtex.pack_id"));
    }

    #[test]
    fn runtime_pack_must_match_manifest() {
        let verified = PackReport {
            state: ModelRuntimeState::Verified,
            pack_id: Some("open-v2".into()),
            detail: None,
        };
        assert_eq!(
            reconcile_pack(Some("open-v2"), &verified),
            ModelRuntimeState::Verified
        );
        assert_eq!(
            reconcile_pack(Some("open-v1"), &verified),
            ModelRuntimeState::Mismatch
        );
        assert_eq!(reconcile_pack(None, &verified), ModelRuntimeState::Mismatch);

        let unstamped = PackReport {
            state: ModelRuntimeState::Unstamped,
            pack_id: None,
            detail: Some("legacy".into()),
        };
        assert_eq!(
            reconcile_pack(Some("open-v1"), &unstamped),
            ModelRuntimeState::Unstamped
        );
    }

    #[test]
    fn ink_result_is_display_formula() {
        let out = ink_formula_result("a+b".into(), (64, 32), 0.01);
        assert_eq!(out.blocks.len(), 1);
        assert_eq!(out.blocks[0].kind, BlockKind::Formula);
        assert_eq!(out.blocks[0].text, "a+b");
        assert!(out.blocks[0].display);
        assert_eq!(out.blocks[0].bbox.w, 64);
        assert_eq!(out.blocks[0].bbox.h, 32);
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
        if !pack_present(&opendoc_dir(&dir), &OPENDOC_FILES) {
            return;
        }
        let engine = Engine::load();
        let img = RgbaImage::from_pixel(64, 32, image::Rgba([255, 255, 255, 255]));
        let _ = engine.recognize(&img).expect("OpenDoc recognize");
        assert_eq!(
            engine.model_info().opendoc_runtime(),
            ModelRuntimeState::Verified
        );
    }

    #[test]
    #[ignore]
    fn inktex_smoke_if_weights_exist() {
        let dir = models_dir();
        if !pack_present(&handwriting_dir(&dir), &INK_FILES) {
            return;
        }
        let engine = Engine::load();
        let traces = vec![vec![
            [10.0, 10.0, 0.0],
            [40.0, 12.0, 80.0],
            [42.0, 40.0, 160.0],
        ]];
        let out = engine
            .recognize_ink(&traces, (64, 64))
            .expect("inktex recognize");
        assert_eq!(out.blocks[0].kind, BlockKind::Formula);
        assert!(out.blocks[0].display);
        assert_eq!(
            engine.model_info().handwriting_runtime(),
            ModelRuntimeState::Verified
        );
    }

    /// End-to-end check against converted dataset samples (sidecar-format
    /// traces JSON + expected label TXT in INKTEX_E2E_DIR). Prints EM and the
    /// first mismatches for inspection.
    #[test]
    #[ignore]
    fn inktex_e2e_dataset_dir() {
        let dir = std::env::var("INKTEX_E2E_DIR").expect("INKTEX_E2E_DIR");
        let dir = std::path::Path::new(&dir);
        if !pack_present(&handwriting_dir(&models_dir()), &INK_FILES) {
            eprintln!("inktex e2e skipped: weights missing (set LOCALTEX_MODELS)");
            return;
        }
        let mut ids: Vec<std::path::PathBuf> = Vec::new();
        for entry in std::fs::read_dir(dir).expect("read e2e dir") {
            let p = entry.expect("entry").path();
            if p.extension().is_some_and(|e| e == "json") {
                ids.push(p);
            }
        }
        ids.sort();
        let engine = Engine::load();
        let mut exact = 0usize;
        let mut shown = 0usize;
        let mut total_ms = 0u128;
        for json_path in &ids {
            let id = json_path.file_stem().unwrap().to_string_lossy().to_string();
            let mut traces: Vec<Vec<[f32; 3]>> =
                serde_json::from_slice(&std::fs::read(json_path).expect("read traces"))
                    .expect("parse traces");
            if std::env::var("INKTEX_E2E_DEBURST").is_ok_and(|v| v == "1") {
                inktex::deburst(&mut traces);
            }
            let expected_path = json_path.with_extension("txt");
            let expected = std::fs::read_to_string(&expected_path)
                .unwrap_or_default()
                .trim()
                .to_string();
            let t0 = std::time::Instant::now();
            let out = engine
                .recognize_ink(&traces, (64, 64))
                .unwrap_or_else(|e| panic!("{id}: recognize failed: {e}"));
            total_ms += t0.elapsed().as_millis();
            let got = out.blocks[0].text.trim().to_string();
            if got == expected {
                exact += 1;
            } else if shown < 15 {
                eprintln!("MISMATCH {id}\n  want: {expected}\n  got : {got}");
                shown += 1;
            }
        }
        eprintln!(
            "inktex e2e: EM {exact}/{} = {:.2}% (avg {:.1} ms/sample)",
            ids.len(),
            100.0 * exact as f64 / ids.len() as f64,
            total_ms as f64 / ids.len().max(1) as f64
        );
    }
}
