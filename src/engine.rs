use std::time::{Duration, Instant};

use anyhow::Result;

use crate::color::Rgb;
use crate::config::Config;
use crate::effects::{Effect, EffectSpec, Flash};
use crate::hw::{Backend, HwMode, Speed};

/// Owns the backend and renders the current effect.
pub struct Engine {
    pub backend: Box<dyn Backend>,
    pub config: Config,
    pub spec: Option<EffectSpec>,
    fx: Option<Box<dyn Effect>>,
    epoch: Instant,
    started: f64,
    last_frame: f64,
    flash: Option<Flash>,
    pub last_color: Option<Rgb>,
    pub last_error: Option<String>,
}

impl Engine {
    pub fn new(backend: Box<dyn Backend>, config: Config) -> Self {
        Self {
            backend,
            config,
            spec: None,
            fx: None,
            epoch: Instant::now(),
            started: 0.0,
            last_frame: 0.0,
            flash: None,
            last_color: None,
            last_error: None,
        }
    }

    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// Switch to a new effect. Hardware effects are written once here.
    pub fn apply(&mut self, spec: EffectSpec) -> Result<()> {
        self.started = self.now();
        self.last_frame = self.started;
        match &spec {
            EffectSpec::Hardware { mode, color, speed, save } => {
                self.backend.set_mode(*mode, *color, *speed, *save)?;
                self.last_color = Some(*color);
                self.fx = None;
            }
            _ => {
                self.fx = Some(spec.build());
            }
        }
        self.spec = Some(spec);
        self.render()?;
        Ok(())
    }

    pub fn apply_profile(&mut self, name: &str) -> Result<()> {
        let spec = self
            .config
            .profiles
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no profile named '{name}'"))?;
        self.apply(spec)?;
        self.config.active = Some(name.to_string());
        Ok(())
    }

    pub fn notify(&mut self, color: Rgb, count: u32, period: f64) {
        self.flash = Some(Flash { color, count: count.max(1), period: period.max(0.05), started: self.now() });
    }

    /// Render one frame if needed and return how long to wait before the next one.
    pub fn render(&mut self) -> Result<Duration> {
        let now = self.now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        let frame = Duration::from_secs_f64(1.0 / self.config.fps.clamp(1.0, 120.0));

        let overlay = self.flash.as_ref().and_then(|f| f.color_at(now));
        if overlay.is_none() && self.flash.take().is_some() {
            // Overlay just ended: restore a firmware mode that the flash overwrote.
            if let Some(EffectSpec::Hardware { mode, color, speed, .. }) = self.spec.clone() {
                self.backend.set_mode(mode, color, speed, false)?;
                return Ok(Duration::from_secs(3600));
            }
        }

        let color = match (overlay, &mut self.fx) {
            (Some(c), _) => c,
            (None, Some(fx)) => fx.frame(now - self.started, dt),
            (None, None) => return Ok(if self.flash.is_some() { frame } else { Duration::from_secs(3600) }),
        };
        let out = color.scale(self.config.intensity).gamma(self.config.gamma);
        self.backend.set_mode(HwMode::Static, out, Speed::Medium, false)?;
        self.last_color = Some(color);

        let animated = overlay.is_some() || self.spec.as_ref().is_some_and(EffectSpec::animated);
        Ok(if animated { frame } else { Duration::from_secs(3600) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::DryRunBackend;

    #[test]
    fn hardware_then_software() {
        let be = DryRunBackend { quiet: true, ..Default::default() };
        let mut e = Engine::new(Box::new(be), Config::default());
        e.apply(toml::from_str("type='hardware'\nmode='strobe'\ncolor='#00ff00'\nspeed='fast'").unwrap())
            .unwrap();
        assert!(e.render().unwrap() > Duration::from_secs(60));
        e.apply_profile("rainbow").unwrap();
        assert!(e.render().unwrap() < Duration::from_secs(1));
    }
}
