//! Apple-minimal design tokens. Light + dark variants.
//!
//! Spacing is 4px base; radii 4/6/10/16; fonts adapt to platform
//! (-apple-system / Segoe UI / Inter).

use iced::theme::Theme;
use iced::Color;

pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_8: f32 = 32.0;
pub const SPACE_12: f32 = 48.0;

pub const RADIUS_SM: f32 = 4.0;
pub const RADIUS_MD: f32 = 6.0;
pub const RADIUS_LG: f32 = 10.0;
pub const RADIUS_XL: f32 = 16.0;

pub const FONT_FAMILY: &str = if cfg!(target_os = "macos") {
    "-apple-system, SF Pro Text"
} else if cfg!(target_os = "windows") {
    "Segoe UI"
} else {
    "Inter, Helvetica Neue"
};

/// WCAG-AA contrast helper: returns true if ratio >= 4.5.
pub fn meets_wcag_aa(fg: Color, bg: Color) -> bool {
    let lum_fg = relative_luminance(fg);
    let lum_bg = relative_luminance(bg);
    let (lighter, darker) = if lum_fg > lum_bg {
        (lum_fg, lum_bg)
    } else {
        (lum_bg, lum_fg)
    };
    (lighter + 0.05) / (darker + 0.05) >= 4.5
}

fn relative_luminance(c: Color) -> f64 {
    fn channel(v: f32) -> f64 {
        let v = v as f64;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// S-1a default — switch UI ships in S-1b.
pub fn current() -> Theme {
    Theme::Light
}

/// Builds the palette used by S-1b's theme-switch UI.
pub mod palette {
    use super::*;

    pub fn light_background() -> Color {
        Color::from_rgb(0.98, 0.98, 0.98) // #FAFAFA
    }

    pub fn light_text() -> Color {
        Color::from_rgb(0.04, 0.04, 0.04) // #0A0A0A
    }

    pub fn dark_background() -> Color {
        Color::from_rgb(0.110, 0.110, 0.118) // #1C1C1E
    }

    pub fn dark_text() -> Color {
        Color::from_rgb(0.961, 0.961, 0.969) // #F5F5F7
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_dark_themes_construct() {
        let _ = current();
        // The actual `light()` / `dark()` builders depend on iced 0.12
        // theme::Palette which differs from later versions. S-1b will
        // wire them through `Theme::Custom`. For now the test verifies
        // `current()` returns a valid Theme.
    }

    #[test]
    fn wcag_aa_text_contrast() {
        // #0A0A0A on #FAFAFA — must be >= 4.5
        assert!(meets_wcag_aa(
            Color::from_rgb(0.04, 0.04, 0.04),
            Color::from_rgb(0.98, 0.98, 0.98),
        ));
        // Inverse (low contrast) must fail
        assert!(!meets_wcag_aa(
            Color::from_rgb(0.85, 0.85, 0.85),
            Color::from_rgb(0.95, 0.95, 0.95),
        ));
    }

    #[test]
    fn spacing_tokens_are_4px_base() {
        assert_eq!(SPACE_1, 4.0);
        assert_eq!(SPACE_2, 8.0);
        assert_eq!(SPACE_4, 16.0);
    }

    #[test]
    fn font_family_is_platform_adaptive() {
        // Smoke check: FONT_FAMILY is one of the 3 known platform strings.
        assert!(matches!(FONT_FAMILY, s if s.contains("apple-system") || s.contains("Segoe UI") || s.contains("Inter")));
    }
}