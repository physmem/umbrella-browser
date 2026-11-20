//! Built-in themes. Community palettes (Tokyo Night, Nord, Catppuccin) use
//! their published colors; muted text picks the palette's own lighter tone
//! where the usual "comment" gray would fail AA.

use super::color::Rgba;
use super::tokens::{Appearance, Backdrop, Effects, Glass, Palette, Shape, ThemeTokens};
use crate::ThemeId;
use crate::spring::SpringParams;

pub const SAFARI_GLASS_DARK: &str = "safari-glass-dark";
pub const TOKYO_NIGHT: &str = "tokyo-night";
pub const NORD: &str = "nord";
pub const CATPPUCCIN_MOCHA: &str = "catppuccin-mocha";

/// The four built-ins, in the order settings shows them.
pub fn all() -> Vec<(ThemeId, ThemeTokens)> {
    [
        (SAFARI_GLASS_DARK, safari_glass_dark()),
        (CATPPUCCIN_MOCHA, catppuccin_mocha()),
        (TOKYO_NIGHT, tokyo_night()),
        (NORD, nord()),
    ]
    .into_iter()
    .map(|(id, tokens)| (ThemeId::new(id), tokens))
    .collect()
}

pub fn default_id() -> ThemeId {
    ThemeId::new(SAFARI_GLASS_DARK)
}

fn shape(sm: f32, md: f32, lg: f32) -> Shape {
    Shape {
        radius_sm: sm,
        radius_md: md,
        radius_lg: lg,
    }
}

fn glass(backdrop: Backdrop, tint: u32, opacity: f32, blur: f32) -> Glass {
    Glass {
        backdrop,
        tint: Rgba::hex(tint),
        opacity,
        blur,
    }
}

pub fn safari_glass_dark() -> ThemeTokens {
    ThemeTokens {
        name: "Safari Glass Dark".into(),
        appearance: Appearance::Dark,
        colors: Palette {
            window: Rgba::hex(0x1c1c1e),
            surface: Rgba::hex(0x2c2c2e),
            surface_raised: Rgba::hex(0x3a3a3c),
            text: Rgba::hex(0xf5f5f7),
            text_muted: Rgba::hex(0xaeaeb2),
            accent: Rgba::hex(0x0071e3),
            on_accent: Rgba::hex(0xffffff),
            border: Rgba::hexa(0xffffff1f),
            focus: Rgba::hex(0x0a84ff),
            selection: Rgba::hexa(0x0a84ff40),
            danger: Rgba::hex(0xff453a),
            warning: Rgba::hex(0xff9f0a),
            success: Rgba::hex(0x30d158),
        },
        glass: glass(Backdrop::Acrylic, 0x1c1c1e, 0.70, 30.0),
        shape: shape(6.0, 10.0, 14.0),
        motion: SpringParams::new(380.0, 30.0),
        effects: Effects::default(),
    }
}

pub fn tokyo_night() -> ThemeTokens {
    ThemeTokens {
        name: "Tokyo Night".into(),
        appearance: Appearance::Dark,
        colors: Palette {
            window: Rgba::hex(0x1a1b26),
            surface: Rgba::hex(0x16161e),
            surface_raised: Rgba::hex(0x292e42),
            text: Rgba::hex(0xc0caf5),
            text_muted: Rgba::hex(0xa9b1d6),
            accent: Rgba::hex(0x7aa2f7),
            on_accent: Rgba::hex(0x1a1b26),
            border: Rgba::hex(0x414868),
            focus: Rgba::hex(0x7aa2f7),
            selection: Rgba::hex(0x283457),
            danger: Rgba::hex(0xf7768e),
            warning: Rgba::hex(0xe0af68),
            success: Rgba::hex(0x9ece6a),
        },
        glass: glass(Backdrop::Mica, 0x1a1b26, 0.85, 24.0),
        shape: shape(5.0, 8.0, 12.0),
        motion: SpringParams::new(400.0, 34.0),
        effects: Effects::default(),
    }
}

pub fn nord() -> ThemeTokens {
    ThemeTokens {
        name: "Nord".into(),
        appearance: Appearance::Dark,
        colors: Palette {
            window: Rgba::hex(0x2e3440),
            surface: Rgba::hex(0x3b4252),
            surface_raised: Rgba::hex(0x434c5e),
            text: Rgba::hex(0xeceff4),
            text_muted: Rgba::hex(0xd8dee9),
            accent: Rgba::hex(0x88c0d0),
            on_accent: Rgba::hex(0x2e3440),
            border: Rgba::hex(0x4c566a),
            focus: Rgba::hex(0x88c0d0),
            selection: Rgba::hex(0x4c566a),
            danger: Rgba::hex(0xbf616a),
            warning: Rgba::hex(0xebcb8b),
            success: Rgba::hex(0xa3be8c),
        },
        glass: glass(Backdrop::Acrylic, 0x2e3440, 0.82, 28.0),
        shape: shape(4.0, 8.0, 12.0),
        motion: SpringParams::new(360.0, 32.0),
        effects: Effects::default(),
    }
}

pub fn catppuccin_mocha() -> ThemeTokens {
    ThemeTokens {
        name: "Catppuccin Mocha".into(),
        appearance: Appearance::Dark,
        colors: Palette {
            window: Rgba::hex(0x1e1e2e),
            surface: Rgba::hex(0x181825),
            surface_raised: Rgba::hex(0x313244),
            text: Rgba::hex(0xcdd6f4),
            text_muted: Rgba::hex(0xa6adc8),
            accent: Rgba::hex(0xcba6f7),
            on_accent: Rgba::hex(0x11111b),
            border: Rgba::hex(0x45475a),
            focus: Rgba::hex(0xb4befe),
            selection: Rgba::hex(0x45475a),
            danger: Rgba::hex(0xf38ba8),
            warning: Rgba::hex(0xf9e2af),
            success: Rgba::hex(0xa6e3a1),
        },
        glass: glass(Backdrop::Mica, 0x1e1e2e, 0.80, 26.0),
        shape: shape(6.0, 10.0, 14.0),
        motion: SpringParams::new(340.0, 28.0),
        effects: Effects::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_valid_and_meets_aa() {
        for (id, theme) in all() {
            assert!(theme.validate().is_ok(), "{id} invalid");
            assert_eq!(theme.contrast_issues(), vec![], "{id} fails contrast");
            assert_eq!(
                theme.solid().contrast_issues(),
                vec![],
                "{id} solid fails contrast"
            );
        }
    }

    #[test]
    fn built_ins_have_no_effects() {
        for (id, theme) in all() {
            assert_eq!(theme.effects, Effects::default(), "{id}");
        }
    }
}
