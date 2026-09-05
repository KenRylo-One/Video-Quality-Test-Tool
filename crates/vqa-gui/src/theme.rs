//! The two themes.
//!
//! Dark is the default. Light is fully supported. Every text token was computed against
//! its surface rather than judged by eye.

use egui::{Color32, CornerRadius, Stroke};
use vqa_core::palette::{Theme, series_color};
use vqa_run::ThemeChoice;

/// The color of every part of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tokens {
    /// Which palette column the graphs use.
    pub theme: Theme,
    /// Outside the window frame.
    pub app_background: Color32,
    /// The window surface.
    pub window: Color32,
    /// A panel on the window.
    pub panel: Color32,
    /// A card or an input.
    pub sunken: Color32,
    /// A normal border.
    pub border: Color32,
    /// A border that must stand out.
    pub border_strong: Color32,
    /// Body text.
    pub text: Color32,
    /// Secondary text.
    pub text_secondary: Color32,
    /// Small labels.
    pub text_muted: Color32,
    /// The accent.
    pub accent: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// A difference mark, and the label of a note.
    pub warn: Color32,
    /// The body of a note.
    pub note_text: Color32,
    /// A good result, and the graphics card lane.
    pub good: Color32,
}

/// The corner radius of everything except the window frame.
pub const RADIUS: CornerRadius = CornerRadius::same(3);

/// Builds a color from a hexadecimal string of the design token table.
const fn rgb(value: u32) -> Color32 {
    Color32::from_rgb(
        (value >> 16) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    )
}

/// The dark theme.
pub const DARK: Tokens = Tokens {
    theme: Theme::Dark,
    app_background: rgb(0x05060a),
    window: rgb(0x12141a),
    panel: rgb(0x161920),
    sunken: rgb(0x191c22),
    border: rgb(0x2b303a),
    border_strong: rgb(0x454a54),
    text: rgb(0xe7e9ee),
    text_secondary: rgb(0x9aa0ac),
    text_muted: rgb(0x7c828e),
    accent: rgb(0x3987e5),
    on_accent: rgb(0x0d1117),
    warn: rgb(0xc98862),
    note_text: rgb(0xc3ab9a),
    good: rgb(0x199e70),
};

/// The light theme.
pub const LIGHT: Tokens = Tokens {
    theme: Theme::Light,
    app_background: rgb(0xe6e5e1),
    window: rgb(0xfcfcfb),
    panel: rgb(0xf4f3f0),
    sunken: rgb(0xffffff),
    border: rgb(0xd8d6d0),
    border_strong: rgb(0xb4b1a9),
    text: rgb(0x0b0b0b),
    text_secondary: rgb(0x52514e),
    text_muted: rgb(0x6b6963),
    accent: rgb(0x1f63b8),
    on_accent: rgb(0xfcfcfb),
    warn: rgb(0xa15b2e),
    note_text: rgb(0x7a5138),
    good: rgb(0x158a5f),
};

/// The flat gray behind the three frames of the frame viewer.
///
/// This value is the same in both themes, and it is deliberate. A tinted surround changes
/// how a person judges an image.
pub const FRAME_VIEWER_GRAY: Color32 = rgb(0x808080);

impl Tokens {
    /// The token set of one theme choice.
    ///
    /// `Match system` arrives after version 1.0, so it reads as dark for now.
    pub fn for_choice(choice: ThemeChoice) -> Self {
        match choice {
            ThemeChoice::Light => LIGHT,
            ThemeChoice::Dark | ThemeChoice::System => DARK,
        }
    }

    /// The color of one series slot. A later milestone draws the graphs.
    #[allow(dead_code)]
    pub fn series(&self, slot: usize) -> Color32 {
        match series_color(self.theme, slot) {
            Some(color) => Color32::from_rgb(color.r, color.g, color.b),
            None => self.text_muted,
        }
    }

    /// Puts the tokens into the style of the window.
    ///
    /// The tool paints every color itself, so both style variants take the same tokens.
    /// The theme setting then only chooses which token set this is.
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(if self.theme == Theme::Dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        ctx.all_styles_mut(|style| self.write_style(style));
    }

    /// Writes the tokens into one style.
    fn write_style(&self, style: &mut egui::Style) {
        let visuals = &mut style.visuals;

        visuals.dark_mode = self.theme == Theme::Dark;
        visuals.override_text_color = Some(self.text);
        visuals.window_fill = self.window;
        visuals.panel_fill = self.window;
        visuals.extreme_bg_color = self.sunken;
        visuals.faint_bg_color = self.panel;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.35);
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
        visuals.hyperlink_color = self.accent;
        visuals.window_corner_radius = CornerRadius::same(8);

        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = RADIUS;
            widget.bg_stroke = Stroke::new(1.0, self.border);
            widget.fg_stroke = Stroke::new(1.0, self.text);
        }
        visuals.widgets.noninteractive.bg_fill = self.panel;
        visuals.widgets.noninteractive.weak_bg_fill = self.panel;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text_secondary);
        visuals.widgets.inactive.bg_fill = self.sunken;
        visuals.widgets.inactive.weak_bg_fill = self.panel;
        visuals.widgets.hovered.bg_fill = self.panel;
        visuals.widgets.hovered.weak_bg_fill = self.panel;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.border_strong);
        visuals.widgets.active.bg_fill = self.accent;
        visuals.widgets.active.weak_bg_fill = self.accent;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, self.accent);

        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 22.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relative luminance of one color, as WCAG defines it.
    fn luminance(color: Color32) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    /// The contrast ratio between two colors.
    fn contrast(first: Color32, second: Color32) -> f64 {
        let (a, b) = (luminance(first), luminance(second));
        let (high, low) = if a > b { (a, b) } else { (b, a) };
        (high + 0.05) / (low + 0.05)
    }

    #[test]
    fn every_text_token_reaches_four_point_five_against_its_surface() {
        for tokens in [DARK, LIGHT] {
            for text in [tokens.text, tokens.text_secondary, tokens.text_muted] {
                let ratio = contrast(text, tokens.window);
                assert!(ratio >= 4.5, "contrast {ratio:.2} is below 4.5");
            }
        }
    }

    #[test]
    fn the_accent_fill_carries_its_own_text() {
        for tokens in [DARK, LIGHT] {
            let ratio = contrast(tokens.on_accent, tokens.accent);
            assert!(ratio >= 4.5, "contrast {ratio:.2} is below 4.5");
        }
    }

    #[test]
    fn the_frame_viewer_gray_is_the_same_in_both_themes() {
        assert_eq!(FRAME_VIEWER_GRAY, rgb(0x808080));
    }

    /// The window and the exported file take their plot colours from two places, so a
    /// changed token here must not leave an SVG painting the old one.
    #[test]
    fn the_plot_chrome_matches_the_tokens_of_the_same_theme() {
        for tokens in [DARK, LIGHT] {
            let chrome = vqa_core::plot::Chrome::for_theme(tokens.theme);
            let same = |left: Color32, right: vqa_core::plot::Rgba| {
                assert_eq!(
                    (left.r(), left.g(), left.b()),
                    (right.r, right.g, right.b),
                    "{:?} chrome and tokens disagree",
                    tokens.theme
                );
            };
            same(tokens.border, chrome.axis);
            same(tokens.text, chrome.text);
            same(tokens.text_secondary, chrome.text_secondary);
            same(tokens.text_muted, chrome.text_muted);
            same(tokens.warn, chrome.warn);
            same(tokens.sunken, chrome.surface);
        }
    }
}
