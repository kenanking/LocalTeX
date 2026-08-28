//! Local resource sampling for Settings → System. Linux reads /proc and
//! statvfs; other targets compile to partial samples (compile-only paths).

use std::path::Path;

#[derive(Clone, Default)]
pub struct SysSnapshot {
    /// System-wide CPU busy % since the previous sample (None on first tick).
    pub cpu_pct: Option<f32>,
    /// Recent CPU % samples, oldest first (sparkline data, ~1.5 s cadence).
    pub cpu_hist: Vec<f32>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    /// This process' resident set.
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

/// Keeps the previous CPU counters so each tick is a delta, plus a short
/// history for the settings sparkline.
pub struct SysMon {
    prev_cpu: Option<(u64, u64)>,
    cpu_hist: std::collections::VecDeque<f32>,
}

/// Sparkline window: 40 samples at 1.5 s ≈ one minute.
pub const CPU_HIST_LEN: usize = 40;

impl SysMon {
    pub fn new() -> Self {
        Self {
            prev_cpu: None,
            cpu_hist: std::collections::VecDeque::with_capacity(CPU_HIST_LEN),
        }
    }

    /// Cheap per-tick sample: a few tiny /proc reads, no directory walks.
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
    let mut snips = dir_size(&data);
    if models_path.starts_with(&data) {
        snips = snips.saturating_sub(models);
    }
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

#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "linux"))]
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

    #[test]
    fn disk_sample_runs() {
        let d = super::disk_sample();
        assert_eq!(d.app_footprint(), d.models + d.snips + d.binary);
    }
}
