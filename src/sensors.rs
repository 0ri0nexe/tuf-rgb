//! Tiny readers for system metrics used by reactive effects (no extra dependencies).

use std::fs;
use std::path::PathBuf;

/// CPU utilisation from /proc/stat, as a delta between two calls.
#[derive(Default)]
pub struct CpuLoad {
    prev: Option<(u64, u64)>,
}

impl CpuLoad {
    /// Returns load in 0..1 since the previous call (None on first call or read error).
    pub fn sample(&mut self) -> Option<f64> {
        let stat = fs::read_to_string("/proc/stat").ok()?;
        let line = stat.lines().next()?;
        let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
        if v.len() < 4 {
            return None;
        }
        let idle = v[3] + v.get(4).copied().unwrap_or(0);
        let total: u64 = v.iter().sum();
        let prev = self.prev.replace((idle, total));
        let (pi, pt) = prev?;
        let dt = total.saturating_sub(pt);
        if dt == 0 {
            return None;
        }
        Some(1.0 - idle.saturating_sub(pi) as f64 / dt as f64)
    }
}

/// Finds a temperature input. `hint` matches a hwmon name (e.g. "coretemp", "k10temp")
/// or a thermal zone type (e.g. "x86_pkg_temp"). Defaults to the CPU package sensor.
pub fn find_temp_sensor(hint: Option<&str>) -> Option<PathBuf> {
    let wanted: Vec<&str> = match hint {
        Some(h) => vec![h],
        None => vec!["coretemp", "k10temp", "zenpower", "x86_pkg_temp", "acpitz"],
    };
    for w in &wanted {
        if let Ok(rd) = fs::read_dir("/sys/class/hwmon") {
            for e in rd.flatten() {
                let name = fs::read_to_string(e.path().join("name")).unwrap_or_default();
                if name.trim() == *w && e.path().join("temp1_input").exists() {
                    return Some(e.path().join("temp1_input"));
                }
            }
        }
        if let Ok(rd) = fs::read_dir("/sys/class/thermal") {
            for e in rd.flatten() {
                let ty = fs::read_to_string(e.path().join("type")).unwrap_or_default();
                if ty.trim() == *w {
                    return Some(e.path().join("temp"));
                }
            }
        }
    }
    None
}

pub fn read_temp_c(path: &PathBuf) -> Option<f64> {
    let milli: f64 = fs::read_to_string(path).ok()?.trim().parse().ok()?;
    Some(milli / 1000.0)
}

pub struct Battery {
    pub percent: f64,
    pub charging: bool,
}

pub fn battery() -> Option<Battery> {
    let rd = fs::read_dir("/sys/class/power_supply").ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if fs::read_to_string(p.join("type")).ok()?.trim() != "Battery" {
            continue;
        }
        let percent: f64 = fs::read_to_string(p.join("capacity")).ok()?.trim().parse().ok()?;
        let status = fs::read_to_string(p.join("status")).unwrap_or_default();
        return Some(Battery { percent, charging: status.trim() == "Charging" });
    }
    None
}
