use std::path::{Path, PathBuf};

use super::{
    INK_FILES, OPENDOC_FILES, handwriting_dir, metadata_mismatch, opendoc_dir, pack_present,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelManifestState {
    Loaded,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRuntimeState {
    Declared,
    Checking,
    Verified,
    Unstamped,
    Mismatch,
}

impl ModelRuntimeState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Declared => "Declared",
            Self::Checking => "Checking…",
            Self::Verified => "Verified",
            Self::Unstamped => "Unstamped",
            Self::Mismatch => "Mismatch",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FileStamp {
    pub(super) schema: Option<String>,
    pub(super) pack_id: Option<String>,
    pub(super) component: Option<String>,
}

pub(super) fn inspect_stamps(stamps: &[(FileStamp, &str)]) -> PackReport {
    let mut pack_id: Option<String> = None;
    let mut unstamped = 0usize;
    for (stamp, expected) in stamps {
        if stamp.schema.is_none() && stamp.pack_id.is_none() && stamp.component.is_none() {
            unstamped += 1;
            continue;
        }
        if stamp.schema.as_deref() != Some("1") {
            return metadata_mismatch("metadata schema is missing or unsupported");
        }
        if stamp.component.as_deref() != Some(*expected) {
            return metadata_mismatch("ONNX component metadata does not match its file");
        }
        let Some(current_pack) = stamp.pack_id.as_deref() else {
            return metadata_mismatch("ONNX pack metadata is incomplete");
        };
        if pack_id
            .as_deref()
            .is_some_and(|value| value != current_pack)
        {
            return metadata_mismatch("ONNX files declare different pack IDs");
        }
        pack_id = Some(current_pack.to_owned());
    }
    if unstamped == stamps.len() {
        return PackReport {
            state: ModelRuntimeState::Unstamped,
            pack_id: None,
            detail: Some("ONNX files have no LocalTeX metadata".into()),
        };
    }
    if unstamped != 0 {
        return metadata_mismatch("only part of the ONNX pack is stamped");
    }
    PackReport {
        state: ModelRuntimeState::Verified,
        pack_id,
        detail: None,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PackReport {
    pub(super) state: ModelRuntimeState,
    pub(super) pack_id: Option<String>,
    pub(super) detail: Option<String>,
}

impl ModelManifestState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Loaded => "Loaded",
            Self::Missing => "Missing",
            Self::Invalid => "Invalid",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub source: &'static str,
    dir: PathBuf,
    opendoc_pack: Option<String>,
    handwriting_pack: Option<String>,
    manifest: ModelManifestState,
    opendoc_available: bool,
    handwriting_available: bool,
    opendoc_runtime: ModelRuntimeState,
    handwriting_runtime: ModelRuntimeState,
    opendoc_observed_pack: Option<String>,
    handwriting_observed_pack: Option<String>,
    opendoc_runtime_detail: Option<String>,
    handwriting_runtime_detail: Option<String>,
}

impl ModelInfo {
    pub(super) fn read(dir: PathBuf) -> Self {
        let opendoc_available = pack_present(&opendoc_dir(&dir), &OPENDOC_FILES);
        let handwriting_available = pack_present(&handwriting_dir(&dir), &INK_FILES);
        let (manifest, opendoc_pack, handwriting_pack) =
            match std::fs::read(dir.join("manifest.json")) {
                Ok(raw) => match serde_json::from_slice::<serde_json::Value>(&raw) {
                    Ok(value) => {
                        let packs = value.get("packs");
                        let opendoc = packs
                            .and_then(|value| value.get("opendoc"))
                            .and_then(|value| value.as_str())
                            .map(str::to_owned);
                        let handwriting = packs
                            .and_then(|value| value.get("handwriting"))
                            .and_then(|value| value.as_str())
                            .map(str::to_owned);
                        let state = if opendoc.is_some() && handwriting.is_some() {
                            ModelManifestState::Loaded
                        } else {
                            ModelManifestState::Invalid
                        };
                        (state, opendoc, handwriting)
                    }
                    Err(_) => (ModelManifestState::Invalid, None, None),
                },
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    (ModelManifestState::Missing, None, None)
                }
                Err(_) => (ModelManifestState::Invalid, None, None),
            };
        let opendoc_runtime = startup_runtime(opendoc_available, opendoc_pack.is_some());
        let handwriting_runtime =
            startup_runtime(handwriting_available, handwriting_pack.is_some());
        Self {
            source: "Specified directory",
            dir,
            opendoc_pack,
            handwriting_pack,
            manifest,
            opendoc_available,
            handwriting_available,
            opendoc_runtime,
            handwriting_runtime,
            opendoc_observed_pack: None,
            handwriting_observed_pack: None,
            opendoc_runtime_detail: None,
            handwriting_runtime_detail: None,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn opendoc_pack(&self) -> Option<&str> {
        self.opendoc_pack.as_deref()
    }

    pub fn handwriting_pack(&self) -> Option<&str> {
        self.handwriting_pack.as_deref()
    }

    pub fn manifest(&self) -> ModelManifestState {
        self.manifest
    }

    pub fn opendoc_available(&self) -> bool {
        self.opendoc_available
    }

    pub fn handwriting_available(&self) -> bool {
        self.handwriting_available
    }

    pub fn opendoc_runtime(&self) -> ModelRuntimeState {
        self.opendoc_runtime
    }

    pub fn handwriting_runtime(&self) -> ModelRuntimeState {
        self.handwriting_runtime
    }

    pub fn opendoc_observed_pack(&self) -> Option<&str> {
        self.opendoc_observed_pack.as_deref()
    }

    pub fn handwriting_observed_pack(&self) -> Option<&str> {
        self.handwriting_observed_pack.as_deref()
    }

    pub fn opendoc_runtime_detail(&self) -> Option<&str> {
        self.opendoc_runtime_detail.as_deref()
    }

    pub fn handwriting_runtime_detail(&self) -> Option<&str> {
        self.handwriting_runtime_detail.as_deref()
    }

    pub(super) fn set_opendoc_metadata(&mut self, metadata: PackReport) {
        let state = reconcile_pack(self.opendoc_pack.as_deref(), &metadata);
        self.opendoc_runtime = state;
        self.opendoc_observed_pack = metadata.pack_id;
        self.opendoc_runtime_detail = metadata.detail;
    }

    pub(super) fn set_handwriting_metadata(&mut self, metadata: PackReport) {
        let state = reconcile_pack(self.handwriting_pack.as_deref(), &metadata);
        self.handwriting_runtime = state;
        self.handwriting_observed_pack = metadata.pack_id;
        self.handwriting_runtime_detail = metadata.detail;
    }
}

fn startup_runtime(available: bool, declared: bool) -> ModelRuntimeState {
    if available && !declared {
        ModelRuntimeState::Mismatch
    } else if available {
        ModelRuntimeState::Checking
    } else {
        ModelRuntimeState::Declared
    }
}

pub(super) fn reconcile_pack(declared: Option<&str>, metadata: &PackReport) -> ModelRuntimeState {
    if metadata.state != ModelRuntimeState::Verified {
        return metadata.state;
    }
    if declared == metadata.pack_id.as_deref() {
        ModelRuntimeState::Verified
    } else {
        ModelRuntimeState::Mismatch
    }
}
