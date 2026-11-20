use serde::{Deserialize, Serialize};

use super::ThemeError;
use super::color::Rgba;
use crate::spring::SpringParams;

/// WCAG AA for normal-size chrome text.
pub const MIN_TEXT_CONTRAST: f32 = 4.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    Light,
    Dark,
}

/// Maps to `DWMWA_SYSTEMBACKDROP_TYPE`. `None` means a solid window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backdrop {
    None,
    Mica,
    MicaAlt,
    Acrylic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub window: Rgba,
    pub surface: Rgba,
    pub surface_raised: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub on_accent: Rgba,
    pub border: Rgba,
    pub focus: Rgba,
    pub selection: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub success: Rgba,
}

/// The frosted layer laid over the DWM backdrop for tab strip and toolbar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Glass {
    pub backdrop: Backdrop,
    pub tint: Rgba,
    /// Tint opacity, 0 is clear glass, 1 is solid.
    pub opacity: f32,
    /// Blur radius in px for in-app frosted layers such as popovers.
    pub blur: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effects {
    #[serde(default)]
    pub glow: f32,
    #[serde(default)]
    pub scanlines: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeTokens {
    pub name: String,
    pub appearance: Appearance,
    pub colors: Palette,
    pub glass: Glass,
    pub shape: Shape,
    pub motion: SpringParams,
    #[serde(default)]
    pub effects: Effects,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContrastIssue {
    pub pair: &'static str,
    pub ratio: f32,
}

impl ThemeTokens {
    pub fn validate(&self) -> Result<(), ThemeError> {
        let unit = |v: f32| (0.0..=1.0).contains(&v);
        if !unit(self.glass.opacity) {
            return Err(ThemeError::Invalid("glass.opacity must be within 0..=1"));
        }
        if !unit(self.effects.glow) || !unit(self.effects.scanlines) {
            return Err(ThemeError::Invalid("effects must be within 0..=1"));
        }
        let s = &self.shape;
        if self.glass.blur < 0.0 || s.radius_sm < 0.0 || s.radius_md < 0.0 || s.radius_lg < 0.0 {
            return Err(ThemeError::Invalid("blur and radii must not be negative"));
        }
        if !self.motion.is_valid() {
            return Err(ThemeError::Invalid(
                "motion needs stiffness > 0, damping >= 0, mass > 0",
            ));
        }
        Ok(())
    }

    /// The reduced-glass variant: no backdrop, opaque tint, no blur, no effects.
    pub fn solid(&self) -> Self {
        let mut solid = self.clone();
        solid.glass.backdrop = Backdrop::None;
        solid.glass.opacity = 1.0;
        solid.glass.blur = 0.0;
        solid.effects = Effects::default();
        solid
    }

    /// The glass chrome over a given backdrop. The real pixels depend on
    /// what sits behind the window, so contrast is checked over both the
    /// theme's window color and pure black (a dark desktop or app behind).
    pub fn chrome_surface_over(&self, backdrop: Rgba) -> Rgba {
        self.glass
            .tint
            .with_alpha(self.glass.opacity)
            .over(backdrop.with_alpha(1.0))
    }

    pub fn contrast_issues(&self) -> Vec<ContrastIssue> {
        let c = &self.colors;
        let chrome = self.chrome_surface_over(c.window);
        let chrome_dark = self.chrome_surface_over(Rgba::hex(0x000000));
        let pairs = [
            ("text on chrome", c.text, chrome),
            ("muted text on chrome", c.text_muted, chrome),
            ("text on chrome over a dark backdrop", c.text, chrome_dark),
            (
                "muted text on chrome over a dark backdrop",
                c.text_muted,
                chrome_dark,
            ),
            ("text on surface", c.text, c.surface),
            ("muted text on surface", c.text_muted, c.surface),
            ("text on raised surface", c.text, c.surface_raised),
            (
                "muted text on raised surface",
                c.text_muted,
                c.surface_raised,
            ),
            ("text on accent", c.on_accent, c.accent),
        ];
        pairs
            .into_iter()
            .map(|(pair, fg, bg)| ContrastIssue {
                pair,
                ratio: fg.contrast(bg),
            })
            .filter(|issue| issue.ratio < MIN_TEXT_CONTRAST)
            .collect()
    }
}
