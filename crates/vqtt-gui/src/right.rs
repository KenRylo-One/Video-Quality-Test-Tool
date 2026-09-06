//! The right column: the graph plot and the results table.

use crate::plot as renderer;
use crate::theme::Tokens;
use crate::widgets::{card, color_swatch, empty_state, mono, sans, section_header};
use egui::Ui;
use std::collections::{BTreeSet, HashMap};
use vqtt_core::media::Rational;
use vqtt_core::metric::MetricId;
use vqtt_core::palette::series_color;
use vqtt_core::plot::{PlotRequest, SeriesInput, build_scenes};
use vqtt_core::pooling::Pooled;
use vqtt_core::set::FileId;
use vqtt_run::Session;

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
    /// True while the run is still working. Export waits for it, because a record
    /// written half way through would name numbers that are still moving.
    pub is_running: bool,
}

/// What the right column asks the window to do after this frame is drawn.
#[derive(Default, PartialEq)]
pub enum Request {
    #[default]
    Nothing,
    Export,
    /// Open the frame viewer on this encode, metric and frame.
    OpenFrame(FileId, MetricId, u64),
}

pub fn show(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    run: &RunView,
    state: &mut PlotUi,
) -> Request {
    section_header(ui, tokens, "Plot");

    let finished = finished_metrics(session, run);
    let mut request = Request::Nothing;
    if finished.is_empty() {
        empty_state(ui, tokens, "Run a comparison to see the plot.", PLOT_HEIGHT);
    } else {
        request = plot_section(ui, tokens, session, run, state, &finished);
    }

    ui.add_space(16.0);
    ui.label(sans("Results", 15.0, tokens.text));
    ui.add_space(6.0);

    if run.results.is_empty() {
        empty_state(ui, tokens, "The numbers appear here.", 120.0);
        return request;
    }
    numbers_table(ui, tokens, session, run.results);
    request
}

/// Every metric that has a result, in registry order, so the tabs never reshuffle.
fn finished_metrics(session: &Session, run: &RunView) -> Vec<MetricId> {
    vqtt_core::metric::REGISTRY
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
) -> Request {
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

    let mut request = Request::Nothing;
    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        request = plot_card(ui, tokens, session, run, state, active);
    });
    request
}

fn plot_card(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    run: &RunView,
    state: &mut PlotUi,
    active: MetricId,
) -> Request {
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

    let mut asked = Request::Nothing;
    ui.horizontal(|ui| {
        ui.label(mono(active.def().label, 14.0, tokens.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let export = ui
                .add_enabled(
                    !run.is_running,
                    egui::Button::new(sans("export", 11.0, tokens.text_secondary)).frame(false),
                )
                .on_hover_text(
                    "Writes the record, both CSV files, the command log and a graph for \
each metric into one folder.",
                )
                .on_disabled_hover_text("The run is still working.");
            if export.clicked() {
                asked = Request::Export;
            }
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
        renderer::paint(&painter, tokens.theme, scene, top);
        top.y += scene.size.1;
    }

    read_pointer(ui, &response, state, &scenes, run);
    if response.clicked()
        && let Some(opened) = clicked_frame(&response, &scenes, &inputs, active)
    {
        asked = opened;
    }
    ui.add_space(6.0);
    legend(ui, tokens, session, state, active, run);
    hover_readout(ui, tokens, state, &inputs, active, run);
    asked
}

/// Turns a click into the frame the frame viewer opens.
///
/// The plot is decimated, so one pixel column can cover a hundred frames. The column
/// already names the worst of them, which is what makes clicking a spike open the
/// spike. The nearest line to the click decides which encode.
fn clicked_frame(
    response: &egui::Response,
    scenes: &[vqtt_core::plot::Scene],
    inputs: &[SeriesInput],
    active: MetricId,
) -> Option<Request> {
    let pointer = response.interact_pointer_pos()?;
    let at = pointer - response.rect.min.to_vec2();

    let mut best: Option<(f32, FileId, u64)> = None;
    for scene in scenes {
        let vqtt_core::plot::Body::Lines(shapes) = &scene.body else {
            continue;
        };
        for shape in shapes {
            let Some(column) = shape
                .columns
                .iter()
                .min_by(|left, right| (left.x - at.x).abs().total_cmp(&(right.x - at.x).abs()))
            else {
                continue;
            };
            let distance = (column.mean - at.y).abs() + (column.x - at.x).abs();
            if best.is_none_or(|(closest, _, _)| distance < closest) {
                best = Some((distance, shape.file, column.worst_frame));
            }
        }
    }

    let (_, file, frame) = best?;
    inputs.iter().find(|input| input.file == file)?;
    Some(Request::OpenFrame(file, active, frame))
}

/// Turns the pointer into a hovered frame and a zoom level.
fn read_pointer(
    ui: &Ui,
    response: &egui::Response,
    state: &mut PlotUi,
    scenes: &[vqtt_core::plot::Scene],
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

    // The plot sits on a page that scrolls, so a bare wheel belongs to the page. Zoom
    // takes the modifier, or the reader cannot scroll past the plot without moving it.
    let (notches, zooming) = ui.input(|input| {
        (
            input.smooth_scroll_delta.y,
            input.modifiers.ctrl || input.modifiers.command,
        )
    });
    if zooming && notches.abs() > 0.1 {
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
            let text = if shown {
                tokens.text
            } else {
                tokens.text_muted
            };
            let entry = ui.horizontal(|ui| {
                color_swatch(ui, 10.0, color, shown);
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
            "Hover the plot to read one frame. Hold Ctrl and use the wheel to zoom.",
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

/// The nine column weights of the results grid, as the design's `fr` units.
const COLUMN_WEIGHTS: [f32; 9] = [1.6, 1.0, 0.8, 0.8, 0.9, 0.9, 0.8, 0.8, 0.8];

/// The gap between two columns.
const COLUMN_GAP: f32 = 14.0;

/// The padding inside the left and the right edge of the table.
const TABLE_PAD: f32 = 14.0;

/// The table never draws narrower than this. Below it, the card scrolls sideways.
const TABLE_MIN_WIDTH: f32 = 640.0;

/// The height of one data row.
const ROW_HEIGHT: f32 = 28.0;

/// The height of the header row.
const HEADER_HEIGHT: f32 = 30.0;

/// The height of the scrolling body, below the header that stays.
const BODY_HEIGHT: f32 = 216.0;

/// The side of the color swatch.
const SWATCH: f32 = 9.0;

/// The gap between the swatch and the file name.
const SWATCH_GAP: f32 = 8.0;

/// The nine column headers, in grid order.
const HEADERS: [&str; 9] = [
    "VIDEO", "METRIC", "MEAN", "MEDIAN", "WORST 5%", "WORST 1%", "MIN", "MAX", "STD DEV",
];

/// The left edge and the width of every column, for a table this wide.
///
/// The design lays the table out as a grid of fixed proportions, so the columns share
/// the width in that proportion rather than each taking what its own text needs. The
/// proportion is the only thing that holds a header over its own numbers.
fn columns(width: f32) -> [(f32, f32); 9] {
    let weight: f32 = COLUMN_WEIGHTS.iter().sum();
    let free = (width - 2.0 * TABLE_PAD - COLUMN_GAP * 8.0).max(1.0);
    let mut out = [(0.0, 0.0); 9];
    let mut left = TABLE_PAD;
    for (column, share) in COLUMN_WEIGHTS.iter().enumerate() {
        let cell = free * share / weight;
        out[column] = (left, cell);
        left += cell + COLUMN_GAP;
    }
    out
}

fn numbers_table(
    ui: &mut Ui,
    tokens: &Tokens,
    session: &Session,
    results: &HashMap<(FileId, MetricId), Pooled>,
) {
    let width = ui.available_width().max(TABLE_MIN_WIDTH);

    egui::Frame::default()
        .fill(tokens.sunken)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .corner_radius(crate::theme::RADIUS)
        .show(ui, |ui| {
            // An outer horizontal scroll carries the header and the body together, so
            // panning sideways never lets the header drift off its own columns. Only
            // the inner vertical scroll moves under a scroll down, which is what keeps
            // the header in view.
            egui::ScrollArea::horizontal()
                .id_salt("results-table")
                .show(ui, |ui| {
                    header_row(ui, tokens, width);
                    egui::ScrollArea::vertical()
                        .id_salt("results-rows")
                        .max_height(BODY_HEIGHT)
                        .show(ui, |ui| {
                            for encode in session.files.encodes() {
                                let slot = session.files.slot_of(encode.id);
                                for metric in vqtt_core::metric::REGISTRY.iter().map(|def| def.id) {
                                    let Some(pooled) = results.get(&(encode.id, metric)) else {
                                        continue;
                                    };
                                    data_row(
                                        ui,
                                        tokens,
                                        slot,
                                        &encode.label,
                                        metric,
                                        pooled,
                                        width,
                                    );
                                }
                            }
                        });
                });
        });
}

fn header_row(ui: &mut Ui, tokens: &Tokens, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, HEADER_HEIGHT), egui::Sense::hover());
    let painter = ui.painter().clone();
    for ((left, cell), label) in columns(width).into_iter().zip(HEADERS) {
        let at = egui::Rect::from_min_size(
            egui::pos2(rect.left() + left, rect.top()),
            egui::vec2(cell, rect.height()),
        );
        cell_text(&painter, at, label, 10.5, tokens.text_muted);
    }
    painter.hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0, tokens.border),
    );
}

fn data_row(
    ui: &mut Ui,
    tokens: &Tokens,
    slot: Option<usize>,
    encode_label: &str,
    metric: MetricId,
    pooled: &Pooled,
    width: f32,
) {
    let def = metric.def();
    let suffix = def.unit.suffix();
    let bad_end = def.direction.bad_end();
    let worst_5 = match bad_end {
        vqtt_core::metric::Percentile::P5 => pooled.p5,
        vqtt_core::metric::Percentile::P95 => pooled.p95,
    };
    let worst_1 = match bad_end {
        vqtt_core::metric::Percentile::P5 => pooled.p1,
        vqtt_core::metric::Percentile::P95 => pooled.p99,
    };

    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
    let painter = ui.painter().clone();
    let grid = columns(width);
    let at = |column: usize| {
        let (left, cell) = grid[column];
        egui::Rect::from_min_size(
            egui::pos2(rect.left() + left, rect.top()),
            egui::vec2(cell, rect.height()),
        )
    };

    // The swatch carries the colour of this encode's line on the plot, so a row reads
    // back to a line without counting either of them.
    let color = series_color(tokens.theme, slot.unwrap_or(0))
        .map(|series| egui::Color32::from_rgb(series.r, series.g, series.b))
        .unwrap_or(tokens.text_muted);
    let video = at(0);
    let swatch = egui::Rect::from_min_size(
        egui::pos2(video.left(), video.center().y - SWATCH / 2.0),
        egui::vec2(SWATCH, SWATCH),
    );
    painter.rect_filled(swatch, 2.0, color);
    let name = egui::Rect::from_min_max(
        egui::pos2(video.left() + SWATCH + SWATCH_GAP, video.top()),
        video.max,
    );
    cell_text(&painter, name, encode_label, 11.5, tokens.text);

    let primary = tokens.text;
    let secondary = tokens.text_secondary;
    let numbers = [
        (1, def.label.to_string(), secondary),
        (2, format!("{:.2}{suffix}", pooled.mean), primary),
        (3, format!("{:.2}{suffix}", pooled.median), primary),
        (4, format!("{worst_5:.2}{suffix}"), primary),
        (5, format!("{worst_1:.2}{suffix}"), primary),
        (6, format!("{:.2}{suffix}", pooled.min), secondary),
        (7, format!("{:.2}{suffix}", pooled.max), secondary),
        (8, format!("{:.2}{suffix}", pooled.stdev), secondary),
    ];
    for (column, text, color) in numbers {
        cell_text(&painter, at(column), &text, 11.5, color);
    }

    painter.hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0, tokens.border),
    );
}

/// One cell of the grid, cut short with an ellipsis rather than pushing its neighbour.
///
/// A cell that grows to fit its own text moves every column after it, and a long file
/// name is normal. The text is laid out to the width of its column and no wider.
fn cell_text(
    painter: &egui::Painter,
    rect: egui::Rect,
    text: &str,
    size: f32,
    color: egui::Color32,
) {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            font_id: egui::FontId::new(
                size,
                egui::FontFamily::Name(crate::fonts::MONO_FAMILY.into()),
            ),
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width());
    let galley = painter.layout_job(job);
    let at = egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    painter.galley(at, galley, color);
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

    /// The header and the rows read the same grid, so a column that moved for one of
    /// them moved for both. What must hold is that the grid fills the table exactly.
    #[test]
    fn the_results_grid_fills_the_table_and_never_overlaps() {
        for width in [TABLE_MIN_WIDTH, 900.0, 1440.0] {
            let grid = columns(width);
            assert_eq!(
                grid[0].0, TABLE_PAD,
                "the first column starts at the padding"
            );

            for pair in grid.windows(2) {
                let (left, cell) = pair[0];
                let (next, _) = pair[1];
                assert!(cell > 0.0, "a column at {width} has no width");
                assert!(
                    (next - (left + cell) - COLUMN_GAP).abs() < 0.01,
                    "the gap between two columns at {width} is not the design gap"
                );
            }

            let (left, cell) = grid[8];
            assert!(
                (width - (left + cell) - TABLE_PAD).abs() < 0.01,
                "the last column at {width} does not end at the padding"
            );
        }
    }

    /// The `Video` column is the widest because a file name is the longest text in the
    /// table. It is cut with an ellipsis rather than allowed to push its neighbours.
    #[test]
    fn the_video_column_is_the_widest_one() {
        let grid = columns(900.0);
        let widest = grid.iter().map(|(_, cell)| *cell).fold(f32::MIN, f32::max);

        assert_eq!(grid[0].1, widest);
    }
}
