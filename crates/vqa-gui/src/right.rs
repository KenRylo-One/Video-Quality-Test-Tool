//! The right column: the graph plot and the results table.

use crate::plot as renderer;
use crate::theme::Tokens;
use crate::widgets::{card, empty_state, mono, sans, section_header};
use egui::Ui;
use std::collections::{BTreeSet, HashMap};
use vqa_core::media::Rational;
use vqa_core::metric::MetricId;
use vqa_core::palette::series_color;
use vqa_core::plot::{PlotRequest, SeriesInput, build_scenes};
use vqa_core::pooling::Pooled;
use vqa_core::set::FileId;
use vqa_run::Session;

/// The height of the plot canvas.
const PLOT_HEIGHT: f32 = 260.0;

/// Direct labels and the dash switch appear above this many encodes.
const CROWDED_ABOVE: usize = 3;

/// How much of the visible span one wheel notch removes.
const ZOOM_STEP: f32 = 0.12;

/// What the reader has set on the plot.
///
/// Zoom and the hover frame belong to the page, not to one tab. Zoom into a moment,
/// switch tabs, and the moment holds. Losing either on a tab switch takes away the
/// reason tabs work at all.
#[derive(Default)]
pub struct PlotUi {
    pub active_tab: Option<MetricId>,
    pub x_domain: Option<(f32, f32)>,
    pub hover_frame: Option<u64>,
    pub high_contrast: bool,
    pub hidden: BTreeSet<FileId>,
}

impl PlotUi {
    fn domain(&self) -> (f32, f32) {
        self.x_domain.unwrap_or((0.0, 1.0))
    }

    /// Moves to another tab. Zoom and the crosshair stay where they are.
    pub fn select_tab(&mut self, metric: MetricId) {
        self.active_tab = Some(metric);
    }

    pub fn reset_zoom(&mut self) {
        self.x_domain = None;
    }

    /// Zooms around the pointer. A positive `notches` moves closer.
    pub fn zoom(&mut self, notches: f32, at: f32) {
        let (start, end) = self.domain();
        let span = end - start;
        let wanted = (span * (1.0 - ZOOM_STEP * notches)).clamp(0.001, 1.0);
        let focus = start + span * at.clamp(0.0, 1.0);
        let mut new_start = focus - wanted * at.clamp(0.0, 1.0);
        let mut new_end = new_start + wanted;
        if new_start < 0.0 {
            new_start = 0.0;
            new_end = wanted;
        }
        if new_end > 1.0 {
            new_end = 1.0;
            new_start = 1.0 - wanted;
        }
        self.x_domain = Some((new_start.max(0.0), new_end.min(1.0)));
    }

    pub fn toggle_line(&mut self, file: FileId) {
        if !self.hidden.remove(&file) {
            self.hidden.insert(file);
        }
    }
}

/// Everything one run gives the right column.
pub struct RunView<'a> {
    pub results: &'a HashMap<(FileId, MetricId), Pooled>,
    pub series: &'a HashMap<(FileId, MetricId), Vec<f32>>,
    pub first_frame: u64,
    pub frame_rate: Rational,
}

pub fn show(ui: &mut Ui, tokens: &Tokens, session: &Session, run: &RunView, state: &mut PlotUi) {
    section_header(ui, tokens, "Plot");

    let finished = finished_metrics(session, run);
    if finished.is_empty() {
        empty_state(ui, tokens, "Run a comparison to see the plot.", PLOT_HEIGHT);
    } else {
        plot_section(ui, tokens, session, run, state, &finished);
    }

    ui.add_space(16.0);
    ui.label(sans("Results", 15.0, tokens.text));
    ui.add_space(6.0);

    if run.results.is_empty() {
        empty_state(ui, tokens, "The numbers appear here.", 120.0);
        return;
    }
    numbers_table(ui, tokens, session, run.results);
}

/// Every metric that has a result, in registry order, so the tabs never reshuffle.
fn finished_metrics(session: &Session, run: &RunView) -> Vec<MetricId> {
    vqa_core::metric::REGISTRY
        .iter()
        .map(|def| def.id)
        .filter(|metric| {
            session
                .files
                .encodes()
                .any(|encode| run.results.contains_key(&(encode.id, *metric)))
        })
        .collect()
}

fn plot_section(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    run: &RunView,
    state: &mut PlotUi,
    finished: &[MetricId],
) {
    let active = match state.active_tab {
        Some(metric) if finished.contains(&metric) => metric,
        _ => finished[0],
    };
    state.active_tab = Some(active);

    ui.horizontal_wrapped(|ui| {
        for metric in finished {
            let chosen = *metric == active;
            let label = sans(
                metric.def().label,
                12.0,
                if chosen {
                    tokens.on_accent
                } else {
                    tokens.text
                },
            );
            let button =
                egui::Button::new(label).fill(if chosen { tokens.accent } else { tokens.sunken });
            if ui.add(button).clicked() {
                state.select_tab(*metric);
            }
        }
    });
    ui.add_space(8.0);

    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        plot_card(ui, tokens, session, run, state, active);
    });
}

fn plot_card(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    run: &RunView,
    state: &mut PlotUi,
    active: MetricId,
) {
    let names: Vec<(FileId, Option<usize>, String)> = session
        .files
        .encodes()
        .filter(|encode| !state.hidden.contains(&encode.id))
        .filter(|encode| run.series.contains_key(&(encode.id, active)))
        .map(|encode| {
            (
                encode.id,
                session.files.slot_of(encode.id),
                encode.label.clone(),
            )
        })
        .collect();
    let crowded = session.files.encodes().count() > CROWDED_ABOVE;

    ui.horizontal(|ui| {
        ui.label(mono(active.def().label, 14.0, tokens.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(sans("zoom reset", 11.0, tokens.text_secondary)).frame(false),
                )
                .clicked()
            {
                state.reset_zoom();
            }
            if crowded {
                ui.checkbox(
                    &mut state.high_contrast,
                    sans("high contrast", 11.0, tokens.text_secondary),
                )
                .on_hover_text(
                    "Adds a dash pattern to each line, for print and for a colour-blind reader.",
                );
            }
        });
    });
    ui.add_space(4.0);

    let inputs: Vec<SeriesInput> = names
        .iter()
        .map(|(file, slot, label)| SeriesInput {
            file: *file,
            slot: *slot,
            name: label,
            values: run
                .series
                .get(&(*file, active))
                .map_or(&[][..], Vec::as_slice),
        })
        .collect();

    let width = ui.available_width();
    let request = PlotRequest {
        metric: active,
        series: &inputs,
        theme: tokens.theme,
        high_contrast: state.high_contrast,
        x_domain: state.domain(),
        first_frame: run.first_frame,
        frame_rate: run.frame_rate,
        size: (width, PLOT_HEIGHT),
        hover_frame: state.hover_frame,
    };
    let scenes = build_scenes(&request);

    let total_height: f32 = scenes.iter().map(|scene| scene.size.1).sum();
    let (response, painter) = ui.allocate_painter(
        egui::vec2(width, total_height),
        egui::Sense::click_and_drag(),
    );

    let mut top = response.rect.min;
    for scene in &scenes {
        if let Some(title) = &scene.title {
            painter.text(
                top + egui::vec2(scene.plot.x, 0.0),
                egui::Align2::LEFT_TOP,
                title,
                egui::FontId::new(
                    11.0,
                    egui::FontFamily::Name(crate::fonts::MONO_FAMILY.into()),
                ),
                tokens.text_secondary,
            );
        }
        renderer::paint(&painter, tokens, scene, top);
        top.y += scene.size.1;
    }

    read_pointer(ui, &response, state, &scenes, run);
    ui.add_space(6.0);
    legend(ui, tokens, session, state, active, run);
    hover_readout(ui, tokens, state, &inputs, active, run);
}

/// Turns the pointer into a hovered frame and a zoom level.
fn read_pointer(
    ui: &Ui,
    response: &egui::Response,
    state: &mut PlotUi,
    scenes: &[vqa_core::plot::Scene],
    run: &RunView,
) {
    let Some(scene) = scenes.first() else {
        return;
    };
    let Some(pointer) = response.hover_pos() else {
        // Leaving the plot must not clear the crosshair, because holding one frame and
        // flipping tabs is how a reader compares two metrics at one moment.
        return;
    };

    let plot_left = response.rect.min.x + scene.plot.x;
    let share = ((pointer.x - plot_left) / scene.plot.w).clamp(0.0, 1.0);

    let notches = ui.input(|input| input.smooth_scroll_delta.y);
    if notches.abs() > 0.1 {
        state.zoom(notches.signum(), share);
    }

    let (start, end) = state.domain();
    let sample_span = sample_count(run, scene.metric).saturating_sub(1) as f32;
    let first = (start * sample_span) as u64;
    let last = (end * sample_span) as u64;
    state.hover_frame = Some(run.first_frame + first + ((last - first) as f32 * share) as u64);
}

fn sample_count(run: &RunView, metric: MetricId) -> usize {
    run.series
        .iter()
        .filter(|((_, id), _)| *id == metric)
        .map(|(_, values)| values.len())
        .max()
        .unwrap_or(0)
}

fn legend(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    state: &mut PlotUi,
    active: MetricId,
    run: &RunView,
) {
    let encodes: Vec<_> = session
        .files
        .encodes()
        .filter(|encode| run.results.contains_key(&(encode.id, active)))
        .map(|encode| {
            (
                encode.id,
                session.files.slot_of(encode.id),
                encode.label.clone(),
            )
        })
        .collect();
    if encodes.len() < 2 {
        return;
    }

    ui.horizontal_wrapped(|ui| {
        for (file, slot, label) in encodes {
            let shown = !state.hidden.contains(&file);
            let color = series_color(tokens.theme, slot.unwrap_or(0))
                .map(|series| egui::Color32::from_rgb(series.r, series.g, series.b))
                .unwrap_or(tokens.text_muted);
            let swatch = if shown { "\u{25a0}" } else { "\u{25a1}" };
            let text = if shown {
                tokens.text
            } else {
                tokens.text_muted
            };
            let entry = ui.horizontal(|ui| {
                ui.label(mono(swatch, 11.0, color));
                ui.label(sans(label, 11.0, text));
            });
            if entry
                .response
                .interact(egui::Sense::click())
                .on_hover_text("Click to hide or show this line.")
                .clicked()
            {
                state.toggle_line(file);
            }
            ui.add_space(6.0);
        }
    });
}

fn hover_readout(
    ui: &mut Ui,
    tokens: &Tokens,
    state: &PlotUi,
    inputs: &[SeriesInput],
    active: MetricId,
    run: &RunView,
) {
    ui.add_space(4.0);
    let Some(frame) = state.hover_frame else {
        ui.label(sans(
            "Hover the plot to read one frame.",
            11.0,
            tokens.text_muted,
        ));
        return;
    };

    let index = frame.saturating_sub(run.first_frame) as usize;
    let suffix = active.def().unit.suffix();
    ui.horizontal_wrapped(|ui| {
        ui.label(mono(format!("frame {frame}"), 11.0, tokens.text_secondary));
        for input in inputs {
            let Some(value) = input.values.get(index) else {
                continue;
            };
            ui.add_space(8.0);
            ui.label(mono(
                format!("{}: {value:.3}{suffix}", input.name),
                11.0,
                tokens.text,
            ));
        }
    });
}

fn numbers_table(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    results: &HashMap<(FileId, MetricId), Pooled>,
) {
    egui::Frame::default()
        .fill(tokens.sunken)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .corner_radius(crate::theme::RADIUS)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::both().max_height(246.0).show(ui, |ui| {
                header_row(ui, tokens);
                for encode in session.files.encodes() {
                    for metric in vqa_core::metric::REGISTRY.iter().map(|def| def.id) {
                        let Some(pooled) = results.get(&(encode.id, metric)) else {
                            continue;
                        };
                        data_row(ui, tokens, &encode.label, metric, pooled);
                    }
                }
            });
        });
}

fn header_row(ui: &mut Ui, tokens: &Tokens) {
    ui.horizontal(|ui| {
        for label in [
            "Video", "metric", "mean", "median", "Worst 5%", "Worst 1%", "min", "max", "std dev",
        ] {
            ui.add_sized(
                [80.0, 16.0],
                egui::Label::new(mono(label, 10.5, tokens.text_muted)),
            );
        }
    });
}

fn data_row(ui: &mut Ui, tokens: &Tokens, encode_label: &str, metric: MetricId, pooled: &Pooled) {
    let def = metric.def();
    let bad_end = def.direction.bad_end();
    let worst_5 = match bad_end {
        vqa_core::metric::Percentile::P5 => pooled.p5,
        vqa_core::metric::Percentile::P95 => pooled.p95,
    };
    let worst_1 = match bad_end {
        vqa_core::metric::Percentile::P5 => pooled.p1,
        vqa_core::metric::Percentile::P95 => pooled.p99,
    };

    ui.horizontal(|ui| {
        let cell = |ui: &mut Ui, text: String| {
            ui.add_sized(
                [80.0, 16.0],
                egui::Label::new(mono(text, 11.0, tokens.text)),
            );
        };
        cell(ui, encode_label.to_string());
        cell(ui, def.label.to_string());
        cell(ui, format!("{:.2}", pooled.mean));
        cell(ui, format!("{:.2}", pooled.median));
        cell(ui, format!("{worst_5:.2}"));
        cell(ui, format!("{worst_1:.2}"));
        cell(ui, format!("{:.2}", pooled.min));
        cell(ui, format!("{:.2}", pooled.max));
        cell(ui, format!("{:.2}", pooled.stdev));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_tabs_keeps_the_zoom_and_the_crosshair() {
        let mut state = PlotUi::default();
        state.zoom(1.0, 0.5);
        state.hover_frame = Some(1234);
        let zoomed = state.domain();

        state.select_tab(MetricId::SsimAll);

        assert_eq!(state.domain(), zoomed);
        assert_eq!(state.hover_frame, Some(1234));
        assert_eq!(state.active_tab, Some(MetricId::SsimAll));
    }

    #[test]
    fn zooming_in_narrows_the_visible_share_and_zoom_reset_restores_it() {
        let mut state = PlotUi::default();
        assert_eq!(state.domain(), (0.0, 1.0));

        state.zoom(1.0, 0.5);
        let (start, end) = state.domain();
        assert!(end - start < 1.0);

        state.reset_zoom();
        assert_eq!(state.domain(), (0.0, 1.0));
    }

    #[test]
    fn zooming_never_leaves_the_series() {
        let mut state = PlotUi::default();
        for _ in 0..40 {
            state.zoom(1.0, 0.0);
        }
        let (start, end) = state.domain();
        assert!(start >= 0.0 && end <= 1.0 && start < end);

        let mut other = PlotUi::default();
        for _ in 0..40 {
            other.zoom(1.0, 1.0);
        }
        let (start, end) = other.domain();
        assert!(start >= 0.0 && end <= 1.0 && start < end);
    }

    #[test]
    fn zooming_out_past_the_whole_series_stops_at_the_whole_series() {
        let mut state = PlotUi::default();
        state.zoom(1.0, 0.5);
        for _ in 0..40 {
            state.zoom(-1.0, 0.5);
        }
        let (start, end) = state.domain();
        assert!(start >= 0.0 && end <= 1.0);
        assert!(end - start > 0.99);
    }

    #[test]
    fn the_legend_toggles_one_line_and_leaves_the_others_alone() {
        let mut state = PlotUi::default();
        state.toggle_line(FileId(2));
        assert!(state.hidden.contains(&FileId(2)));
        assert!(!state.hidden.contains(&FileId(1)));

        state.toggle_line(FileId(2));
        assert!(state.hidden.is_empty());
    }
}
