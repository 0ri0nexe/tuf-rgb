use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::effects::EffectSpec;
use crate::hw::PowerState;

fn d_fps() -> f64 {
    30.0
}
fn d_one() -> f64 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Frame rate for software effects. Each frame is one ACPI call into the EC.
    #[serde(default = "d_fps")]
    pub fps: f64,
    /// Gamma applied to every software frame (1.0 = off, ~2.2 = perceptual).
    #[serde(default = "d_one")]
    pub gamma: f64,
    /// Software dimmer applied on top of the hardware brightness (0..1).
    #[serde(default = "d_one")]
    pub intensity: f64,
    /// Profile restored when the daemon starts.
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub power: PowerState,
    #[serde(default)]
    pub profiles: BTreeMap<String, EffectSpec>,
}

impl Default for Config {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        let presets = [
            ("tuf-orange", "type = 'solid'\ncolor = '#ff3c00'"),
            ("rainbow", "type = 'rainbow'\nperiod = 6.0"),
            ("ocean", "type = 'gradient'\ncolors = ['#001eff', '#00c8ff', '#00ffa0']\nperiod = 8.0"),
            ("calm-breathe", "type = 'breathe'\ncolors = ['#8000ff', '#00a0ff']\nperiod = 5.0\nmin = 0.05"),
            ("fire", "type = 'candle'\ncolor = '#ff6a10'\nflicker = 0.6"),
            ("cpu", "type = 'cpu-load'\nlow = '#00ff40'\nhigh = '#ff0000'"),
            ("thermal", "type = 'temperature'\ncold = '#0040ff'\nhot = '#ff0000'\nmin_c = 45.0\nmax_c = 95.0"),
            ("fw-breathing", "type = 'hardware'\nmode = 'breathing'\ncolor = '#ff0000'\nspeed = 'medium'"),
            ("fw-cycle", "type = 'hardware'\nmode = 'color-cycle'\nspeed = 'slow'"),
        ];
        for (name, spec) in presets {
            profiles.insert(name.to_string(), toml::from_str(spec).expect("preset"));
        }
        Self {
            fps: d_fps(),
            gamma: 1.0,
            intensity: 1.0,
            active: Some("tuf-orange".into()),
            power: PowerState::default(),
            profiles,
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("tuf-rgb").join("config.toml")
}

impl Config {
    pub fn load() -> Result<Self> {
        let p = config_path();
        match fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("invalid config {}", p.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", p.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let p = config_path();
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("toml.tmp");
        fs::write(&tmp, toml::to_string_pretty(self)?)?;
        fs::rename(&tmp, &p)?;
        Ok(())
    }
}
