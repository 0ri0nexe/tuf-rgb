//! tuf-rgb: command-line control. Talks to tuf-rgbd when it runs, otherwise drives sysfs directly.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tuf_rgb::Rgb;
use tuf_rgb::config::{Config, config_path};
use tuf_rgb::effects::EffectSpec;
use tuf_rgb::engine::Engine;
use tuf_rgb::hw::{self, HwMode, PowerState, Speed};
use tuf_rgb::ipc::{self, Request, Status};

#[derive(Parser)]
#[command(version, about = "ASUS TUF keyboard lighting control", after_help = EXAMPLES)]
struct Cli {
    /// Print sysfs writes instead of performing them.
    #[arg(long, global = true)]
    dry_run: bool,
    /// Ignore a running daemon and talk to sysfs directly.
    #[arg(long, global = true)]
    direct: bool,
    /// LED class directory.
    #[arg(long, global = true)]
    led: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

const EXAMPLES: &str = "Examples:
  tuf-rgb color '#ff3c00'                 static color (temporary)
  tuf-rgb color cyan --save               static color stored in the EC (survives reboot)
  tuf-rgb hw breathing -c red -s slow     firmware breathing
  tuf-rgb effect rainbow period=5
  tuf-rgb effect breathe colors=#ff0000,#0000ff period=3 min=0.1
  tuf-rgb effect temperature cold=#0040ff hot=red min_c=45 max_c=95
  tuf-rgb profile save night breathe colors=#300060 period=8
  tuf-rgb notify green --count 3
  tuf-rgb power --sleep off --save";

#[derive(Subcommand)]
enum Cmd {
    /// Show daemon / hardware status.
    Status,
    /// Set a static color.
    Color {
        color: Rgb,
        /// Persist in the EC (cmd 0xB4).
        #[arg(long)]
        save: bool,
    },
    /// Use a firmware lighting mode (static, breathing, color-cycle, strobe, raw:N).
    Hw {
        mode: String,
        #[arg(short, long, default_value = "red")]
        color: Rgb,
        #[arg(short, long, default_value = "medium")]
        speed: String,
        #[arg(long)]
        save: bool,
    },
    /// Run a software effect: `effect <kind> [key=value ...]`. See `effects`.
    Effect {
        kind: String,
        params: Vec<String>,
        /// Render in this process (Ctrl-C to stop) even if the daemon is running.
        #[arg(long)]
        foreground: bool,
    },
    /// List effect kinds and their parameters.
    Effects,
    /// Manage profiles.
    Profile {
        #[command(subcommand)]
        action: ProfileCmd,
    },
    /// Configure when the keyboard is lit (boot / awake / sleep / keyboard).
    Power {
        #[arg(long)]
        boot: Option<OnOff>,
        #[arg(long)]
        awake: Option<OnOff>,
        #[arg(long)]
        sleep: Option<OnOff>,
        #[arg(long)]
        keyboard: Option<OnOff>,
        #[arg(long)]
        save: bool,
    },
    /// Get or set the hardware brightness level (0..3).
    Brightness { level: Option<u8> },
    /// Blink a color, then resume the running effect (daemon only).
    Notify {
        color: Rgb,
        #[arg(short, long, default_value_t = 3)]
        count: u32,
        #[arg(short, long, default_value_t = 0.4)]
        period: f64,
    },
    /// Daemon rendering settings.
    Settings {
        #[arg(long)]
        fps: Option<f64>,
        #[arg(long)]
        gamma: Option<f64>,
        #[arg(long)]
        intensity: Option<f64>,
    },
    /// Stop the daemon.
    Stop,
}

#[derive(Subcommand)]
enum ProfileCmd {
    List,
    Show { name: String },
    Apply { name: String },
    /// `profile save <name> <kind> [key=value ...]`
    Save { name: String, kind: String, params: Vec<String> },
    Delete { name: String },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum OnOff {
    On,
    Off,
}

/// Build an EffectSpec from `kind` and `key=value` pairs by assembling a TOML document.
fn parse_effect(kind: &str, params: &[String]) -> Result<EffectSpec> {
    let mut doc = toml::Table::new();
    doc.insert("type".into(), toml::Value::String(kind.into()));
    for p in params {
        let (k, v) = p.split_once('=').with_context(|| format!("expected key=value, got '{p}'"))?;
        let value = if k == "colors" {
            toml::Value::Array(
                v.split(',').map(|c| Ok(toml::Value::String(c.parse::<Rgb>()?.hex()))).collect::<Result<_>>()?,
            )
        } else if let Ok(t) = toml::from_str::<toml::Table>(&format!("v = {v}")) {
            t["v"].clone()
        } else {
            toml::Value::String(v.to_string())
        };
        doc.insert(k.to_string(), value);
    }
    toml::Value::Table(doc)
        .try_into()
        .with_context(|| format!("invalid parameters for effect '{kind}' (see `tuf-rgb effects`)"))
}

fn print_status(st: &Status) {
    println!("backend     : {}", st.backend);
    println!("profile     : {}", st.active_profile.as_deref().unwrap_or("-"));
    if let Some(e) = &st.effect {
        println!("effect      : {}", serde_json::to_string(e).unwrap_or_default());
    }
    if let Some(c) = st.color {
        println!("last color  : {c}");
    }
    if let Some(b) = st.brightness {
        println!("brightness  : {b}/3");
    }
    println!("fps/gamma/intensity: {} / {} / {}", st.fps, st.gamma, st.intensity);
    if let Some(p) = st.power {
        println!(
            "power       : boot={} awake={} sleep={} keyboard={}",
            p.boot, p.awake, p.sleep, p.keyboard
        );
    }
    println!("profiles    : {}", st.profiles.join(", "));
    if let Some(e) = &st.last_error {
        println!("last error  : {e}");
    }
}

/// Render an effect in this process until Ctrl-C.
fn run_foreground(cli: &Cli, spec: EffectSpec) -> Result<()> {
    let backend = hw::open_backend(cli.dry_run, cli.led.as_deref())?;
    let mut engine = Engine::new(backend, Config::load()?);
    engine.apply(spec)?;
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    ctrlc::set_handler(move || s.store(true, Ordering::SeqCst))?;
    eprintln!("rendering in foreground, Ctrl-C to stop");
    while !stop.load(Ordering::SeqCst) {
        let wait = engine.render()?;
        std::thread::sleep(wait.min(std::time::Duration::from_millis(200)));
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let daemon = !cli.direct && !cli.dry_run && ipc::daemon_running();
    let direct = || hw::open_backend(cli.dry_run, cli.led.as_deref());

    match &cli.cmd {
        Cmd::Status => {
            if daemon {
                print_status(&ipc::send(&Request::Status)?.status.unwrap_or_default());
            } else {
                let be = direct()?;
                println!("daemon      : not running");
                println!("backend     : {}", be.describe());
                println!("brightness  : {}/{}", be.brightness()?, be.max_brightness()?);
                if let Ok(s) = hw::SysfsBackend::open(cli.led.clone().unwrap_or(hw::DEFAULT_LED.into())) {
                    println!("writable    : {}", s.writable());
                }
                println!("config      : {}", config_path().display());
            }
        }
        Cmd::Color { color, save } => {
            let spec = EffectSpec::Hardware { mode: HwMode::Static, color: *color, speed: Speed::Medium, save: *save };
            if daemon {
                ipc::send(&Request::Effect { effect: spec })?;
            } else {
                direct()?.set_mode(HwMode::Static, *color, Speed::Medium, *save)?;
            }
        }
        Cmd::Hw { mode, color, speed, save } => {
            let (mode, speed) = (HwMode::parse(mode)?, Speed::parse(speed)?);
            if daemon {
                ipc::send(&Request::Effect { effect: EffectSpec::Hardware { mode, color: *color, speed, save: *save } })?;
            } else {
                direct()?.set_mode(mode, *color, speed, *save)?;
            }
        }
        Cmd::Effect { kind, params, foreground } => {
            let spec = parse_effect(kind, params)?;
            if daemon && !foreground {
                ipc::send(&Request::Effect { effect: spec })?;
            } else {
                run_foreground(&cli, spec)?;
            }
        }
        Cmd::Effects => {
            for spec in EffectSpec::catalog() {
                let t = toml::to_string(&spec)?;
                let params: Vec<String> =
                    t.lines().filter(|l| !l.starts_with("type")).map(|l| l.replace(" = ", "=")).collect();
                println!("{:<12} {}", spec.kind(), params.join(" "));
            }
        }
        Cmd::Profile { action } => match action {
            ProfileCmd::List => {
                let cfg = Config::load()?;
                for (name, spec) in &cfg.profiles {
                    let mark = if cfg.active.as_deref() == Some(name) { "*" } else { " " };
                    println!("{mark} {name:<16} {}", spec.kind());
                }
            }
            ProfileCmd::Show { name } => {
                let cfg = Config::load()?;
                let spec = cfg.profiles.get(name).with_context(|| format!("no profile '{name}'"))?;
                print!("{}", toml::to_string_pretty(spec)?);
            }
            ProfileCmd::Apply { name } => {
                if daemon {
                    ipc::send(&Request::Profile { name: name.clone() })?;
                } else {
                    let mut cfg = Config::load()?;
                    let spec = cfg.profiles.get(name).cloned().with_context(|| format!("no profile '{name}'"))?;
                    cfg.active = Some(name.clone());
                    cfg.save()?;
                    run_foreground(&cli, spec)?;
                }
            }
            ProfileCmd::Save { name, kind, params } => {
                let spec = parse_effect(kind, params)?;
                if daemon {
                    ipc::send(&Request::SaveProfile { name: name.clone(), effect: spec })?;
                } else {
                    let mut cfg = Config::load()?;
                    cfg.profiles.insert(name.clone(), spec);
                    cfg.save()?;
                }
            }
            ProfileCmd::Delete { name } => {
                if daemon {
                    ipc::send(&Request::DeleteProfile { name: name.clone() })?;
                } else {
                    let mut cfg = Config::load()?;
                    if cfg.profiles.remove(name).is_none() {
                        bail!("no profile '{name}'");
                    }
                    cfg.save()?;
                }
            }
        },
        Cmd::Power { boot, awake, sleep, keyboard, save } => {
            let mut cfg = Config::load()?;
            let on = |v: &Option<OnOff>, cur: bool| v.map(|x| matches!(x, OnOff::On)).unwrap_or(cur);
            let st = PowerState {
                boot: on(boot, cfg.power.boot),
                awake: on(awake, cfg.power.awake),
                sleep: on(sleep, cfg.power.sleep),
                keyboard: on(keyboard, cfg.power.keyboard),
            };
            if daemon {
                ipc::send(&Request::Power { state: st, save: *save })?;
            } else {
                direct()?.set_power_state(st, *save)?;
                cfg.power = st;
                cfg.save()?;
            }
        }
        Cmd::Brightness { level } => match level {
            Some(l) if daemon => {
                ipc::send(&Request::Brightness { level: *l })?;
            }
            Some(l) => direct()?.set_brightness(*l)?,
            None => println!("{}", direct()?.brightness()?),
        },
        Cmd::Notify { color, count, period } => {
            if !daemon {
                bail!("notify needs the daemon (tuf-rgbd)");
            }
            ipc::send(&Request::Notify { color: *color, count: *count, period: *period })?;
        }
        Cmd::Settings { fps, gamma, intensity } => {
            if daemon {
                let st = ipc::send(&Request::Settings { fps: *fps, gamma: *gamma, intensity: *intensity })?;
                print_status(&st.status.unwrap_or_default());
            } else {
                let mut cfg = Config::load()?;
                cfg.fps = fps.unwrap_or(cfg.fps);
                cfg.gamma = gamma.unwrap_or(cfg.gamma);
                cfg.intensity = intensity.unwrap_or(cfg.intensity);
                cfg.save()?;
            }
        }
        Cmd::Stop => {
            ipc::send(&Request::Shutdown)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_args() {
        let s = parse_effect("breathe", &["colors=#ff0000,blue".into(), "period=3".into()]).unwrap();
        assert_eq!(
            s,
            EffectSpec::Breathe { colors: vec![Rgb::new(255, 0, 0), Rgb::new(0, 0, 255)], period: 3.0, min: 0.0 }
        );
        let s = parse_effect("temperature", &["hot=red".into(), "sensor=k10temp".into()]).unwrap();
        assert!(matches!(s, EffectSpec::Temperature { sensor: Some(_), .. }));
        assert!(parse_effect("rainbow", &["period=fast".into()]).is_err());
    }
}
