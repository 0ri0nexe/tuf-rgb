//! JSON-lines protocol between the daemon and its clients over a Unix socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::color::Rgb;
use crate::effects::EffectSpec;
use crate::hw::PowerState;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    Status,
    /// Run an effect now (not persisted).
    Effect { effect: EffectSpec },
    /// Run a named profile and remember it as the active one.
    Profile { name: String },
    /// Store a profile in the config.
    SaveProfile { name: String, effect: EffectSpec },
    DeleteProfile { name: String },
    /// Temporary blink overlay, then resume the running effect.
    Notify { color: Rgb, count: u32, period: f64 },
    Power { state: PowerState, save: bool },
    Brightness { level: u8 },
    Settings { fps: Option<f64>, gamma: Option<f64>, intensity: Option<f64> },
    /// Re-read the config file.
    Reload,
    Shutdown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Status {
    pub backend: String,
    pub effect: Option<EffectSpec>,
    pub active_profile: Option<String>,
    pub profiles: Vec<String>,
    pub color: Option<Rgb>,
    pub brightness: Option<u8>,
    pub fps: f64,
    pub gamma: f64,
    pub intensity: f64,
    pub power: Option<PowerState>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

impl Response {
    pub fn ok(status: Option<Status>) -> Self {
        Self { ok: true, error: None, status }
    }
    pub fn err(e: impl ToString) -> Self {
        Self { ok: false, error: Some(e.to_string()), status: None }
    }
}

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("TUF_RGB_SOCKET") {
        return p.into();
    }
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("tuf-rgb.sock")
}

pub fn send(req: &Request) -> Result<Response> {
    let path = socket_path();
    let mut s = UnixStream::connect(&path)
        .with_context(|| format!("daemon not reachable at {} (start tuf-rgbd)", path.display()))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut buf = String::new();
    BufReader::new(s).read_line(&mut buf)?;
    let resp: Response = serde_json::from_str(&buf).context("invalid daemon response")?;
    if !resp.ok {
        bail!("{}", resp.error.unwrap_or_else(|| "daemon error".into()));
    }
    Ok(resp)
}

pub fn daemon_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}
