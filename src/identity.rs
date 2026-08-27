use std::path::PathBuf;

/// Window title, tray label, and user-facing product name.
pub const APP_NAME: &str = "LocalTeX";
/// Crate, binary, XDG slug, key-context, and log prefix. Must match `actions!`.
pub const APP_SLUG: &str = "localtex";
/// Freedesktop / GPUI application id.
pub const APP_ID: &str = "com.localtex.app";

/// `$LOCALTEX_MODELS`, else `{data_local_dir}/{APP_SLUG}/models`.
/// Linux: `~/.local/share/localtex/models`. Windows: `%LOCALAPPDATA%\localtex\models`.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        })
        .join(APP_SLUG)
}

/// `$LOCALTEX_MODELS`, else `{data_local_dir}/{APP_SLUG}/models`.
/// Linux: `~/.local/share/localtex/models`. Windows: `%LOCALAPPDATA%\localtex\models`.
pub fn models_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("LOCALTEX_MODELS") {
        return PathBuf::from(dir);
    }
    data_dir().join("models")
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
