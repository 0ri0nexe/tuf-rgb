//! Hardware access through the asus-wmi "TUF RGB" sysfs interface.
//!
//! Reverse-engineering summary (see docs/REVERSING.md):
//! Armoury Crate (AacNBDTHal) builds an Aura packet `[0x5D, 0xB3, mode, R, G, B, speed, dir]`.
//! On TUF notebooks the BIOS exposes the same bytes through WMI `DEVS 0x00100056`
//! (`arg0 = cmd | mode<<8 | R<<16 | G<<24`, `arg1 = B | speed<<8`), and the ACPI method
//! `TU56` forwards them untouched to the EC mailbox. Linux' asus-wmi exposes this as
//! `kbd_rgb_mode` ("cmd mode r g b speed") and `kbd_rgb_state` (`DEVS 0x00100057`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::color::Rgb;

pub const DEFAULT_LED: &str = "/sys/class/leds/asus::kbd_backlight";

/// Firmware lighting modes. Values are the `mode` byte of the Aura packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HwMode {
    Static,
    Breathing,
    ColorCycle,
    Strobe,
    /// Any other mode byte (0..=11 except 9, kernel clamps the rest to 10).
    Raw(u8),
}

impl HwMode {
    pub fn byte(self) -> u8 {
        match self {
            HwMode::Static => 0,
            HwMode::Breathing => 1,
            HwMode::ColorCycle => 2,
            HwMode::Strobe => 3,
            HwMode::Raw(b) => b,
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "static" | "0" => HwMode::Static,
            "breathing" | "breathe" | "1" => HwMode::Breathing,
            "color-cycle" | "cycle" | "colorcycle" | "2" => HwMode::ColorCycle,
            "strobe" | "strobing" | "3" => HwMode::Strobe,
            other => match other.strip_prefix("raw:").unwrap_or(other).parse::<u8>() {
                Ok(b) if b <= 11 => HwMode::Raw(b),
                _ => bail!("unknown hardware mode '{s}' (static, breathing, color-cycle, strobe, raw:N)"),
            },
        })
    }

    pub const NAMED: [HwMode; 4] = [HwMode::Static, HwMode::Breathing, HwMode::ColorCycle, HwMode::Strobe];

    pub fn label(self) -> String {
        match self {
            HwMode::Static => "Static".into(),
            HwMode::Breathing => "Breathing".into(),
            HwMode::ColorCycle => "Color cycle".into(),
            HwMode::Strobe => "Strobe".into(),
            HwMode::Raw(b) => format!("Raw {b}"),
        }
    }
}

/// Firmware animation speed. Index sent to asus-wmi, which maps 0/1/2 to 0xE1/0xEB/0xF5.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Speed {
    Slow,
    #[default]
    Medium,
    Fast,
}

impl Speed {
    pub fn index(self) -> u8 {
        match self {
            Speed::Slow => 0,
            Speed::Medium => 1,
            Speed::Fast => 2,
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "slow" | "0" => Speed::Slow,
            "medium" | "1" => Speed::Medium,
            "fast" | "2" => Speed::Fast,
            _ => bail!("unknown speed '{s}' (slow, medium, fast)"),
        })
    }
}

/// When the keyboard is lit (DEVS 0x00100057).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerState {
    pub boot: bool,
    pub awake: bool,
    pub sleep: bool,
    pub keyboard: bool,
}

impl Default for PowerState {
    fn default() -> Self {
        Self { boot: true, awake: true, sleep: true, keyboard: true }
    }
}

/// Anything that can drive the keyboard.
pub trait Backend: Send {
    /// `save = true` asks the firmware to persist the setting (cmd 0xB4) instead of applying it
    /// temporarily (cmd 0xB3). Never save in an animation loop.
    fn set_mode(&mut self, mode: HwMode, color: Rgb, speed: Speed, save: bool) -> Result<()>;
    fn set_power_state(&mut self, st: PowerState, save: bool) -> Result<()>;
    fn brightness(&self) -> Result<u8>;
    fn max_brightness(&self) -> Result<u8>;
    fn set_brightness(&mut self, level: u8) -> Result<()>;
    fn describe(&self) -> String;

    fn set_color(&mut self, color: Rgb) -> Result<()> {
        self.set_mode(HwMode::Static, color, Speed::Medium, false)
    }
}

pub struct SysfsBackend {
    dir: PathBuf,
    last: Option<(u8, Rgb, u8)>,
}

impl SysfsBackend {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        if !dir.join("kbd_rgb_mode").exists() {
            bail!(
                "{} has no kbd_rgb_mode: asus-wmi TUF RGB interface not available on this machine",
                dir.display()
            );
        }
        Ok(Self { dir, last: None })
    }

    fn write(&self, file: &str, value: &str) -> Result<()> {
        let p = self.dir.join(file);
        fs::write(&p, value).with_context(|| {
            format!(
                "cannot write '{}' to {} (install the udev rule or run as root)",
                value.trim(),
                p.display()
            )
        })
    }

    fn read(&self, file: &str) -> Result<u8> {
        let p = self.dir.join(file);
        let s = fs::read_to_string(&p).with_context(|| format!("cannot read {}", p.display()))?;
        Ok(s.trim().parse()?)
    }

    pub fn writable(&self) -> bool {
        fs::OpenOptions::new().write(true).open(self.dir.join("kbd_rgb_mode")).is_ok()
    }
}

/// The exact line asus-wmi expects in kbd_rgb_mode.
pub fn mode_line(mode: HwMode, color: Rgb, speed: Speed, save: bool) -> String {
    format!("{} {} {} {} {} {}\n", save as u8, mode.byte(), color.r, color.g, color.b, speed.index())
}

pub fn state_line(st: PowerState, save: bool) -> String {
    format!(
        "{} {} {} {} {}\n",
        save as u8, st.boot as u8, st.awake as u8, st.sleep as u8, st.keyboard as u8
    )
}

impl Backend for SysfsBackend {
    fn set_mode(&mut self, mode: HwMode, color: Rgb, speed: Speed, save: bool) -> Result<()> {
        let key = (mode.byte(), color, speed.index());
        // Each write is an ACPI call into the EC; skip redundant frames.
        if !save && self.last == Some(key) {
            return Ok(());
        }
        self.write("kbd_rgb_mode", &mode_line(mode, color, speed, save))?;
        self.last = Some(key);
        Ok(())
    }

    fn set_power_state(&mut self, st: PowerState, save: bool) -> Result<()> {
        self.write("kbd_rgb_state", &state_line(st, save))
    }

    fn brightness(&self) -> Result<u8> {
        self.read("brightness")
    }

    fn max_brightness(&self) -> Result<u8> {
        self.read("max_brightness")
    }

    fn set_brightness(&mut self, level: u8) -> Result<()> {
        let max = self.max_brightness().unwrap_or(3);
        self.write("brightness", &format!("{}\n", level.min(max)))
    }

    fn describe(&self) -> String {
        format!("sysfs {}", self.dir.display())
    }
}

/// Prints what would be written. Used for testing without hardware access.
#[derive(Default)]
pub struct DryRunBackend {
    pub brightness: u8,
    pub log: Vec<String>,
    pub quiet: bool,
}

impl Backend for DryRunBackend {
    fn set_mode(&mut self, mode: HwMode, color: Rgb, speed: Speed, save: bool) -> Result<()> {
        let l = format!("kbd_rgb_mode <- {}", mode_line(mode, color, speed, save).trim());
        if !self.quiet {
            eprintln!("[dry-run] {l}");
        }
        self.log.push(l);
        Ok(())
    }

    fn set_power_state(&mut self, st: PowerState, save: bool) -> Result<()> {
        let l = format!("kbd_rgb_state <- {}", state_line(st, save).trim());
        if !self.quiet {
            eprintln!("[dry-run] {l}");
        }
        self.log.push(l);
        Ok(())
    }

    fn brightness(&self) -> Result<u8> {
        Ok(self.brightness)
    }

    fn max_brightness(&self) -> Result<u8> {
        Ok(3)
    }

    fn set_brightness(&mut self, level: u8) -> Result<()> {
        self.brightness = level.min(3);
        Ok(())
    }

    fn describe(&self) -> String {
        "dry-run".into()
    }
}

/// Opens the sysfs backend, or the dry-run one if `dry_run` or TUF_RGB_DRY_RUN is set.
pub fn open_backend(dry_run: bool, led_dir: Option<&Path>) -> Result<Box<dyn Backend>> {
    if dry_run || std::env::var_os("TUF_RGB_DRY_RUN").is_some() {
        return Ok(Box::new(DryRunBackend::default()));
    }
    let dir = led_dir.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(DEFAULT_LED));
    Ok(Box::new(SysfsBackend::open(dir)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        assert_eq!(
            mode_line(HwMode::Breathing, Rgb::new(1, 2, 3), Speed::Fast, false),
            "0 1 1 2 3 2\n"
        );
        assert_eq!(mode_line(HwMode::Raw(10), Rgb::new(0, 0, 0), Speed::Slow, true), "1 10 0 0 0 0\n");
        let st = PowerState { boot: true, awake: true, sleep: false, keyboard: true };
        assert_eq!(state_line(st, true), "1 1 1 0 1\n");
    }

    #[test]
    fn parse_mode() {
        assert_eq!(HwMode::parse("color-cycle").unwrap(), HwMode::ColorCycle);
        assert_eq!(HwMode::parse("raw:11").unwrap(), HwMode::Raw(11));
        assert!(HwMode::parse("raw:42").is_err());
    }
}
