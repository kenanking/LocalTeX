use std::path::{Path, PathBuf};

/// Window title, tray label, and user-facing product name.
pub const APP_NAME: &str = "LocalTeX";
/// Crate, binary, XDG slug, key-context, and log prefix. Must match `actions!`.
pub const APP_SLUG: &str = "localtex";
/// Freedesktop / GPUI application id.
pub const APP_ID: &str = "com.localtex.app";

const OPENDOC_MARKER: &str = "opendoc/layout.onnx";
const HANDWRITING_MARKER: &str = "handwriting/encoder.onnx";

/// Linux: `~/.config/localtex`. Windows: `%APPDATA%\localtex`.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        })
        .join(APP_SLUG)
}

/// Ship weights live on disk, never inside the binary.
///
/// Search order:
/// 1. `$LOCALTEX_MODELS` when set and non-empty (always wins, even if files are missing)
/// 2. `<exe_dir>/models` when it looks like a ship root (Windows zip / installer)
/// 3. `<exe_dir>/../share/localtex/models` when it looks like a ship root (Linux prefix / deb)
/// 4. `{data_local_dir}/localtex/models` (XDG / `%LOCALAPPDATA%`, `download-models.sh`)
pub fn models_dir() -> PathBuf {
    resolve_models_dir(
        std::env::var("LOCALTEX_MODELS").ok(),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
        data_dir(),
    )
}

/// `{data_local_dir}/{APP_SLUG}` — models, snip library, PNG files.
pub fn data_dir() -> PathBuf {
    data_local_dir().join(APP_SLUG)
}

fn data_local_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/share")
    })
}

pub(crate) fn resolve_models_dir(
    env: Option<String>,
    exe_dir: Option<PathBuf>,
    data: PathBuf,
) -> PathBuf {
    if let Some(dir) = env.filter(|s| !s.trim().is_empty()) {
        return PathBuf::from(dir);
    }
    let xdg = data.join("models");
    let mut bundled = Vec::new();
    if let Some(exe) = exe_dir {
        bundled.push(exe.join("models"));
        if let Some(prefix) = exe.parent() {
            bundled.push(prefix.join("share").join(APP_SLUG).join("models"));
        }
    }
    bundled
        .into_iter()
        .find(|path| is_ship_models_root(path))
        .unwrap_or(xdg)
}

fn is_ship_models_root(dir: &Path) -> bool {
    dir.join(OPENDOC_MARKER).is_file() || dir.join(HANDWRITING_MARKER).is_file()
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
        touch(&exe.join("models").join(OPENDOC_MARKER));
        let dest = scratch().join("from-env");
        let got = resolve_models_dir(Some(dest.to_string_lossy().into()), Some(exe), data);
        assert_eq!(got, dest);
    }

    #[test]
    fn blank_env_is_unset() {
        let data = scratch();
        let exe = scratch();
        let adjacent = exe.join("models");
        touch(&adjacent.join(OPENDOC_MARKER));
        let got = resolve_models_dir(Some("  ".into()), Some(exe), data);
        assert_eq!(got, adjacent);
    }

    #[test]
    fn prefers_models_beside_the_executable() {
        let data = scratch();
        let exe = scratch();
        let adjacent = exe.join("models");
        touch(&adjacent.join(OPENDOC_MARKER));
        touch(&data.join("models").join(OPENDOC_MARKER));
        let got = resolve_models_dir(None, Some(exe), data);
        assert_eq!(got, adjacent);
    }

    #[test]
    fn uses_prefix_share_when_adjacent_models_are_absent() {
        let data = scratch();
        let prefix = scratch();
        let exe = prefix.join("bin");
        fs::create_dir_all(&exe).unwrap();
        let share = prefix.join("share").join(APP_SLUG).join("models");
        touch(&share.join(HANDWRITING_MARKER));
        let got = resolve_models_dir(None, Some(exe), data);
        assert_eq!(got, share);
    }

    #[test]
    fn falls_back_to_xdg_when_nothing_is_bundled() {
        let data = scratch();
        let exe = scratch();
        let got = resolve_models_dir(None, Some(exe), data.clone());
        assert_eq!(got, data.join("models"));
    }
}
