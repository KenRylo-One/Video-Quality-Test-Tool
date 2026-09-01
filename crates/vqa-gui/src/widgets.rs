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

/// The heading of one numbered section.
pub fn section_header(ui: &mut Ui, tokens: &Tokens, step: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(mono(step, 10.5, tokens.accent));
        ui.label(sans(title, 17.0, tokens.text));
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
