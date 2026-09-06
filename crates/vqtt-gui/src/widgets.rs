//! Small pieces that every section uses.

use crate::fonts::{MONO_FAMILY, SANS_FAMILY};
use crate::theme::{RADIUS, Tokens};
use egui::{Color32, FontFamily, FontId, Margin, RichText, Stroke, Ui};

/// Text in the data font. Numbers, file properties and technical labels use it.
pub fn mono(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text)
        .font(FontId::new(size, FontFamily::Name(MONO_FAMILY.into())))
        .color(color)
}

/// Text in the interface font. Labels, buttons and body text use it.
pub fn sans(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text)
        .font(FontId::new(size, FontFamily::Name(SANS_FAMILY.into())))
        .color(color)
}

/// The heading of one section.
pub fn section_header(ui: &mut Ui, tokens: &Tokens, title: &str) {
    section_header_with(ui, tokens, title, |_| {});
}

/// The heading of one section, with more content at the right of the row.
///
/// A heading carries no step number. The page is not a sequence of steps.
pub fn section_header_with(
    ui: &mut Ui,
    tokens: &Tokens,
    title: &str,
    right_side: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.label(sans(title, 17.0, tokens.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right_side);
    });
    ui.add_space(6.0);
}

/// A card: a sunken surface with a thin border.
pub fn card<R>(ui: &mut Ui, tokens: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::default()
        .fill(tokens.sunken)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(10))
        .show(ui, add)
        .inner
}

/// A box that holds the space of a section that has no content yet.
///
/// The page never shows an empty gap.
pub fn empty_state(ui: &mut Ui, tokens: &Tokens, text: &str, height: f32) {
    egui::Frame::default()
        .fill(tokens.panel)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_height(height);
            ui.set_min_width(ui.available_width());
            ui.centered_and_justified(|ui| {
                ui.label(sans(text, 12.5, tokens.text_muted));
            });
        });
}

/// The mark that says that a value differs from the reference.
///
/// The mark is not a fault. It says that the tool will correct the difference.
pub fn diff_mark(ui: &mut Ui, tokens: &Tokens, message: &str) {
    ui.label(mono("●", 9.0, tokens.warn)).on_hover_text(message);
}

/// A line of small secondary text.
pub fn note_line(ui: &mut Ui, tokens: &Tokens, text: &str) {
    ui.label(sans(text, 11.0, tokens.text_muted).italics());
}

/// A small cog, drawn with the painter rather than a font glyph.
///
/// No text font ships every symbol, and a missing glyph draws as a tofu box with no
/// warning. The tool draws the icons it depends on for the same reason it draws its own
/// plot and its own range slider: a shape it paints itself cannot go missing.
pub fn gear_icon(ui: &mut Ui, size: f32, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let painter = ui.painter();
    let center = rect.center();
    let outer = size * 0.36;
    let inner = size * 0.16;
    let stroke = Stroke::new((size * 0.09).max(1.0), color);

    for tooth in 0..8 {
        let angle = std::f32::consts::TAU * tooth as f32 / 8.0;
        let direction = egui::vec2(angle.cos(), angle.sin());
        painter.line_segment(
            [
                center + direction * outer * 0.7,
                center + direction * outer * 1.15,
            ],
            stroke,
        );
    }
    painter.circle_stroke(center, outer, stroke);
    painter.circle_filled(center, inner, color);

    response
}

/// A small colored square standing in for a series line.
///
/// This used to be the glyph "■" (or "□" hollow), but that glyph is one more symbol
/// IBM Plex Mono does not carry, so it drew as the same tofu box as the gear. Painted
/// like this it cannot go missing.
pub fn color_swatch(ui: &mut Ui, size: f32, color: Color32, filled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    if filled {
        ui.painter().rect_filled(rect, 1.0, color);
    } else {
        ui.painter()
            .rect_stroke(rect, 1.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    }
    response
}
