use std::path::Path;
#[cfg(any(test, target_os = "linux"))]
use std::path::PathBuf;

#[cfg(any(test, target_os = "linux"))]
use crate::identity::APP_ID;
use crate::identity::APP_NAME;

/// Align the OS login-item with `enabled`. Safe to call twice.
pub fn apply(enabled: bool) -> anyhow::Result<()> {
    platform_apply(enabled)
}

#[cfg(target_os = "linux")]
fn platform_apply(enabled: bool) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    apply_linux_at(enabled, &linux_desktop_path()?, &exe)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn platform_apply(enabled: bool) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    apply_windows(enabled, &exe)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn platform_apply(_enabled: bool) -> anyhow::Result<()> {
    Ok(())
}

pub(crate) fn quoted_command(exe: &Path) -> String {
    let raw = exe.to_string_lossy();
    format!("\"{}\"", raw.replace('"', "\\\""))
}

fn autostart_command(exe: &Path) -> String {
    format!("{} --autostart", quoted_command(exe))
}

#[cfg(any(test, target_os = "linux"))]
fn linux_desktop_entry(exe: &Path) -> String {
    let exec = autostart_command(exe);
    format!(
        "\
[Desktop Entry]
Type=Application
Version=1.0
Name={APP_NAME}
Comment=Screenshot to TeX and Markdown
Exec={exec}
Icon={APP_ID}
Terminal=false
Categories=Utility;Graphics;Office;
StartupWMClass={APP_ID}
StartupNotify=false
X-GNOME-Autostart-enabled=true
"
    )
}

#[cfg(target_os = "linux")]
fn linux_desktop_path() -> anyhow::Result<PathBuf> {
    let dir = dirs::config_dir().ok_or_else(|| anyhow::anyhow!("no config dir"))?;
    Ok(dir.join("autostart").join(format!("{APP_ID}.desktop")))
}

#[cfg(any(test, target_os = "linux"))]
fn apply_linux_at(enabled: bool, desktop: &Path, exe: &Path) -> std::io::Result<()> {
    if enabled {
        if let Some(parent) = desktop.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let desired = linux_desktop_entry(exe);
        if std::fs::read_to_string(desktop).is_ok_and(|current| current == desired) {
            return Ok(());
        }
        std::fs::write(desktop, desired)
    } else {
        match std::fs::remove_file(desktop) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

#[cfg(target_os = "windows")]
fn apply_windows(enabled: bool, exe: &Path) -> anyhow::Result<()> {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
        RegOpenKeyExW, RegSetValueExW,
    };
    use windows::core::PCWSTR;

    let mut key = HKEY::default();
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        anyhow::bail!("open Run key {status:?}");
    }

    let name: Vec<u16> = APP_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let result = if enabled {
        let command = autostart_command(exe);
        let bytes = reg_sz_bytes(&command);
        let status =
            unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&bytes)) };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(anyhow::anyhow!("set Run value {status:?}"))
        }
    } else {
        let status = unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(anyhow::anyhow!("delete Run value {status:?}"))
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

#[cfg(any(test, target_os = "windows"))]
fn reg_sz_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for unit in s.encode_utf16().chain(std::iter::once(0)) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "localtex-autostart-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn quoted_command_wraps_spaces() {
        assert_eq!(
            quoted_command(Path::new(r"C:\Program Files\LocalTeX\localtex.exe")),
            r#""C:\Program Files\LocalTeX\localtex.exe""#
        );
    }

    #[test]
    fn quoted_command_escapes_quotes_in_path() {
        assert_eq!(
            quoted_command(Path::new(r#"C:\odd"name\localtex.exe"#)),
            r#""C:\odd\"name\localtex.exe""#
        );
    }

    #[test]
    fn linux_desktop_entry_points_at_this_exe() {
        let text = linux_desktop_entry(Path::new("/opt/LocalTeX/bin/localtex"));
        assert!(text.contains("Name=LocalTeX"));
        assert!(text.contains("Exec=\"/opt/LocalTeX/bin/localtex\" --autostart"));
        assert!(text.contains("Icon=com.localtex.app"));
        assert!(text.contains("X-GNOME-Autostart-enabled=true"));
    }

    #[test]
    fn apply_linux_writes_and_removes_the_desktop_file() {
        let dir = scratch();
        let desktop = dir.join("autostart").join("com.localtex.app.desktop");
        let exe = Path::new("/opt/LocalTeX/bin/localtex");
        apply_linux_at(true, &desktop, exe).unwrap();
        let raw = fs::read_to_string(&desktop).unwrap();
        assert!(raw.contains("Exec=\"/opt/LocalTeX/bin/localtex\" --autostart"));
        apply_linux_at(true, &desktop, exe).unwrap();
        apply_linux_at(false, &desktop, exe).unwrap();
        assert!(!desktop.exists());
        apply_linux_at(false, &desktop, exe).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reg_sz_bytes_are_utf16_nul_terminated() {
        let bytes = reg_sz_bytes("A");
        assert_eq!(bytes, vec![b'A', 0, 0, 0]);
    }
}
