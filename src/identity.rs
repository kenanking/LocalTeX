use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Window title, tray label, and user-facing product name.
pub const APP_NAME: &str = "LocalTeX";
pub const APP_SLUG: &str = "localtex";
/// Freedesktop / GPUI application id.
pub const APP_ID: &str = "com.localtex.app";

const OPENDOC_FILES: [&str; 4] = [
    "opendoc/layout.onnx",
    "opendoc/encoder.onnx",
    "opendoc/decoder.onnx",
    "opendoc/unirec_tokenizer_mapping.json",
];
const HANDWRITING_FILES: [&str; 3] = [
    "handwriting/encoder.onnx",
    "handwriting/decoder_step.onnx",
    "handwriting/vocab.json",
];

/// Linux: `~/.config/localtex`. Windows: `%APPDATA%\localtex`.
pub fn config_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(root) = std::env::var_os("LOCALTEX_TEST_ROOT") {
        return PathBuf::from(root).join("config");
    }
    dirs::config_dir()
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        })
        .join(APP_SLUG)
}

/// Search order for on-disk ship packs:
/// 1. `$LOCALTEX_MODELS` when set and non-empty (always wins, even if files are missing)
/// 2. `<exe_dir>/models`, including the parent of a test `deps` directory
/// 3. `<exe_dir>/../share/localtex/models` (Linux prefix / deb)
/// 4. `{data_local_dir}/localtex/models` (XDG / `%LOCALAPPDATA%`, `download-models.sh`)
/// 5. Registered Windows installation, then the default per-user installation
pub fn models_dir() -> PathBuf {
    models_location().path
}

pub struct ModelLocation {
    pub path: PathBuf,
    pub source: &'static str,
}

pub fn models_location() -> ModelLocation {
    resolve_models_location(
        std::env::var_os("LOCALTEX_MODELS"),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
        data_dir(),
        installed_models_dirs(),
    )
}

/// `{data_local_dir}/{APP_SLUG}` — models, snip library, PNG files.
pub fn data_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(root) = std::env::var_os("LOCALTEX_TEST_ROOT") {
        return PathBuf::from(root).join("data");
    }
    data_local_dir().join(APP_SLUG)
}

fn data_local_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/share")
    })
}

#[cfg(test)]
fn resolve_models_dir(
    env: Option<OsString>,
    exe_dir: Option<PathBuf>,
    data: PathBuf,
    installed: Vec<PathBuf>,
) -> PathBuf {
    resolve_models_location(env, exe_dir, data, installed).path
}

fn resolve_models_location(
    env: Option<OsString>,
    exe_dir: Option<PathBuf>,
    data: PathBuf,
    installed: Vec<PathBuf>,
) -> ModelLocation {
    if let Some(dir) = env.filter(|s| !s.to_string_lossy().trim().is_empty()) {
        return ModelLocation {
            path: PathBuf::from(dir),
            source: "LOCALTEX_MODELS",
        };
    }
    let xdg = data.join("models");
    let mut bundled = Vec::new();
    if let Some(exe) = exe_dir {
        bundled.push((exe.join("models"), "Beside executable"));
        if exe.file_name().is_some_and(|name| name == "deps") {
            if let Some(parent) = exe.parent() {
                bundled.push((parent.join("models"), "Test executable parent"));
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(prefix) = exe.parent() {
            bundled.push((
                prefix.join("share").join(APP_SLUG).join("models"),
                "Linux prefix",
            ));
        }
    }
    bundled.push((xdg.clone(), "User data directory"));
    bundled.extend(
        installed
            .into_iter()
            .map(|path| (path, "Windows installation")),
    );
    let mut seen = std::collections::HashSet::new();
    let mut partial = None;
    for (path, source) in bundled {
        if !seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())) {
            continue;
        }
        let page = OPENDOC_FILES.iter().all(|file| path.join(file).is_file());
        let ink = HANDWRITING_FILES
            .iter()
            .all(|file| path.join(file).is_file());
        if page && ink {
            return ModelLocation { path, source };
        }
        if (page || ink) && partial.is_none() {
            partial = Some(ModelLocation { path, source });
        }
    }
    partial.unwrap_or(ModelLocation {
        path: xdg,
        source: "User data directory",
    })
}

#[cfg(not(target_os = "windows"))]
fn installed_models_dirs() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn installed_models_dirs() -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::core::w;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::*;

    let mut paths = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let mut key = HKEY::default();
            unsafe {
                if RegOpenKeyExW(root, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{4B509438-D116-4E0A-87F7-76023183DEA0}_is1"), None, KEY_QUERY_VALUE | view, &mut key) != ERROR_SUCCESS { continue; }
                let mut bytes = 0;
                let flags = RRF_RT_REG_SZ;
                if RegGetValueW(
                    key,
                    None,
                    w!("InstallLocation"),
                    flags,
                    None,
                    None,
                    Some(&mut bytes),
                ) == ERROR_SUCCESS
                    && bytes > 2
                {
                    let mut value = vec![0u16; (bytes as usize).div_ceil(2)];
                    if RegGetValueW(
                        key,
                        None,
                        w!("InstallLocation"),
                        flags,
                        None,
                        Some(value.as_mut_ptr().cast()),
                        Some(&mut bytes),
                    ) == ERROR_SUCCESS
                    {
                        let end = value
                            .iter()
                            .position(|unit| *unit == 0)
                            .unwrap_or(value.len());
                        if end > 0 {
                            paths.push(
                                PathBuf::from(OsString::from_wide(&value[..end])).join("models"),
                            );
                        }
                    }
                }
                let _ = RegCloseKey(key);
            }
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join("Programs/LocalTeX/models"));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "localtex-models-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pack(dir: &Path, files: &[&str]) {
        for file in files {
            touch(&dir.join(file));
        }
    }
    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn env_wins_even_when_the_directory_is_empty() {
        let data = scratch();
        let exe = scratch();
        pack(&exe.join("models"), &OPENDOC_FILES);
        let dest = scratch().join("from-env");
        let got = resolve_models_dir(
            Some(dest.as_os_str().to_owned()),
            Some(exe),
            data,
            Vec::new(),
        );
        assert_eq!(got, dest);
    }

    #[test]
    fn blank_env_is_unset() {
        let data = scratch();
        let exe = scratch();
        let adjacent = exe.join("models");
        pack(&adjacent, &OPENDOC_FILES);
        let got = resolve_models_dir(Some("  ".into()), Some(exe), data, Vec::new());
        assert_eq!(got, adjacent);
    }

    #[test]
    fn prefers_models_beside_the_executable() {
        let data = scratch();
        let exe = scratch();
        let adjacent = exe.join("models");
        pack(&adjacent, &OPENDOC_FILES);
        pack(&data.join("models"), &OPENDOC_FILES);
        let got = resolve_models_dir(None, Some(exe), data, Vec::new());
        assert_eq!(got, adjacent);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn uses_prefix_share_when_adjacent_models_are_absent() {
        let data = scratch();
        let prefix = scratch();
        let exe = prefix.join("bin");
        fs::create_dir_all(&exe).unwrap();
        let share = prefix.join("share").join(APP_SLUG).join("models");
        pack(&share, &HANDWRITING_FILES);
        let got = resolve_models_dir(None, Some(exe), data, Vec::new());
        assert_eq!(got, share);
    }

    #[test]
    fn falls_back_to_xdg_when_nothing_is_bundled() {
        let data = scratch();
        let exe = scratch();
        let got = resolve_models_dir(None, Some(exe), data.clone(), Vec::new());
        assert_eq!(got, data.join("models"));
    }

    #[test]
    fn complete_install_beats_partial_adjacent_without_mixing_roots() {
        let root = scratch();
        let exe = root.join("debug/deps");
        let adjacent = root.join("debug/models");
        let installed = root.join("自定义 安装/models");
        pack(&adjacent, &OPENDOC_FILES);
        pack(&installed, &HANDWRITING_FILES);
        assert_eq!(
            resolve_models_dir(
                None,
                Some(exe.clone()),
                root.join("data"),
                vec![installed.clone()]
            ),
            adjacent
        );
        pack(&installed, &OPENDOC_FILES);
        assert_eq!(
            resolve_models_dir(
                None,
                Some(exe.clone()),
                root.join("data"),
                vec![installed.clone()]
            ),
            installed
        );
        let bad = root.join("missing override");
        assert_eq!(
            resolve_models_dir(
                Some(bad.clone().into_os_string()),
                Some(exe),
                root.join("data"),
                vec![installed]
            ),
            bad
        );
        fs::remove_dir_all(root).unwrap();
    }
}
