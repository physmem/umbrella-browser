use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("`{0}` is not a #rrggbb or #rrggbbaa color")]
pub struct ColorError(String);

/// 8-bit sRGB color with straight alpha. Serialized as `#rrggbb[aa]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// From `0xRRGGBB`, fully opaque.
    pub const fn hex(rgb: u32) -> Self {
        Self::hexa((rgb << 8) | 0xff)
    }

    /// From `0xRRGGBBAA`.
    pub const fn hexa(rgba: u32) -> Self {
        let [r, g, b, a] = rgba.to_be_bytes();
        Self { r, g, b, a }
    }

    pub fn parse(text: &str) -> Result<Self, ColorError> {
        let err = || ColorError(text.to_owned());
        let digits = text.strip_prefix('#').ok_or_else(err)?;
        let value = u32::from_str_radix(digits, 16).map_err(|_| err())?;
        match digits.len() {
            6 => Ok(Self::hex(value)),
            8 => Ok(Self::hexa(value)),
            _ => Err(err()),
        }
    }

    pub fn to_hex(self) -> String {
        if self.a == 0xff {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// Hue in degrees (0..360), saturation and lightness in 0..=1.
    pub fn to_hsl(self) -> (f32, f32, f32) {
        let [r, g, b] = [self.r, self.g, self.b].map(|c| f32::from(c) / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        if d <= f32::EPSILON {
            return (0.0, 0.0, l);
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            60.0 * ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        (h, s.clamp(0.0, 1.0), l)
    }

    /// Hue in degrees, saturation and value (brightness) in 0..=1. The
    /// colour picker's square is saturation across and value down.
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let (h, s_l, l) = self.to_hsl();
        let v = l + s_l * l.min(1.0 - l);
        let s = if v <= f32::EPSILON {
            0.0
        } else {
            2.0 * (1.0 - l / v)
        };
        (h, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
    }

    /// Inverse of `to_hsv`, keeping this colour's alpha.
    pub fn with_hsv(self, h: f32, s: f32, v: f32) -> Self {
        let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        let l = v * (1.0 - s / 2.0);
        let s_l = if l <= f32::EPSILON || l >= 1.0 - f32::EPSILON {
            0.0
        } else {
            (v - l) / l.min(1.0 - l)
        };
        self.with_hsl(h, s_l, l)
    }

    /// Inverse of `to_hsl`, keeping this colour's alpha.
    pub fn with_hsl(self, h: f32, s: f32, l: f32) -> Self {
        let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let hp = h.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        Self {
            r: to_channel(r + m),
            g: to_channel(g + m),
            b: to_channel(b + m),
            a: self.a,
        }
    }

    pub fn alpha(self) -> f32 {
        f32::from(self.a) / 255.0
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        Self {
            a: to_channel(alpha),
            ..self
        }
    }

    /// Source-over compositing onto an opaque `base`.
    pub fn over(self, base: Rgba) -> Rgba {
        let a = self.alpha();
        let mix = |top: u8, bottom: u8| to_channel(unit(top) * a + unit(bottom) * (1.0 - a));
        Rgba {
            r: mix(self.r, base.r),
            g: mix(self.g, base.g),
            b: mix(self.b, base.b),
            a: 0xff,
        }
    }

    /// WCAG 2.x relative luminance, ignoring alpha.
    pub fn luminance(self) -> f32 {
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    pub fn contrast(self, other: Rgba) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

fn unit(channel: u8) -> f32 {
    f32::from(channel) / 255.0
}

fn to_channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn linear(channel: u8) -> f32 {
    let c = unit(channel);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl fmt::Debug for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl TryFrom<String> for Rgba {
    type Error = ColorError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<Rgba> for String {
    fn from(color: Rgba) -> Self {
        color.to_hex()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hsv_round_trips() {
        for hex in [
            0x0071e3, 0x88c0d0, 0x1e1e2e, 0xf5f5f7, 0x000000, 0xcba6f7, 0xffffff,
        ] {
            let c = super::Rgba::hex(hex);
            let (h, s, v) = c.to_hsv();
            let back = c.with_hsv(h, s, v);
            for (a, b) in [(c.r, back.r), (c.g, back.g), (c.b, back.b)] {
                assert!(a.abs_diff(b) <= 1, "{hex:06x}: {c:?} -> {back:?}");
            }
        }
        let red = super::Rgba::hex(0x000000).with_hsv(0.0, 1.0, 1.0);
        assert_eq!(red, super::Rgba::hex(0xff0000));
    }

    #[test]
    fn hsl_round_trips() {
        for hex in [0x0071e3, 0x88c0d0, 0x1e1e2e, 0xf5f5f7, 0x000000, 0xcba6f7] {
            let c = super::Rgba::hex(hex);
            let (h, s, l) = c.to_hsl();
            let back = c.with_hsl(h, s, l);
            for (a, b) in [(c.r, back.r), (c.g, back.g), (c.b, back.b)] {
                assert!(a.abs_diff(b) <= 1, "{hex:06x}: {c:?} -> {back:?}");
            }
        }
    }

    use super::*;

    #[test]
    fn parses_and_prints() {
        assert_eq!(Rgba::parse("#0066cc"), Ok(Rgba::hex(0x0066cc)));
        assert_eq!(Rgba::parse("#00000080").map(|c| c.a), Ok(0x80));
        assert_eq!(Rgba::hexa(0x11223344).to_hex(), "#11223344");
        assert!(Rgba::parse("0066cc").is_err());
        assert!(Rgba::parse("#06c").is_err());
    }

    #[test]
    fn black_on_white_is_21() {
        let ratio = Rgba::hex(0x000000).contrast(Rgba::hex(0xffffff));
        assert!((ratio - 21.0).abs() < 0.01);
    }

    #[test]
    fn half_black_over_white_is_mid_gray() {
        let gray = Rgba::hex(0x000000)
            .with_alpha(0.5)
            .over(Rgba::hex(0xffffff));
        // 0.5 alpha quantizes to 128/255, so either neighbor is correct.
        assert!((127..=128).contains(&gray.r));
    }
}
