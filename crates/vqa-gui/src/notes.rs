//! Section 6: the notes.
//!
//! One flat, numbered list. It holds every correction the tool made and every caution
//! it wants the user to read, in one tone. It only appears once a result exists.

use crate::theme::Tokens;
use crate::widgets::{card, mono, sans};
use egui::Ui;

/// Draws the notes card, with a header that opens and closes the list.
pub fn show(ui: &mut Ui, tokens: &Tokens, notes: &[String]) {
    let open_id = ui.id().with("notes-open");
    let mut open = ui.data(|data| data.get_temp(open_id)).unwrap_or(true);

    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());

        let header = ui
            .horizontal(|ui| {
                ui.set_width(ui.available_width());
                ui.label(sans("Notes", 15.0, tokens.text).strong());
                ui.label(sans(format!("({})", notes.len()), 12.5, tokens.text_muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(mono(if open { "▾" } else { "▸" }, 11.0, tokens.text_muted));
                });
            })
            .response;

        let header = ui
            .interact(header.rect, open_id.with("button"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if header.clicked() {
            open = !open;
            ui.data_mut(|data| data.insert_temp(open_id, open));
        }

        if open && !notes.is_empty() {
            ui.add_space(8.0);
            for (index, note) in notes.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(sans(format!("{}.", index + 1), 12.5, tokens.text_muted));
                    ui.label(sans(note, 12.5, tokens.text));
                });
                ui.add_space(6.0);
            }
        }
    });
}
