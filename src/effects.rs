//! Software effects. The TUF single-zone keyboard only accepts one color at a time, so every
//! animation Armoury Crate offers beyond the 4 firmware modes is rendered on the host and pushed
//! frame by frame as a temporary (non-saved) static color. This module does the same, with more.

use std::f64::consts::TAU;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::color::{BLACK, Rgb, palette_at};
use crate::hw::{HwMode, Speed};
use crate::sensors;

fn d_period() -> f64 {
    4.0
}
fn d_one() -> f64 {
    1.0
}
fn d_rainbow() -> Vec<Rgb> {
    ["#ff0000", "#ffff00", "#00ff00", "#00ffff", "#0000ff", "#ff00ff"]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect()
}
fn d_warm() -> Rgb {
    Rgb::new(255, 110, 20)
}
fn d_red() -> Rgb {
    Rgb::new(255, 0, 0)
}
fn d_blue() -> Rgb {
    Rgb::new(0, 0, 255)
}
fn d_green() -> Rgb {
    Rgb::new(0, 255, 0)
}
fn d_white() -> Rgb {
    Rgb::new(255, 255, 255)
}

/// Declarative description of an effect, stored in profiles and sent over IPC.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum EffectSpec {
    /// Firmware-driven mode, written once (no host CPU usage).
    Hardware {
        mode: HwMode,
        #[serde(default = "d_red")]
        color: Rgb,
        #[serde(default)]
        speed: Speed,
        /// Persist in the EC so it survives reboot (cmd 0xB4).
        #[serde(default)]
        save: bool,
    },
    Solid {
        color: Rgb,
    },
    /// Smooth breathing, cycling through `colors` on each breath.
    Breathe {
        #[serde(default = "d_rainbow")]
        colors: Vec<Rgb>,
        #[serde(default = "d_period")]
        period: f64,
        /// Lowest brightness of a breath (0..1).
        #[serde(default)]
        min: f64,
    },
    /// HSV hue rotation.
    Rainbow {
        #[serde(default = "d_period")]
        period: f64,
        #[serde(default = "d_one")]
        saturation: f64,
        #[serde(default = "d_one")]
        value: f64,
    },
    /// Continuous blend around a palette.
    Gradient {
        #[serde(default = "d_rainbow")]
        colors: Vec<Rgb>,
        #[serde(default = "d_period")]
        period: f64,
    },
    /// Holds each color, then crossfades to the next.
    Cycle {
        #[serde(default = "d_rainbow")]
        colors: Vec<Rgb>,
        #[serde(default = "d_one")]
        hold: f64,
        #[serde(default = "d_one")]
        fade: f64,
    },
    Strobe {
        #[serde(default = "d_white_vec")]
        colors: Vec<Rgb>,
        /// Flashes per second.
        #[serde(default = "d_strobe_hz")]
        frequency: f64,
        /// Fraction of the period the LED is on.
        #[serde(default = "d_duty")]
        duty: f64,
    },
    Heartbeat {
        #[serde(default = "d_red")]
        color: Rgb,
        #[serde(default = "d_bpm")]
        bpm: f64,
        #[serde(default)]
        min: f64,
    },
    /// Flickering flame.
    Candle {
        #[serde(default = "d_warm")]
        color: Rgb,
        /// Flicker depth (0..1).
        #[serde(default = "d_flicker")]
        flicker: f64,
    },
    /// Alternating double flashes.
    Police {
        #[serde(default = "d_red")]
        a: Rgb,
        #[serde(default = "d_blue")]
        b: Rgb,
        #[serde(default = "d_one")]
        period: f64,
    },
    /// Base color with random lightning flashes.
    Storm {
        #[serde(default = "d_storm_base")]
        base: Rgb,
        #[serde(default = "d_white")]
        flash: Rgb,
        /// Average flashes per minute.
        #[serde(default = "d_storm_rate")]
        rate: f64,
    },
    /// Color follows CPU utilisation.
    CpuLoad {
        #[serde(default = "d_green")]
        low: Rgb,
        #[serde(default = "d_red")]
        high: Rgb,
    },
    /// Color follows a temperature sensor.
    Temperature {
        #[serde(default = "d_blue")]
        cold: Rgb,
        #[serde(default = "d_red")]
        hot: Rgb,
        #[serde(default = "d_tmin")]
        min_c: f64,
        #[serde(default = "d_tmax")]
        max_c: f64,
        /// hwmon name or thermal zone type, default: CPU package.
        #[serde(default)]
        sensor: Option<String>,
    },
    /// Color follows battery charge; breathes while charging.
    Battery {
        #[serde(default = "d_red")]
        empty: Rgb,
        #[serde(default = "d_green")]
        full: Rgb,
    },
    Off,
}

fn d_white_vec() -> Vec<Rgb> {
    vec![d_white()]
}
fn d_strobe_hz() -> f64 {
    8.0
}
fn d_duty() -> f64 {
    0.3
}
fn d_bpm() -> f64 {
    60.0
}
fn d_flicker() -> f64 {
    0.5
}
fn d_storm_base() -> Rgb {
    Rgb::new(10, 0, 60)
}
fn d_storm_rate() -> f64 {
    12.0
}
fn d_tmin() -> f64 {
    40.0
}
fn d_tmax() -> f64 {
    90.0
}

impl EffectSpec {
    pub fn kind(&self) -> &'static str {
        match self {
            EffectSpec::Hardware { .. } => "hardware",
            EffectSpec::Solid { .. } => "solid",
            EffectSpec::Breathe { .. } => "breathe",
            EffectSpec::Rainbow { .. } => "rainbow",
            EffectSpec::Gradient { .. } => "gradient",
            EffectSpec::Cycle { .. } => "cycle",
            EffectSpec::Strobe { .. } => "strobe",
            EffectSpec::Heartbeat { .. } => "heartbeat",
            EffectSpec::Candle { .. } => "candle",
            EffectSpec::Police { .. } => "police",
            EffectSpec::Storm { .. } => "storm",
            EffectSpec::CpuLoad { .. } => "cpu-load",
            EffectSpec::Temperature { .. } => "temperature",
            EffectSpec::Battery { .. } => "battery",
            EffectSpec::Off => "off",
        }
    }

    /// One default instance of every effect kind, in display order.
    pub fn catalog() -> Vec<EffectSpec> {
        [
            "hardware", "solid", "breathe", "rainbow", "gradient", "cycle", "strobe", "heartbeat",
            "candle", "police", "storm", "cpu-load", "temperature", "battery", "off",
        ]
        .iter()
        .map(|k| EffectSpec::default_for(k).unwrap())
        .collect()
    }

    /// Default parameters for a kind name.
    pub fn default_for(kind: &str) -> Option<EffectSpec> {
        let base = match kind {
            "hardware" => "mode = 'breathing'",
            "solid" => "color = '#ff3c00'",
            _ => "",
        };
        toml::from_str(&format!("type = '{kind}'\n{base}")).ok()
    }

    /// True if the effect needs to be re-rendered continuously.
    pub fn animated(&self) -> bool {
        !matches!(self, EffectSpec::Hardware { .. } | EffectSpec::Solid { .. } | EffectSpec::Off)
    }

    pub fn build(&self) -> Box<dyn Effect> {
        Box::new(Renderer::new(self.clone()))
    }
}

/// Produces one color per frame.
pub trait Effect: Send {
    /// `t` = seconds since the effect started, `dt` = seconds since previous frame.
    fn frame(&mut self, t: f64, dt: f64) -> Rgb;
}

/// xorshift64*, enough for flicker.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Rng(seed | 1)
    }
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Renderer {
    spec: EffectSpec,
    rng: Rng,
    smooth: f64,
    target: f64,
    next_sample: f64,
    cpu: sensors::CpuLoad,
    temp_path: Option<PathBuf>,
    flash_until: f64,
    charging: bool,
}

impl Renderer {
    fn new(spec: EffectSpec) -> Self {
        let temp_path = match &spec {
            EffectSpec::Temperature { sensor, .. } => sensors::find_temp_sensor(sensor.as_deref()),
            _ => None,
        };
        Self {
            spec,
            rng: Rng::new(),
            smooth: 0.0,
            target: 0.0,
            next_sample: 0.0,
            cpu: sensors::CpuLoad::default(),
            temp_path,
            flash_until: -1.0,
            charging: false,
        }
    }

    /// Exponential smoothing toward `target`, `tau` seconds time constant.
    fn ease(&mut self, dt: f64, tau: f64) -> f64 {
        let k = 1.0 - (-dt / tau.max(1e-3)).exp();
        self.smooth += (self.target - self.smooth) * k;
        self.smooth
    }

    fn sample_due(&mut self, t: f64, every: f64) -> bool {
        if t >= self.next_sample {
            self.next_sample = t + every;
            true
        } else {
            false
        }
    }
}

fn breath(phase: f64) -> f64 {
    0.5 - 0.5 * (phase * TAU).cos()
}

impl Effect for Renderer {
    fn frame(&mut self, t: f64, dt: f64) -> Rgb {
        match self.spec.clone() {
            EffectSpec::Hardware { color, .. } | EffectSpec::Solid { color } => color,
            EffectSpec::Off => BLACK,
            EffectSpec::Breathe { colors, period, min } => {
                if colors.is_empty() {
                    return BLACK;
                }
                let period = period.max(0.1);
                let n = (t / period).floor() as usize;
                let k = min + (1.0 - min) * breath((t / period).fract());
                colors[n % colors.len()].scale(k)
            }
            EffectSpec::Rainbow { period, saturation, value } => {
                Rgb::from_hsv(360.0 * t / period.max(0.1), saturation, value)
            }
            EffectSpec::Gradient { colors, period } => palette_at(&colors, t / period.max(0.1)),
            EffectSpec::Cycle { colors, hold, fade } => {
                if colors.is_empty() {
                    return BLACK;
                }
                let step = (hold + fade).max(0.05);
                let n = (t / step).floor() as usize;
                let within = t - n as f64 * step;
                let a = colors[n % colors.len()];
                let b = colors[(n + 1) % colors.len()];
                if within < hold { a } else { a.lerp(b, (within - hold) / fade.max(1e-3)) }
            }
            EffectSpec::Strobe { colors, frequency, duty } => {
                if colors.is_empty() {
                    return BLACK;
                }
                let p = t * frequency.max(0.1);
                if p.fract() < duty { colors[p.floor() as usize % colors.len()] } else { BLACK }
            }
            EffectSpec::Heartbeat { color, bpm, min } => {
                // "lub-dub": two gaussian pulses per beat.
                let ph = (t * bpm / 60.0).fract();
                let g = |c: f64, w: f64| (-((ph - c) / w).powi(2)).exp();
                let k = (g(0.10, 0.05) + 0.6 * g(0.30, 0.06)).min(1.0);
                color.scale(min + (1.0 - min) * k)
            }
            EffectSpec::Candle { color, flicker } => {
                if self.sample_due(t, 0.06) {
                    self.target = 1.0 - flicker * self.rng.next().powf(2.0);
                }
                let k = self.ease(dt, 0.05);
                // Dimmer flame also gets redder.
                color.lerp(Rgb::new(255, 40, 0), (1.0 - k) * 0.5).scale(k)
            }
            EffectSpec::Police { a, b, period } => {
                let ph = (t / period.max(0.1)).fract();
                let (c, local) = if ph < 0.5 { (a, ph * 2.0) } else { (b, (ph - 0.5) * 2.0) };
                // Two short flashes per half-period.
                if (0.0..0.2).contains(&local) || (0.3..0.5).contains(&local) { c } else { BLACK }
            }
            EffectSpec::Storm { base, flash, rate } => {
                if t > self.flash_until && self.rng.next() < rate / 60.0 * dt {
                    self.flash_until = t + 0.05 + 0.15 * self.rng.next();
                }
                if t < self.flash_until {
                    // Jittery flash.
                    if self.rng.next() > 0.3 { flash } else { base }
                } else {
                    base
                }
            }
            EffectSpec::CpuLoad { low, high } => {
                if self.sample_due(t, 0.5) {
                    if let Some(v) = self.cpu.sample() {
                        self.target = v;
                    }
                }
                let k = self.ease(dt, 0.4);
                low.lerp(high, k)
            }
            EffectSpec::Temperature { cold, hot, min_c, max_c, .. } => {
                if self.sample_due(t, 1.0) {
                    if let Some(c) = self.temp_path.as_ref().and_then(sensors::read_temp_c) {
                        self.target = ((c - min_c) / (max_c - min_c).max(1.0)).clamp(0.0, 1.0);
                    }
                }
                let k = self.ease(dt, 1.0);
                cold.lerp(hot, k)
            }
            EffectSpec::Battery { empty, full } => {
                if self.sample_due(t, 5.0) {
                    if let Some(b) = sensors::battery() {
                        self.target = b.percent / 100.0;
                        self.charging = b.charging;
                    }
                }
                let k = self.ease(dt, 0.5);
                let c = empty.lerp(full, k);
                if self.charging { c.scale(0.3 + 0.7 * breath((t / 3.0).fract())) } else { c }
            }
        }
    }
}

/// Short overlay used for notifications: blink `color` `count` times then hand back.
pub struct Flash {
    pub color: Rgb,
    pub count: u32,
    pub period: f64,
    pub started: f64,
}

impl Flash {
    pub fn color_at(&self, t: f64) -> Option<Rgb> {
        let local = t - self.started;
        if local < 0.0 || local >= self.count as f64 * self.period {
            return None;
        }
        Some(if (local / self.period).fract() < 0.5 { self.color } else { BLACK })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_roundtrip() {
        for spec in EffectSpec::catalog() {
            let s = toml::to_string(&spec).unwrap();
            let back: EffectSpec = toml::from_str(&s).unwrap();
            assert_eq!(spec, back, "{s}");
            let mut fx = spec.build();
            for i in 0..50 {
                fx.frame(i as f64 * 0.033, 0.033);
            }
        }
    }

    #[test]
    fn breathe_bounds() {
        let spec = EffectSpec::Breathe { colors: vec![Rgb::new(200, 0, 0)], period: 2.0, min: 0.0 };
        let mut fx = spec.build();
        assert_eq!(fx.frame(0.0, 0.0), BLACK);
        assert_eq!(fx.frame(1.0, 0.0), Rgb::new(200, 0, 0));
    }

    #[test]
    fn flash() {
        let f = Flash { color: d_red(), count: 2, period: 1.0, started: 10.0 };
        assert_eq!(f.color_at(10.1), Some(d_red()));
        assert_eq!(f.color_at(10.6), Some(BLACK));
        assert_eq!(f.color_at(12.1), None);
    }
}
