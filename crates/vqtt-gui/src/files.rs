//! Section 1: the files.
//!
//! Drag files onto the page. The first file becomes the reference. Click any other file
//! name to promote it to reference.

use crate::theme::Tokens;
use crate::widgets::{
    card, close_icon, diff_mark, drag_handle_icon, mono, sans, section_header_with,
};
use egui::{Margin, Stroke, Ui};
use std::path::PathBuf;
use vqtt_core::set::FileId;
use vqtt_run::Session;

/// The width of the drag handle column, which every row keeps whether it draws a
/// handle or not.
const HANDLE_COLUMN: f32 = 14.0;

/// What the user did in this section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesAction {
    /// Nothing.
    None,
    /// Make this file the reference.
    Promote(FileId),
    /// Take this file out of the comparison.
    Remove(FileId),
    /// Move the first file to the place of the second.
    Move(FileId, FileId),
    /// Add every one of these files to the comparison.
    Import(Vec<PathBuf>),
}

/// Draws section 1 and reports what the user did.
///
/// Drag and drop needs the operating system to tell the window which file was
/// dropped. Wayland gives no such message to this kind of window, on any Linux
/// desktop, so the Import videos button is not a fallback. On Wayland, it is the
/// only way in.
pub fn show(ui: &mut Ui, tokens: &Tokens, session: &Session) -> FilesAction {
    let mut action = FilesAction::None;

    section_header_with(ui, tokens, "Files", |ui| {
        if ui
            .button(sans("Import videos", 11.5, tokens.text))
            .clicked()
        {
            let paths = pick_video_files();
            if !paths.is_empty() {
                action = FilesAction::Import(paths);
            }
        }
    });

    if session.files.is_empty() {
        drop_zone(ui, tokens, true);
        return action;
    }

    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());

        if let Some(reference) = session.files.reference() {
            row(ui, tokens, session, reference.id, true, &mut action);
        }

        let encodes: Vec<FileId> = session.files.encodes().map(|file| file.id).collect();
        for id in encodes {
            ui.add_space(4.0);
            row(ui, tokens, session, id, false, &mut action);
        }
    });

    if session.files.exceeds_palette() {
        ui.add_space(4.0);
        ui.label(
            sans(
                "More than eight encodes. The palette holds eight slots, so the graphs draw small multiples.",
                11.0,
                tokens.text_muted,
            )
            .italics(),
        );
    }

    ui.add_space(8.0);
    drop_zone(ui, tokens, false);
    action
}

/// One file row.
fn row(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    id: FileId,
    is_reference: bool,
    action: &mut FilesAction,
) {
    let Some(file) = session.files.get(id) else {
        return;
    };
    let marks = session.files.diff_marks(id);
    let row_id = egui::Id::new(("file-row", id.0));

    let row = ui.vertical(|ui| {
        ui.horizontal(|ui| {
            handle(ui, tokens, row_id, id, is_reference);

            if is_reference {
                ui.label(mono("REFERENCE", 10.0, tokens.accent));
                ui.label(sans(&file.label, 12.5, tokens.text));
            } else {
                let name = ui.add(
                    egui::Label::new(sans(&file.label, 12.5, tokens.text).underline())
                        .sense(egui::Sense::click()),
                );
                if name
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Click to make this file the reference.")
                    .clicked()
                {
                    *action = FilesAction::Promote(id);
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !is_reference {
                    let remove = close_icon(ui, 12.0, tokens.text_muted);
                    if remove.on_hover_text("Remove this file from the comparison.").clicked() {
                        *action = FilesAction::Remove(id);
                    }
                }
            });
        });

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // The facts line up under the file name, past the handle column.
            ui.add_space(HANDLE_COLUMN);
            ui.label(mono(file.info.resolution_label(), 11.0, tokens.text_secondary));
            if marks.resolution {
                diff_mark(ui, tokens, "The frame size differs from the reference. The tool scales the encode up to match.");
            }
            ui.label(mono(&file.info.codec, 11.0, tokens.text_secondary));
            ui.label(mono(&file.info.pix_fmt, 11.0, tokens.text_secondary));
            ui.label(mono(file.info.color_range.tag(), 11.0, tokens.text_secondary));
            if marks.color_range {
                diff_mark(ui, tokens, "The color range differs from the reference. The tool converts the encode to match.");
            }
            ui.label(mono(file.info.frame_rate.label(), 11.0, tokens.text_secondary));
            match file.info.frame_count() {
                Some(frames) => {
                    ui.label(mono(format!("{frames} fr"), 11.0, tokens.text_secondary));
                    if marks.frame_count {
                        diff_mark(ui, tokens, "The frame count differs from the reference. The tool measures the frames both files share.");
                    }
                }
                None => {
                    ui.label(mono("? fr", 11.0, tokens.text_muted));
                }
            }
            ui.label(mono(file.info.bitrate_label(), 11.0, tokens.text_secondary));
        });
    });

    // The whole row takes the drop, so a file can land anywhere on the row it replaces.
    // Only the handle starts a drag, which is what leaves the file name free to click.
    if let Some(dragged) = row.response.dnd_release_payload::<FileId>()
        && *dragged != id
    {
        *action = FilesAction::Move(*dragged, id);
    }
}

/// The grip that starts a drag.
///
/// The drag lives on the handle alone. A drag source covers everything inside it with
/// one drag sense, so a row-wide source swallows the click that promotes a file to
/// reference. The reference row keeps the column and draws nothing in it, so every file
/// name starts at the same place and only a row that can move looks like it can.
fn handle(ui: &mut Ui, tokens: &Tokens, row_id: egui::Id, id: FileId, is_reference: bool) {
    if is_reference {
        ui.add_space(HANDLE_COLUMN);
        return;
    }

    ui.dnd_drag_source(row_id, id, |ui| {
        drag_handle_icon(ui, HANDLE_COLUMN, tokens.border_strong);
    })
    .response
    .on_hover_text("Drag to reorder.");
}

/// The strip that takes dropped files.
///
/// Drag and drop works when the operating system supports it. On Wayland it does not,
/// so the hint always names the button too.
fn drop_zone(ui: &mut Ui, tokens: &Tokens, is_empty: bool) {
    let hint = if is_empty {
        "Drop the reference and the encodes here, or press Import videos above. The first file becomes the reference."
    } else {
        "Drop files here, or press Import videos above. Click a file name to make it the reference."
    };

    egui::Frame::default()
        .fill(tokens.panel)
        .stroke(Stroke::new(1.0, tokens.border_strong))
        .corner_radius(crate::theme::RADIUS)
        .inner_margin(Margin::symmetric(10, 14))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(sans(hint, 11.5, tokens.text_muted));
            });
        });
}

/// Opens the native file picker, and reports every video file that the user chose.
///
/// Returns an empty list when the user cancels the dialog.
fn pick_video_files() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import videos")
        .add_filter(
            "Video",
            &[
                "mp4", "mkv", "mov", "webm", "avi", "m4v", "ts", "wmv", "flv",
            ],
        )
        .add_filter("All files", &["*"])
        .pick_files()
        .unwrap_or_default()
}
