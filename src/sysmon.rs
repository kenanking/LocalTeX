//! Local resource sampling for Settings → System. Linux reads /proc and
//! statvfs; Windows uses GetSystemTimes / GlobalMemoryStatusEx / GetDiskFreeSpaceEx.
//! Other targets compile to partial samples (compile-only paths).

use std::path::Path;
use std::time::Duration;

#[derive(Clone, Default)]
pub struct SysSnapshot {
    pub cpu_pct: Option<f32>,
    pub cpu_hist: Vec<f32>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    pub app_rss: Option<u64>,
    pub disk: Option<DiskSnapshot>,
}

#[derive(Clone)]
pub struct DiskSnapshot {
    pub drive_total: Option<u64>,
    pub drive_free: Option<u64>,
    pub models: u64,
    pub snips: u64,
    pub binary: u64,
}

impl DiskSnapshot {
    pub fn app_footprint(&self) -> u64 {
        self.models + self.snips + self.binary
    }
}

pub struct SysMon {
    prev_cpu: Option<(u64, u64)>,
    cpu_hist: std::collections::VecDeque<f32>,
}

pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(1500);
pub const DISK_EVERY_TICKS: u32 = 20;
pub const CPU_HIST_LEN: usize = 40;

impl SysMon {
    pub fn new() -> Self {
        Self {
            prev_cpu: None,
            cpu_hist: std::collections::VecDeque::with_capacity(CPU_HIST_LEN),
        }
    }

    pub fn sample(&mut self) -> SysSnapshot {
        let cpu_pct = cpu_jiffies().and_then(|(total, idle)| {
            let prev = self.prev_cpu.replace((total, idle))?;
            let dt = total.saturating_sub(prev.0) as f32;
            let di = idle.saturating_sub(prev.1) as f32;
            (dt > 0.0).then(|| (1.0 - di / dt).clamp(0.0, 1.0) * 100.0)
        });
        if let Some(pct) = cpu_pct {
            if self.cpu_hist.len() == CPU_HIST_LEN {
                self.cpu_hist.pop_front();
            }
            self.cpu_hist.push_back(pct);
        }
        let (mem_total, mem_used) = mem_info();
        SysSnapshot {
            cpu_pct,
            cpu_hist: self.cpu_hist.iter().copied().collect(),
            mem_used,
            mem_total,
            app_rss: self_rss(),
            disk: None,
        }
    }
}

/// Drive + app-footprint sizes. Walks the models/snips trees — call off the
/// UI thread and not on every tick.
pub fn disk_sample() -> DiskSnapshot {
    let data = crate::identity::data_dir();
    let models_path = crate::identity::models_dir();
    let models = dir_size(&models_path);
    let snips = snips_size(&data);
    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.metadata().ok())
        .map(|m| m.len())
        .unwrap_or(0);
    let (drive_total, drive_free) = drive_stats(&data);
    DiskSnapshot {
        drive_total,
        drive_free,
        models,
        snips,
        binary,
    }
}

/// Library bytes: `snips/` plus `snips.db` and its WAL sidecars.
fn snips_size(data: &Path) -> u64 {
    dir_size(&data.join("snips"))
        + file_size(&data.join("snips.db"))
        + file_size(&data.join("snips.db-wal"))
        + file_size(&data.join("snips.db-shm"))
}

fn file_size(path: &Path) -> u64 {
    path.metadata()
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .unwrap_or(0)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        total += if meta.is_dir() {
            dir_size(&entry.path())
        } else {
            meta.len()
        };
    }
    total
}

pub fn fmt_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else {
        format!("{} KB", bytes / KB)
    }
}

/// "used / total" sharing the total's unit: "11.2 / 60.5 GB".
pub fn fmt_used_total(used: u64, total: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    if total >= GB {
        format!(
            "{:.1} / {:.1} GB",
            used as f64 / GB as f64,
            total as f64 / GB as f64
        )
    } else {
        format!("{} / {}", fmt_bytes(used), fmt_bytes(total))
    }
}

#[cfg(target_os = "linux")]
fn cpu_jiffies() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let nums: Vec<u64> = stat
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() < 5 {
        return None;
    }
    Some((nums.iter().sum(), nums[3] + nums[4])) // idle + iowait
}

#[cfg(target_os = "windows")]
fn cpu_jiffies() -> Option<(u64, u64)> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetSystemTimes;
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetSystemTimes(
            Some(&mut idle as *mut FILETIME),
            Some(&mut kernel as *mut FILETIME),
            Some(&mut user as *mut FILETIME),
        )
    }
    .ok()?;
    // Kernel time already includes idle.
    let idle = filetime_u64(idle);
    let kernel = filetime_u64(kernel);
    let user = filetime_u64(user);
    Some((kernel.saturating_add(user), idle))
}

#[cfg(target_os = "windows")]
fn filetime_u64(ft: windows::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn cpu_jiffies() -> Option<(u64, u64)> {
    None
}

#[cfg(target_os = "linux")]
fn mem_info() -> (Option<u64>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return (None, None);
    };
    let mut total = None;
    let mut avail = None;
    for line in text.lines() {
        // meminfo values are kB.
        let kb = || line.split_whitespace().nth(1)?.parse::<u64>().ok();
        if line.starts_with("MemTotal:") {
            total = kb().map(|k| k * 1024);
        } else if line.starts_with("MemAvailable:") {
            avail = kb().map(|k| k * 1024);
        }
    }
    (total, total.zip(avail).map(|(t, a)| t.saturating_sub(a)))
}

#[cfg(target_os = "windows")]
fn mem_info() -> (Option<u64>, Option<u64>) {
    use std::mem::size_of;
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut mem = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut mem) }.is_err() {
        return (None, None);
    }
    let total = mem.ullTotalPhys;
    (Some(total), Some(total.saturating_sub(mem.ullAvailPhys)))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn mem_info() -> (Option<u64>, Option<u64>) {
    (None, None)
}

#[cfg(target_os = "linux")]
fn self_rss() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
    Some(pages * page)
}

#[cfg(target_os = "windows")]
fn self_rss() -> Option<u64> {
    use std::mem::size_of;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut pmc = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) }
        .ok()
        .map(|()| pmc.WorkingSetSize as u64)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn self_rss() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn drive_stats(path: &Path) -> (Option<u64>, Option<u64>) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return (None, None);
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
        return (None, None);
    }
    let frag = stat.f_frsize.max(1);
    (Some(stat.f_blocks * frag), Some(stat.f_bavail * frag))
}

#[cfg(target_os = "windows")]
fn drive_stats(path: &Path) -> (Option<u64>, Option<u64>) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut total = 0u64;
    let mut free = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            None,
            Some(&mut total as *mut u64),
            Some(&mut free as *mut u64),
        )
    }
    .ok()
    .map(|()| (Some(total), Some(free)))
    .unwrap_or((None, None))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn drive_stats(_path: &Path) -> (Option<u64>, Option<u64>) {
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_bytes_scales() {
        assert_eq!(fmt_bytes(512), "0 KB");
        assert_eq!(fmt_bytes(3 * 1024), "3 KB");
        assert_eq!(fmt_bytes(244 * 1024 * 1024), "244 MB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024 * 1024 + 536_870_912), "5.5 GB");
    }

    #[test]
    fn fmt_used_total_shares_unit() {
        let gb = 1024 * 1024 * 1024;
        assert_eq!(
            fmt_used_total(11 * gb + 246_960_742, 60 * gb + 536_870_912),
            "11.2 / 60.5 GB"
        );
        assert_eq!(
            fmt_used_total(300 * 1024 * 1024, 512 * 1024 * 1024),
            "300 MB / 512 MB"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_samples_are_populated() {
        let mut mon = SysMon::new();
        let first = mon.sample();
        assert!(first.cpu_pct.is_none());
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = mon.sample();
        assert!(second.cpu_pct.is_some());
        assert!(second.mem_total.unwrap_or(0) > 0);
        assert!(second.app_rss.unwrap_or(0) > 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_samples_are_populated() {
        let mut mon = SysMon::new();
        let first = mon.sample();
        assert!(first.cpu_pct.is_none());
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = mon.sample();
        assert!(second.cpu_pct.is_some());
        assert!(second.mem_total.unwrap_or(0) > 0);
        assert!(second.mem_used.unwrap_or(0) > 0);
        assert!(second.app_rss.unwrap_or(0) > 0);
        let disk = super::disk_sample();
        assert!(disk.drive_total.unwrap_or(0) > 0);
    }

    #[test]
    fn disk_sample_runs() {
        let d = super::disk_sample();
        assert_eq!(d.app_footprint(), d.models + d.snips + d.binary);
    }

    #[test]
    fn snips_size_ignores_unrelated_data_dir_files() {
        let root = std::env::temp_dir().join(format!(
            "localtex-disk-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("snips")).unwrap();
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::write(root.join("snips/a.png"), vec![0u8; 1000]).unwrap();
        std::fs::write(root.join("snips.db"), vec![0u8; 200]).unwrap();
        std::fs::write(root.join("snips.db-wal"), vec![0u8; 50]).unwrap();
        std::fs::write(root.join("snips.db-shm"), vec![0u8; 25]).unwrap();
        std::fs::write(root.join("junk.log"), vec![0u8; 9999]).unwrap();
        std::fs::write(root.join("models/x.bin"), vec![0u8; 8888]).unwrap();
        let n = snips_size(&root);
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(n, 1000 + 200 + 50 + 25);
    }
}
