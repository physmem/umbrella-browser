//! Turns theme tokens into GPUI values. Views never hard-code colors.

use browser_core::theme::{Backdrop, Rgba, ThemeTokens};
use gpui::{Hsla, WindowBackgroundAppearance};

pub const UI_FONT: &str = "Segoe UI Variable Text";
pub const ICON_FONT: &str = "Segoe Fluent Icons";

pub const TAB_ROW: f32 = 36.0;
pub const TOOLBAR_ROW: f32 = 40.0;
pub const CHROME_HEIGHT: f32 = TAB_ROW + TOOLBAR_ROW;
pub const CAPTION_BUTTON: f32 = 46.0;

/// Segoe Fluent Icons code points; the same glyphs Windows 11 uses.
pub mod icon {
    pub const BACK: &str = "\u{E72B}";
    pub const FORWARD: &str = "\u{E72A}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const STOP: &str = "\u{E711}";
    pub const ADD: &str = "\u{E710}";
    pub const REMOVE: &str = "\u{E738}";
    pub const CLOSE: &str = "\u{E8BB}";
    pub const MINIMIZE: &str = "\u{E921}";
    pub const MAXIMIZE: &str = "\u{E922}";
    pub const RESTORE: &str = "\u{E923}";
    pub const GLOBE: &str = "\u{E774}";
    pub const VOLUME: &str = "\u{E767}";
    pub const MUTE: &str = "\u{E74F}";
    pub const LOCK: &str = "\u{E72E}";
    pub const SHIELD: &str = "\u{EA18}";
    pub const STAR: &str = "\u{E734}";
    pub const SEARCH: &str = "\u{E721}";
    pub const HISTORY: &str = "\u{E81C}";
    pub const TAB: &str = "\u{E8AB}";
    pub const COMMAND: &str = "\u{E756}";
    pub const KEY: &str = "\u{E8D7}";
    pub const DOWNLOAD: &str = "\u{E896}";
    pub const WORKSPACE: &str = "\u{E81E}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const PIN: &str = "\u{E718}";
    pub const PICTURE: &str = "\u{E8B9}";
    pub const CHECK: &str = "\u{E73E}";
    pub const INFO: &str = "\u{E946}";
    pub const PALETTE: &str = "\u{E790}";
    pub const PUZZLE: &str = "\u{EA86}";
    pub const UP: &str = "\u{E70E}";
    pub const DOWN: &str = "\u{E70D}";
    pub const READER: &str = "\u{E736}";
    pub const SPLIT: &str = "\u{E8A9}";
    pub const STAR_FILLED: &str = "\u{E735}";
}

pub fn hsla(color: Rgba) -> Hsla {
    let hex = u32::from_be_bytes([color.r, color.g, color.b, color.a]);
    gpui::rgb_to_hsla(gpui::rgba(hex))
}

/// Blends `from` toward `to` by `t` in straight RGB, alpha included, so a
/// spring can carry a colour change instead of snapping it.
pub fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = (gpui::hsla_to_rgba(from), gpui::hsla_to_rgba(to));
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    gpui::rgb_to_hsla(gpui::Rgba::new(
        lerp(a.red, b.red),
        lerp(a.green, b.green),
        lerp(a.blue, b.blue),
        lerp(a.alpha, b.alpha),
    ))
}

/// A CSS `rgba()` string, for UI the chrome hands to a page.
pub fn css(color: Hsla) -> String {
    let c = gpui::hsla_to_rgba(color);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "rgba({}, {}, {}, {:.3})",
        byte(c.red),
        byte(c.green),
        byte(c.blue),
        c.alpha.clamp(0.0, 1.0)
    )
}

pub fn backdrop(theme: &ThemeTokens) -> WindowBackgroundAppearance {
    match theme.glass.backdrop {
        Backdrop::None => WindowBackgroundAppearance::Opaque,
        Backdrop::Mica => WindowBackgroundAppearance::MicaBackdrop,
        Backdrop::MicaAlt => WindowBackgroundAppearance::MicaAltBackdrop,
        Backdrop::Acrylic => WindowBackgroundAppearance::Blurred,
    }
}

/// Colors the chrome actually paints, derived once per theme change.
#[derive(Debug, Clone, Copy)]
pub struct ChromeColors {
    pub glass: Hsla,
    pub capsule: Hsla,
    pub hover: Hsla,
    pub field: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub capsule_edge: Hsla,
    pub border: Hsla,
    pub focus: Hsla,
    pub close_hover: Hsla,
    pub selection: Hsla,
}

impl ChromeColors {
    pub fn from_theme(theme: &ThemeTokens) -> Self {
        let c = &theme.colors;
        let glass = theme.glass.tint.with_alpha(theme.glass.opacity);
        Self {
            glass: hsla(glass),
            capsule: hsla(c.surface_raised.with_alpha(0.9)),
            hover: hsla(c.text.with_alpha(0.07)),
            field: hsla(c.text.with_alpha(0.06)),
            text: hsla(c.text),
            muted: hsla(c.text_muted),
            accent: hsla(c.accent),
            on_accent: hsla(c.on_accent),
            capsule_edge: hsla(c.accent.with_alpha(0.35)),
            border: hsla(c.border),
            focus: hsla(c.focus),
            // The Windows caption close-button red, kept for muscle memory.
            close_hover: hsla(Rgba::hex(0xc42b1c)),
            selection: hsla(c.accent.with_alpha(0.22)),
        }
    }
}

impl ChromeColors {
    /// Windows high contrast: system colors only, no glass, no theme hues.
    /// `sys` maps a `GetSysColor` index to a color.
    pub fn high_contrast(sys: impl Fn(i32) -> Rgba) -> Self {
        const WINDOW: i32 = 5;
        const WINDOW_TEXT: i32 = 8;
        const HIGHLIGHT: i32 = 13;
        const HIGHLIGHT_TEXT: i32 = 14;
        let (window, text) = (sys(WINDOW), sys(WINDOW_TEXT));
        let (highlight, on_highlight) = (sys(HIGHLIGHT), sys(HIGHLIGHT_TEXT));
        Self {
            glass: hsla(window),
            capsule: hsla(window),
            hover: hsla(highlight.with_alpha(0.35)),
            field: hsla(window),
            text: hsla(text),
            muted: hsla(text),
            accent: hsla(highlight),
            on_accent: hsla(on_highlight),
            capsule_edge: hsla(highlight),
            border: hsla(text),
            focus: hsla(highlight),
            close_hover: hsla(highlight),
            selection: hsla(highlight.with_alpha(0.5)),
        }
    }
}
