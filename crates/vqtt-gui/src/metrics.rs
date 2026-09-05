//! Section 2: what to measure, and how much of the file to measure.

use crate::theme::{RADIUS, Tokens};
use crate::widgets::{card, mono, note_line, sans, section_header};
use egui::{Sense, Ui};
use vqtt_core::media::Rational;
use vqtt_core::metric::{MetricGroup, MetricId, metrics_in_group};
use vqtt_core::preset::PRESETS;
use vqtt_run::Session;

/// Which unit the frame range control shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RangeUnit {
    /// A frame number.
    #[default]
    Frames,
    /// Seconds from the start.
    Seconds,
    /// Hours, minutes, seconds and frames.
    Timecode,
}

impl RangeUnit {
    /// Every unit, in the order of the three buttons.
    pub const ALL: [RangeUnit; 3] = [RangeUnit::Frames, RangeUnit::Seconds, RangeUnit::Timecode];

    /// The label on the button.
    pub fn label(self) -> &'static str {
        match self {
            Self::Frames => "frames",
            Self::Seconds => "seconds",
            Self::Timecode => "timecode",
        }
    }
}

/// What the user did in this section.
#[derive(Debug, Clone, PartialEq)]
pub enum MetricsAction {
    /// Nothing.
    None,
    /// Tick or untick one metric.
    Toggle(MetricId, bool),
    /// Apply one preset.
    Preset(String),
    /// Measure the whole file, or a part of it.
    WholeFile(bool),
    /// Set the frame range.
    Range(u64, u64),
}

/// The state that belongs to the interface alone.
#[derive(Debug, Default)]
pub struct MetricsUi {
    /// Which unit the range control shows.
    pub unit: RangeUnit,
    /// The text of the first frame field.
    pub start_text: String,
    /// The text of the last frame field.
    pub end_text: String,
}

/// Draws section 2 and reports what the user did.
pub fn show(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    state: &mut MetricsUi,
) -> MetricsAction {
    let mut action = MetricsAction::None;

    section_header(ui, tokens, "Metric setup");

    let preset_label = session.selection.preset.unwrap_or("Preset…").to_string();
    egui::ComboBox::from_id_salt("preset")
        .selected_text(sans(preset_label, 12.0, tokens.text))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for preset in PRESETS {
                let picked = ui
                    .selectable_label(
                        session.selection.preset == Some(preset.name),
                        sans(preset.name, 12.0, tokens.text),
                    )
                    .on_hover_text(preset.reason);
                if picked.clicked() {
                    action = MetricsAction::Preset(preset.name.to_string());
                }
            }
        });

    ui.add_space(8.0);

    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        for group in MetricGroup::ALL {
            ui.label(mono(group.label(), 10.5, tokens.text_muted));
            ui.add_space(2.0);

            let mut reasons: Vec<String> = Vec::new();
            ui.horizontal_wrapped(|ui| {
                for def in metrics_in_group(group) {
                    let state = session.availability(def.id);
                    let mut ticked = session.selection.metrics.contains(&def.id);

                    if state.is_available() {
                        let box_response =
                            ui.checkbox(&mut ticked, sans(def.label, 12.0, tokens.text));
                        let hover = format!(
                            "{}. Range {}. {}",
                            state.provider.map(|p| p.implementation).unwrap_or_default(),
                            def.range,
                            def.direction.label()
                        );
                        if box_response.on_hover_text(hover).changed() {
                            action = MetricsAction::Toggle(def.id, ticked);
                        }
                    } else {
                        let mut off = false;
                        ui.add_enabled(
                            false,
                            egui::Checkbox::new(&mut off, sans(def.label, 12.0, tokens.text_muted)),
                        );
                        if let Some(reason) = state.reason {
                            reasons.push(format!("{} {}", def.label, reason));
                        }
                    }
                }
            });

            for reason in reasons {
                note_line(ui, tokens, &reason);
            }
            ui.add_space(8.0);
        }
    });

    ui.add_space(8.0);

    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        let total = session.total_frames();
        let mut whole = session.selection.whole_file;
        if ui
            .checkbox(&mut whole, sans("whole file", 12.0, tokens.text))
            .changed()
        {
            action = MetricsAction::WholeFile(whole);
        }

        if !session.selection.whole_file && total > 0 {
            ui.add_space(6.0);
            let mut first = session.selection.first_frame;
            let mut last = session.selection.last_frame;
            if range_slider(ui, tokens, &mut first, &mut last, total) {
                action = MetricsAction::Range(first, last);
            }

            ui.add_space(4.0);
            let fps = session
                .files
                .reference()
                .map(|file| file.info.frame_rate)
                .unwrap_or(Rational::ZERO);

            ui.horizontal(|ui| {
                for unit in RangeUnit::ALL {
                    let picked = ui.selectable_label(
                        state.unit == unit,
                        mono(unit.label(), 10.5, tokens.text),
                    );
                    if picked.clicked() {
                        state.unit = unit;
                        state.start_text.clear();
                        state.end_text.clear();
                    }
                }
            });

            ui.add_space(4.0);
            if state.start_text.is_empty() {
                state.start_text = format_unit(first, fps, state.unit);
            }
            if state.end_text.is_empty() {
                state.end_text = format_unit(last, fps, state.unit);
            }

            ui.horizontal(|ui| {
                let start_field = ui.add(
                    egui::TextEdit::singleline(&mut state.start_text)
                        .desired_width(90.0)
                        .font(egui::FontId::monospace(11.0)),
                );
                ui.label(sans("to", 11.5, tokens.text_muted));
                let end_field = ui.add(
                    egui::TextEdit::singleline(&mut state.end_text)
                        .desired_width(90.0)
                        .font(egui::FontId::monospace(11.0)),
                );

                if start_field.lost_focus() || end_field.lost_focus() {
                    let parsed_start = parse_unit(&state.start_text, fps, state.unit)
                        .unwrap_or(first)
                        .min(total - 1);
                    let parsed_end = parse_unit(&state.end_text, fps, state.unit)
                        .unwrap_or(last)
                        .min(total - 1);
                    let (low, high) = if parsed_start <= parsed_end {
                        (parsed_start, parsed_end)
                    } else {
                        (parsed_end, parsed_start)
                    };
                    state.start_text = format_unit(low, fps, state.unit);
                    state.end_text = format_unit(high, fps, state.unit);
                    action = MetricsAction::Range(low, high);
                }
            });
        }

        ui.add_space(6.0);
        match session.estimate() {
            Some(estimate) => {
                ui.label(sans(format!("estimated run time {}", estimate.label()), 11.5, tokens.text_secondary))
                    .on_hover_text(
                        "The estimate is a range, never one number. The cost values are provisional until a run measures this machine.",
                    );
            }
            None => {
                ui.label(sans(
                    "estimated run time: nothing to run yet",
                    11.5,
                    tokens.text_muted,
                ));
            }
        }
    });

    action
}

/// A slider with two handles.
///
/// `egui` has no range slider, so the tool draws one. It returns true when a handle moved.
fn range_slider(ui: &mut Ui, tokens: &Tokens, first: &mut u64, last: &mut u64, total: u64) -> bool {
    let height = 22.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        Sense::click_and_drag(),
    );
    let painter = ui.painter();

    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width() - 12.0, 4.0));
    painter.rect_filled(track, RADIUS, tokens.border);

    let span = (total.saturating_sub(1)).max(1) as f32;
    let position = |frame: u64| track.left() + track.width() * (frame as f32 / span);
    let mut low = position(*first);
    let mut high = position(*last);

    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(low, track.top()),
            egui::pos2(high, track.bottom()),
        ),
        RADIUS,
        tokens.accent,
    );
    painter.circle_filled(egui::pos2(low, rect.center().y), 6.0, tokens.accent);
    painter.circle_filled(egui::pos2(high, rect.center().y), 6.0, tokens.accent);

    let handle_id = response.id.with("handle");
    let mut changed = false;

    if response.drag_started() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let nearest_low = (pointer.x - low).abs() <= (pointer.x - high).abs();
            ui.memory_mut(|memory| memory.data.insert_temp(handle_id, nearest_low));
        }
    }

    if response.dragged() || response.clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let fraction = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
            let frame = (fraction * span).round() as u64;
            let move_low = ui
                .memory(|memory| memory.data.get_temp::<bool>(handle_id))
                .unwrap_or((pointer.x - low).abs() <= (pointer.x - high).abs());
            if move_low {
                *first = frame.min(*last);
                low = position(*first);
            } else {
                *last = frame.max(*first);
                high = position(*last);
            }
            changed = true;
        }
    }

    let _ = (low, high);
    changed
}

/// Writes one frame number in the unit that the user picked.
pub fn format_unit(frame: u64, fps: Rational, unit: RangeUnit) -> String {
    let rate = fps.as_f64();
    match unit {
        RangeUnit::Frames => frame.to_string(),
        RangeUnit::Seconds => {
            if rate <= 0.0 {
                return frame.to_string();
            }
            format!("{:.2}", frame as f64 / rate)
        }
        RangeUnit::Timecode => {
            if rate <= 0.0 {
                return frame.to_string();
            }
            let whole = rate.round().max(1.0) as u64;
            let hours = frame / (whole * 3600);
            let minutes = (frame / (whole * 60)) % 60;
            let seconds = (frame / whole) % 60;
            let frames = frame % whole;
            format!("{hours:02}:{minutes:02}:{seconds:02}:{frames:02}")
        }
    }
}

/// Reads a frame number back out of the unit that the user picked.
pub fn parse_unit(text: &str, fps: Rational, unit: RangeUnit) -> Option<u64> {
    let text = text.trim();
    let rate = fps.as_f64();
    match unit {
        RangeUnit::Frames => text.parse().ok(),
        RangeUnit::Seconds => {
            let seconds: f64 = text.parse().ok()?;
            if rate <= 0.0 || seconds < 0.0 {
                return None;
            }
            Some((seconds * rate).round() as u64)
        }
        RangeUnit::Timecode => {
            if rate <= 0.0 {
                return None;
            }
            let whole = rate.round().max(1.0) as u64;
            let parts: Vec<u64> = text
                .split(':')
                .map(|part| part.trim().parse().ok())
                .collect::<Option<Vec<u64>>>()?;
            let [hours, minutes, seconds, frames] = parts.as_slice() else {
                return None;
            };
            Some(((hours * 60 + minutes) * 60 + seconds) * whole + frames)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fps(num: u64, den: u64) -> Rational {
        Rational { num, den }
    }

    #[test]
    fn a_frame_number_reads_back_in_every_unit() {
        let rate = fps(30, 1);
        for unit in RangeUnit::ALL {
            let text = format_unit(4321, rate, unit);
            assert_eq!(
                parse_unit(&text, rate, unit),
                Some(4321),
                "unit {}",
                unit.label()
            );
        }
    }

    #[test]
    fn timecode_counts_frames_inside_the_second() {
        assert_eq!(
            format_unit(0, fps(30, 1), RangeUnit::Timecode),
            "00:00:00:00"
        );
        assert_eq!(
            format_unit(30, fps(30, 1), RangeUnit::Timecode),
            "00:00:01:00"
        );
        assert_eq!(
            format_unit(1801, fps(30, 1), RangeUnit::Timecode),
            "00:01:00:01"
        );
    }

    #[test]
    fn a_file_with_no_frame_rate_falls_back_to_the_frame_number() {
        assert_eq!(format_unit(120, Rational::ZERO, RangeUnit::Seconds), "120");
        assert_eq!(format_unit(120, Rational::ZERO, RangeUnit::Timecode), "120");
    }

    #[test]
    fn text_that_is_not_a_number_reads_as_nothing() {
        assert_eq!(parse_unit("later", fps(30, 1), RangeUnit::Frames), None);
        assert_eq!(parse_unit("00:00", fps(30, 1), RangeUnit::Timecode), None);
    }
}
