//! Color core — RGB struct, HSL/ARGB/hex conversion, hue shift, surface adjust, and lerp_hsv.
//! Port of `src/theme/color.{cpp,h}` with commit `73c7c91c` achromatic HSV fix.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ParseColorError {
    #[error("invalid hex string length (expected 6 or 7 with '#')")]
    InvalidLength,
    #[error("invalid hex digit")]
    InvalidDigit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const WHITE: Self = Self::new(255, 255, 255);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn from_hex(mut hex: &str) -> Result<Self, ParseColorError> {
        if let Some(stripped) = hex.strip_prefix('#') {
            hex = stripped;
        }
        if hex.len() != 6 {
            return Err(ParseColorError::InvalidLength);
        }

        let parse_byte = |s: &str| -> Result<u8, ParseColorError> {
            u8::from_str_radix(s, 16).map_err(|_| ParseColorError::InvalidDigit)
        };

        let r = parse_byte(&hex[0..2])?;
        let g = parse_byte(&hex[2..4])?;
        let b = parse_byte(&hex[4..6])?;

        Ok(Self::new(r, g, b))
    }

    pub fn from_argb(argb: u32) -> Self {
        Self::new(
            ((argb >> 16) & 0xff) as u8,
            ((argb >> 8) & 0xff) as u8,
            (argb & 0xff) as u8,
        )
    }

    pub fn to_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn to_argb(&self) -> u32 {
        0xff00_0000 | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    pub fn to_hsl(&self) -> (f64, f64, f64) {
        let rn = self.r as f64 / 255.0;
        let gn = self.g as f64 / 255.0;
        let bn = self.b as f64 / 255.0;
        let max_c = rn.max(gn).max(bn);
        let min_c = rn.min(gn).min(bn);
        let delta = max_c - min_c;

        let l = (max_c + min_c) / 2.0;
        let mut h = 0.0;
        let mut s = 0.0;

        if delta != 0.0 {
            if l != 0.0 && l != 1.0 {
                s = delta / (1.0 - (2.0 * l - 1.0).abs());
            }
            if max_c == rn {
                let mut t = ((gn - bn) / delta) % 6.0;
                if t < 0.0 {
                    t += 6.0;
                }
                h = 60.0 * t;
            } else if max_c == gn {
                h = 60.0 * (((bn - rn) / delta) + 2.0);
            } else {
                h = 60.0 * (((rn - gn) / delta) + 4.0);
            }
        }
        (h, s, l)
    }

    pub fn from_hsl(h: f64, s: f64, l: f64) -> Self {
        if s == 0.0 {
            let v = round_clamp_255(l);
            return Color::new(v, v, v);
        }
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        let hn = h / 360.0;

        let hue_to_rgb = |t: f64| -> f64 {
            let mut t = t;
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 1.0 / 2.0 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };

        Color::new(
            round_clamp_255(hue_to_rgb(hn + 1.0 / 3.0)),
            round_clamp_255(hue_to_rgb(hn)),
            round_clamp_255(hue_to_rgb(hn - 1.0 / 3.0)),
        )
    }
}

fn round_clamp_255(v: f64) -> u8 {
    let r = (v * 255.0).round() as i64;
    r.clamp(0, 255) as u8
}

pub fn hue_distance(h1: f64, h2: f64) -> f64 {
    let diff = (h1 - h2).abs();
    diff.min(360.0 - diff)
}

pub fn shift_hue(c: &Color, degrees: f64) -> Color {
    let (h, s, l) = c.to_hsl();
    let mut new_h = (h + degrees) % 360.0;
    if new_h < 0.0 {
        new_h += 360.0;
    }
    Color::from_hsl(new_h, s, l)
}

pub fn adjust_surface(base: &Color, s_max: f64, l_target: f64) -> Color {
    let (h, s, _l) = base.to_hsl();
    Color::from_hsl(h, s.min(s_max), l_target)
}

pub fn rgb_to_hsv(c: &Color) -> (f32, f32, f32) {
    let r = c.r as f32 / 255.0;
    let g = c.g as f32 / 255.0;
    let b = c.b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let v = max;
    let s = if max == 0.0 { 0.0 } else { delta / max };

    let mut h = if delta == 0.0 {
        0.0
    } else if max == r {
        (g - b) / delta
    } else if max == g {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };

    h /= 6.0;
    if h < 0.0 {
        h += 1.0;
    }
    (h, s, v)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color {
    let h = (h % 1.0 + 1.0) % 1.0;
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);

    let (r, g, b) = match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };

    Color::new(
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// Blends from `a` to `b` through HSV space on shortest hue path, with commit `73c7c91c` achromatic fix.
pub fn lerp_hsv(a: &Color, b: &Color, t: f32) -> Color {
    let (mut h0, s0, v0) = rgb_to_hsv(a);
    let (mut h1, s1, v1) = rgb_to_hsv(b);

    const CHROMA_EPSILON: f32 = 1e-6;
    if s0 * v0 <= CHROMA_EPSILON {
        h0 = h1;
    }
    if s1 * v1 <= CHROMA_EPSILON {
        h1 = h0;
    }

    let mut dh = h1 - h0;
    if dh > 0.5 {
        dh -= 1.0;
    } else if dh < -0.5 {
        dh += 1.0;
    }

    let h = (h0 + dh * t) % 1.0;
    let s = s0 + (s1 - s0) * t;
    let v = v0 + (v1 - v0) * t;

    hsv_to_rgb(h, s, v)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let c = Color::from_hex("#ff8000").unwrap();
        assert_eq!(c, Color::new(255, 128, 0));
        assert_eq!(c.to_hex(), "#ff8000");

        let c2 = Color::from_hex("003366").unwrap();
        assert_eq!(c2, Color::new(0, 51, 102));
        assert_eq!(c2.to_hex(), "#003366");

        assert!(Color::from_hex("#fff").is_err());
        assert!(Color::from_hex("#zzzzzz").is_err());
    }

    #[test]
    fn argb_conversions() {
        let c = Color::new(0x12, 0x34, 0x56);
        let argb = c.to_argb();
        assert_eq!(argb, 0xff123456);
        assert_eq!(Color::from_argb(argb), c);
    }

    #[test]
    fn hsl_round_trips() {
        let c = Color::new(255, 0, 0);
        let (h, s, l) = c.to_hsl();
        assert_eq!(h, 0.0);
        assert_eq!(s, 1.0);
        assert_eq!(l, 0.5);
        assert_eq!(Color::from_hsl(h, s, l), c);

        let green = Color::new(0, 255, 0);
        let (gh, gs, gl) = green.to_hsl();
        assert!((gh - 120.0).abs() < 1e-4);
        assert_eq!(Color::from_hsl(gh, gs, gl), green);
    }

    #[test]
    fn hue_distance_calculation() {
        assert_eq!(hue_distance(10.0, 30.0), 20.0);
        assert_eq!(hue_distance(350.0, 10.0), 20.0);
        assert_eq!(hue_distance(0.0, 180.0), 180.0);
    }

    #[test]
    fn shift_hue_preserves_saturation_and_lightness() {
        let c = Color::new(255, 0, 0);
        let shifted = shift_hue(&c, 120.0);
        assert_eq!(shifted, Color::new(0, 255, 0));
    }

    #[test]
    fn achromatic_hsv_interpolation_prevents_spurious_tints() {
        let gray = Color::new(128, 128, 128); // s0 = 0
        let red = Color::new(255, 0, 0);

        let blended = lerp_hsv(&gray, &red, 0.5);
        // achromatic endpoint borrows red's hue, avoiding tint artifacts
        assert!(blended.r > blended.g && blended.r > blended.b);
    }
}
