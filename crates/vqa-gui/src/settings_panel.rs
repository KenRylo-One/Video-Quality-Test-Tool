//! The Settings panel.
//!
//! This panel is also the first-run screen. A new user has none of the binaries, and the
//! first thing that the tool shows must teach.

use crate::theme::Tokens;
use crate::widgets::{card, mono, sans};
use egui::{Margin, Stroke, Ui};
use std::collections::BTreeMap;
use std::path::PathBuf;
use vqa_core::capability::BinaryId;
use vqa_run::settings::{BUTTERAUGLI_PRESET_NITS, VIEWING_DISTANCES};
use vqa_run::{Session, ThemeChoice};

/// The width of the panel.
pub const WIDTH: f32 = 380.0;

/// The interface state of the panel.
#[derive(Debug, Default)]
pub struct SettingsUi {
    /// Whether the panel is open.
    pub open: bool,
    /// The text of each path field.
    pub paths: BTreeMap<BinaryId, String>,
}

impl SettingsUi {
    /// Fills the path fields from the settings.
    pub fn sync(&mut self, session: &Session) {
        for id in BinaryId::ALL {
            let text = session
                .settings
                .binary_path(id)
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            self.paths.entry(id).or_insert(text);
        }
    }
}

/// Draws the panel. Returns true when a setting changed.
pub fn show(ui: &mut Ui, tokens: &Tokens, session: &mut Session, state: &mut SettingsUi) -> bool {
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label(sans("Settings", 15.0, tokens.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Label::new(mono("✕", 12.0, tokens.text_muted))
                        .sense(egui::Sense::click()),
                )
                .clicked()
            {
                state.open = false;
            }
        });
    });
    ui.add_space(10.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        if session.inventory.is_empty() {
            first_run_banner(ui, tokens);
            ui.add_space(10.0);
        }

        ui.label(mono("BINARIES", 10.5, tokens.text_muted));
        ui.add_space(4.0);
        for id in BinaryId::ALL {
            changed |= binary_row(ui, tokens, session, state, id);
            ui.add_space(6.0);
        }

        ui.add_space(10.0);
        ui.label(mono("RUN PARAMETERS", 10.5, tokens.text_muted));
        crate::widgets::note_line(
            ui,
            tokens,
            "Every setting here changes a number, so every one of them goes in the run record.",
        );
        ui.add_space(4.0);
        changed |= run_parameters(ui, tokens, session);

        ui.add_space(10.0);
        ui.label(mono("GENERAL", 10.5, tokens.text_muted));
        ui.add_space(4.0);
        changed |= general(ui, tokens, session);
    });

    changed
}

/// The banner that a new user sees.
fn first_run_banner(ui: &mut Ui, tokens: &Tokens) {
    egui::Frame::default()
        .fill(tokens.panel)
        .stroke(Stroke::new(1.0, tokens.warn))
        .corner_radius(crate::theme::RADIUS)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(sans("No back end found", 12.5, tokens.warn));
            ui.add_space(4.0);
            ui.label(sans(
                "The tool ships no binary. Get the programs below, then press find. The tool never downloads anything.",
                11.5,
                tokens.note_text,
            ));
        });
}

/// One row of the binary list.
fn binary_row(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &mut Session,
    state: &mut SettingsUi,
    id: BinaryId,
) -> bool {
    let mut changed = false;

    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(mono(id.display_name(), 12.0, tokens.text));
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match session.inventory.get(id) {
                    Some(found) => {
                        let version = found.capabilities.version_label().to_string();
                        ui.label(mono(format!("found · {version}"), 10.5, tokens.good));
                    }
                    None => {
                        ui.label(mono("not found", 10.5, tokens.warn));
                    }
                },
            );
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let text = state.paths.entry(id).or_default();
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(ui.available_width() - 60.0)
                    .hint_text("leave empty to search the PATH")
                    .font(egui::FontId::monospace(11.0)),
            );
            if ui.button(sans("find", 11.5, tokens.text)).on_hover_text(
                "Searches the path above, then a bin folder beside the tool, then the PATH of the operating system.",
            ).clicked() {
                let value = state.paths.get(&id).cloned().unwrap_or_default();
                let path = if value.trim().is_empty() { None } else { Some(PathBuf::from(value.trim())) };
                session.set_binary_path(id, path);
                changed = true;
            }
        });

        ui.add_space(2.0);
        ui.label(sans(
            format!("Gives {}.", id.provides()),
            11.0,
            tokens.text_muted,
        ));
        if session.inventory.get(id).is_none() {
            ui.label(sans(
                format!("Get it from {}.", id.source()),
                11.0,
                tokens.text_muted,
            ));
        }
    });

    changed
}

/// The settings that change a number.
fn run_parameters(ui: &mut Ui, tokens: &Tokens, session: &mut Session) -> bool {
    let mut changed = false;

    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());

        ui.label(sans("VMAF viewing distance", 12.0, tokens.text));
        ui.horizontal(|ui| {
            for distance in VIEWING_DISTANCES {
                let picked = ui.selectable_label(
                    (session.settings.vmaf_viewing_distance - distance).abs() < f32::EPSILON,
                    mono(format!("{distance:.1}H"), 11.0, tokens.text),
                );
                if picked
                    .on_hover_text("The distance in picture heights. It chooses the VMAF v1 model.")
                    .clicked()
                {
                    session.settings.vmaf_viewing_distance = distance;
                    changed = true;
                }
            }
        });

        ui.add_space(8.0);
        ui.label(sans("Butteraugli intensity target", 12.0, tokens.text));
        ui.horizontal_wrapped(|ui| {
            for nits in BUTTERAUGLI_PRESET_NITS {
                let picked = ui.selectable_label(
                    session.settings.butteraugli_intensity_nits == nits,
                    mono(format!("{nits}"), 11.0, tokens.text),
                );
                if picked.clicked() {
                    session.settings.butteraugli_intensity_nits = nits;
                    changed = true;
                }
            }
            ui.label(sans("nits", 11.0, tokens.text_muted));
        });
        crate::widgets::note_line(ui, tokens, "203 nits is the reference white of BT.2408.");

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(sans("CPU lane permits", 12.0, tokens.text));
            if ui
                .add(egui::DragValue::new(&mut session.settings.cpu_lane_permits).range(1..=64))
                .on_hover_text("Two full FFmpeg passes fight each other. Half of the logical cores is the default.")
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(sans("GPU lane permits", 12.0, tokens.text));
            if ui
                .add(egui::DragValue::new(&mut session.settings.gpu_lane_permits).range(1..=8))
                .on_hover_text("A second job on one device does not go faster.")
                .changed()
            {
                changed = true;
            }
        });

        ui.add_space(4.0);
        if ui
            .checkbox(
                &mut session.settings.fused_passes,
                sans("Combine FFmpeg metrics into one pass", 12.0, tokens.text),
            )
            .on_hover_text(
                "Turn this off to isolate one back end that gives a value you do not trust.",
            )
            .changed()
        {
            changed = true;
        }
    });

    changed
}

/// The settings that change no number.
fn general(ui: &mut Ui, tokens: &Tokens, session: &mut Session) -> bool {
    let mut changed = false;

    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());

        for (label, folder, hint) in [
            (
                "Temporary folder",
                0_u8,
                "FFVship writes the scaled intermediate file here.",
            ),
            ("Export folder", 1, "CSV, JSON and PNG go here."),
            (
                "VMAF model folder",
                2,
                "Where vmaf_v1.0.16 and vmaf_v1.0.16_hfr live. Leave empty to search the usual places.",
            ),
        ] {
            ui.label(sans(label, 12.0, tokens.text));
            let mut text = match folder {
                0 => session.settings.temp_folder.clone(),
                1 => session.settings.export_folder.clone(),
                _ => session.settings.vmaf_model_folder.clone(),
            }
            .map(|path| path.display().to_string())
            .unwrap_or_default();

            let hint_text = if folder == 2 {
                match vqa_run::vmaf_models::find_model_folder(None) {
                    Some(found) => found.display().to_string(),
                    None => "not found in the usual places".to_string(),
                }
            } else {
                "the folder of the operating system".to_string()
            };

            let field = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(ui.available_width())
                    .hint_text(hint_text)
                    .font(egui::FontId::monospace(11.0)),
            );
            if field.on_hover_text(hint).changed() {
                let value = if text.trim().is_empty() {
                    None
                } else {
                    Some(PathBuf::from(text.trim()))
                };
                match folder {
                    0 => session.settings.temp_folder = value,
                    1 => session.settings.export_folder = value,
                    _ => session.settings.vmaf_model_folder = value,
                }
                changed = true;
            }
            ui.add_space(6.0);
        }

        ui.label(sans("Theme", 12.0, tokens.text));
        ui.horizontal(|ui| {
            for (choice, label, enabled) in [
                (ThemeChoice::Dark, "Dark", true),
                (ThemeChoice::Light, "Light", true),
                (ThemeChoice::System, "Match system", false),
            ] {
                let picked = ui.add_enabled(
                    enabled,
                    egui::Button::selectable(
                        session.settings.theme == choice,
                        sans(label, 11.5, tokens.text),
                    ),
                );
                if !enabled {
                    picked
                        .clone()
                        .on_hover_text("Match system arrives after version 1.0.");
                }
                if enabled && picked.clicked() {
                    session.settings.theme = choice;
                    changed = true;
                }
            }
        });
    });

    changed
}
