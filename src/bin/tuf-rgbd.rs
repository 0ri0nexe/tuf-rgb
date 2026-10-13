//! tuf-rgbd: renders software effects and serves CLI/GUI requests over a Unix socket.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tuf_rgb::config::Config;
use tuf_rgb::engine::Engine;
use tuf_rgb::hw;
use tuf_rgb::ipc::{self, Request, Response, Status};

#[derive(Parser)]
#[command(version, about = "ASUS TUF keyboard lighting daemon")]
struct Args {
    /// Do not touch the hardware, print writes instead.
    #[arg(long)]
    dry_run: bool,
    /// LED class directory.
    #[arg(long)]
    led: Option<PathBuf>,
    /// Start with this profile instead of the configured one.
    #[arg(long)]
    profile: Option<String>,
}

enum Msg {
    Req(Request, Sender<Response>),
    Quit,
}

fn serve_client(stream: UnixStream, tx: Sender<Msg>) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut out = stream;
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(req) => {
                let (rtx, rrx) = mpsc::channel();
                tx.send(Msg::Req(req, rtx))?;
                rrx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| Response::err("timeout"))
            }
            Err(e) => Response::err(format!("bad request: {e}")),
        };
        let mut s = serde_json::to_string(&resp)?;
        s.push('\n');
        out.write_all(s.as_bytes())?;
        line.clear();
    }
    Ok(())
}

fn status(e: &Engine) -> Status {
    Status {
        backend: e.backend.describe(),
        effect: e.spec.clone(),
        active_profile: e.config.active.clone(),
        profiles: e.config.profiles.keys().cloned().collect(),
        color: e.last_color,
        brightness: e.backend.brightness().ok(),
        fps: e.config.fps,
        gamma: e.config.gamma,
        intensity: e.config.intensity,
        power: Some(e.config.power),
        last_error: e.last_error.clone(),
    }
}

fn handle(e: &mut Engine, req: Request) -> Result<Option<Status>> {
    match req {
        Request::Status => {}
        Request::Effect { effect } => e.apply(effect)?,
        Request::Profile { name } => {
            e.apply_profile(&name)?;
            e.config.save()?;
        }
        Request::SaveProfile { name, effect } => {
            e.config.profiles.insert(name, effect);
            e.config.save()?;
        }
        Request::DeleteProfile { name } => {
            e.config.profiles.remove(&name);
            if e.config.active.as_deref() == Some(name.as_str()) {
                e.config.active = None;
            }
            e.config.save()?;
        }
        Request::Notify { color, count, period } => e.notify(color, count, period),
        Request::Power { state, save } => {
            e.backend.set_power_state(state, save)?;
            e.config.power = state;
            e.config.save()?;
        }
        Request::Brightness { level } => e.backend.set_brightness(level)?,
        Request::Settings { fps, gamma, intensity } => {
            if let Some(v) = fps {
                e.config.fps = v.clamp(1.0, 120.0);
            }
            if let Some(v) = gamma {
                e.config.gamma = v.clamp(0.2, 4.0);
            }
            if let Some(v) = intensity {
                e.config.intensity = v.clamp(0.0, 1.0);
            }
            e.config.save()?;
            // Re-render static effects with the new settings.
            e.render()?;
        }
        Request::Reload => {
            e.config = Config::load()?;
            if let Some(name) = e.config.active.clone() {
                e.apply_profile(&name)?;
            }
        }
        Request::Shutdown => return Ok(None),
    }
    Ok(Some(status(e)))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let backend = hw::open_backend(args.dry_run, args.led.as_deref())?;
    let config = Config::load()?;
    let mut engine = Engine::new(backend, config);

    let start = args.profile.clone().or_else(|| engine.config.active.clone());
    if let Some(name) = start {
        if let Err(err) = engine.apply_profile(&name) {
            eprintln!("tuf-rgbd: cannot apply profile '{name}': {err:#}");
        }
    }

    let sock = ipc::socket_path();
    if ipc::daemon_running() {
        anyhow::bail!("another tuf-rgbd is already listening on {}", sock.display());
    }
    let _ = fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).with_context(|| format!("bind {}", sock.display()))?;
    fs::set_permissions(&sock, fs::Permissions::from_mode(0o600))?;
    eprintln!("tuf-rgbd: {} | socket {}", engine.backend.describe(), sock.display());

    let (tx, rx) = mpsc::channel::<Msg>();
    let ltx = tx.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = ltx.clone();
            thread::spawn(move || {
                let _ = serve_client(stream, tx);
            });
        }
    });
    let qtx = tx.clone();
    ctrlc::set_handler(move || {
        let _ = qtx.send(Msg::Quit);
    })?;

    let mut wait = Duration::ZERO;
    loop {
        match rx.recv_timeout(wait) {
            Ok(Msg::Req(req, reply)) => {
                let resp = match handle(&mut engine, req) {
                    Ok(Some(st)) => Response::ok(Some(st)),
                    Ok(None) => {
                        let _ = reply.send(Response::ok(None));
                        break;
                    }
                    Err(err) => Response::err(format!("{err:#}")),
                };
                let _ = reply.send(resp);
                // Keep the frame cadence: render right away after a change.
                wait = Duration::ZERO;
            }
            Ok(Msg::Quit) => break,
            Err(RecvTimeoutError::Timeout) => {
                wait = match engine.render() {
                    Ok(w) => {
                        engine.last_error = None;
                        w
                    }
                    Err(err) => {
                        // Typically a permission problem; retry slowly instead of spinning.
                        let msg = format!("{err:#}");
                        if engine.last_error.as_deref() != Some(msg.as_str()) {
                            eprintln!("tuf-rgbd: {msg}");
                        }
                        engine.last_error = Some(msg);
                        Duration::from_secs(2)
                    }
                };
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = fs::remove_file(&sock);
    Ok(())
}
