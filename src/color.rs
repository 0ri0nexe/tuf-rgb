use std::fmt;
use std::str::FromStr;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 8-bit RGB color. Serialized as "#rrggbb".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const BLACK: Rgb = Rgb::new(0, 0, 0);
pub const WHITE: Rgb = Rgb::new(255, 255, 255);

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Hue in degrees, saturation and value in 0..=1.
    pub fn from_hsv(h: f64, s: f64, v: f64) -> Self {
        let h = h.rem_euclid(360.0) / 60.0;
        let c = v * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        Self::from_f64(r + m, g + m, b + m)
    }

    pub fn from_f64(r: f64, g: f64, b: f64) -> Self {
        let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::new(q(r), q(g), q(b))
    }

    pub fn to_f64(self) -> (f64, f64, f64) {
        (self.r as f64 / 255.0, self.g as f64 / 255.0, self.b as f64 / 255.0)
    }

    /// Linear interpolation, t in 0..=1.
    pub fn lerp(self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let (a, b) = (self.to_f64(), other.to_f64());
        Self::from_f64(
            a.0 + (b.0 - a.0) * t,
            a.1 + (b.1 - a.1) * t,
            a.2 + (b.2 - a.2) * t,
        )
    }

    pub fn scale(self, k: f64) -> Rgb {
        let (r, g, b) = self.to_f64();
        Self::from_f64(r * k, g * k, b * k)
    }

    /// Per-channel gamma correction (LEDs are roughly linear in PWM, eyes are not).
    pub fn gamma(self, g: f64) -> Rgb {
        if (g - 1.0).abs() < f64::EPSILON {
            return self;
        }
        let (r, gr, b) = self.to_f64();
        Self::from_f64(r.powf(g), gr.powf(g), b.powf(g))
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// Sample a palette as a closed loop at position t in 0..1.
pub fn palette_at(colors: &[Rgb], t: f64) -> Rgb {
    match colors.len() {
        0 => BLACK,
        1 => colors[0],
        n => {
            let p = t.rem_euclid(1.0) * n as f64;
            let i = p.floor() as usize % n;
            colors[i].lerp(colors[(i + 1) % n], p.fract())
        }
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

const NAMED: &[(&str, Rgb)] = &[
    ("black", Rgb::new(0, 0, 0)),
    ("off", Rgb::new(0, 0, 0)),
    ("white", Rgb::new(255, 255, 255)),
    ("red", Rgb::new(255, 0, 0)),
    ("green", Rgb::new(0, 255, 0)),
    ("blue", Rgb::new(0, 0, 255)),
    ("yellow", Rgb::new(255, 255, 0)),
    ("cyan", Rgb::new(0, 255, 255)),
    ("magenta", Rgb::new(255, 0, 255)),
    ("orange", Rgb::new(255, 100, 0)),
    ("purple", Rgb::new(128, 0, 255)),
    ("pink", Rgb::new(255, 40, 120)),
    ("teal", Rgb::new(0, 200, 160)),
    ("warm", Rgb::new(255, 140, 40)),
    ("tuf", Rgb::new(255, 60, 0)),
];

impl FromStr for Rgb {
    type Err = anyhow::Error;

    /// Accepts "#rrggbb", "rrggbb", "#rgb", "r,g,b" and a few names.
    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        if let Some((_, c)) = NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)) {
            return Ok(*c);
        }
        if s.contains(',') {
            let parts: Vec<u8> = s
                .split(',')
                .map(|p| p.trim().parse::<u8>())
                .collect::<Result<_, _>>()
                .map_err(|_| anyhow!("invalid color '{s}'"))?;
            if let [r, g, b] = parts[..] {
                return Ok(Rgb::new(r, g, b));
            }
            bail!("invalid color '{s}'");
        }
        let h = s.trim_start_matches('#');
        let byte = |i: usize, len: usize| u8::from_str_radix(&h[i..i + len], 16);
        match h.len() {
            6 => Ok(Rgb::new(byte(0, 2)?, byte(2, 2)?, byte(4, 2)?)),
            3 => {
                let n = |i| byte(i, 1).map(|v| v * 17);
                Ok(Rgb::new(n(0)?, n(1)?, n(2)?))
            }
            _ => bail!("invalid color '{s}' (expected #rrggbb, r,g,b or a name)"),
        }
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        assert_eq!("#ff8000".parse::<Rgb>().unwrap(), Rgb::new(255, 128, 0));
        assert_eq!("f00".parse::<Rgb>().unwrap(), Rgb::new(255, 0, 0));
        assert_eq!("1, 2,3".parse::<Rgb>().unwrap(), Rgb::new(1, 2, 3));
        assert_eq!("Cyan".parse::<Rgb>().unwrap(), Rgb::new(0, 255, 255));
        assert!("#12345".parse::<Rgb>().is_err());
    }

    #[test]
    fn hsv() {
        assert_eq!(Rgb::from_hsv(0.0, 1.0, 1.0), Rgb::new(255, 0, 0));
        assert_eq!(Rgb::from_hsv(120.0, 1.0, 1.0), Rgb::new(0, 255, 0));
        assert_eq!(Rgb::from_hsv(240.0, 1.0, 1.0), Rgb::new(0, 0, 255));
        assert_eq!(Rgb::from_hsv(360.0, 1.0, 1.0), Rgb::new(255, 0, 0));
    }

    #[test]
    fn palette() {
        let p = [Rgb::new(0, 0, 0), Rgb::new(200, 0, 0)];
        assert_eq!(palette_at(&p, 0.25), Rgb::new(100, 0, 0));
        assert_eq!(palette_at(&p, 0.75), Rgb::new(100, 0, 0));
        assert_eq!(palette_at(&p, 1.0), Rgb::new(0, 0, 0));
    }
}
