//! The window.
//!
//! One page, two columns. The left column is what you set. The right column is what you
//! get.

use crate::files::{self, FilesAction};
use crate::metrics::{self, MetricsAction, MetricsUi};
use crate::notes;
use crate::right::PlotUi;
use crate::run::RunState;
use crate::settings_panel::{self, SettingsUi};
use crate::theme::Tokens;
use crate::widgets::{mono, sans};
use crate::{right, widgets};
use std::collections::HashMap;
use vqa_run::Session;

/// The width of the left column. It does not stretch.
const LEFT_COLUMN: f32 = 400.0;

/// The gap between the two columns.
const COLUMN_GAP: f32 = 32.0;

/// The gap between the sections of the left column.
const SECTION_GAP: f32 = 36.0;

/// The height of the title bar. It never scrolls.
const TITLE_BAR: f32 = 46.0;

/// The name in the title bar. The final name is not chosen.
const APP_NAME: &str = "Video Compression Analyzer";

/// The window.
pub struct VqaApp {
    session: Session,
    tokens: Tokens,
    metrics_ui: MetricsUi,
    settings_ui: SettingsUi,
    plot_ui: PlotUi,
    run: Option<RunState>,
}

impl VqaApp {
    /// Builds the window and reads the settings.
    pub fn new(context: &egui::Context) -> Self {
        context.set_fonts(crate::fonts::definitions());

        let session = Session::load();
        let tokens = Tokens::for_choice(session.settings.theme);
        tokens.apply(context);

        let mut settings_ui = SettingsUi::default();
        settings_ui.sync(&session);
        // A new user has none of the binaries. The first thing that the tool shows must
        // teach, so the panel opens on its own.
        settings_ui.open = session.inventory.is_empty();

        Self {
            session,
            tokens,
            metrics_ui: MetricsUi::default(),
            settings_ui,
            plot_ui: PlotUi::default(),
            run: None,
        }
    }

    /// Adds every file that the user dropped on the window.
    fn take_dropped_files(&mut self, context: &egui::Context) {
        let dropped: Vec<std::path::PathBuf> = context.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });
        for path in dropped {
            self.session.add_file(&path);
        }
    }

    /// The title bar.
    fn title_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("title-bar")
            .exact_size(TITLE_BAR)
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(self.tokens.window)
                    .stroke(egui::Stroke::new(1.0, self.tokens.border))
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let dot = ui.allocate_space(egui::vec2(6.0, 6.0));
                    ui.painter()
                        .circle_filled(dot.1.center(), 3.0, self.tokens.accent);
                    ui.add_space(6.0);
                    ui.label(mono(APP_NAME, 11.5, self.tokens.text));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let gear = ui.add(
                            egui::Button::new(mono("⚙", 13.0, self.tokens.text))
                                .fill(self.tokens.window)
                                .stroke(egui::Stroke::new(1.0, self.tokens.border)),
                        );
                        if gear.on_hover_text("Settings").clicked() {
                            self.settings_ui.open = !self.settings_ui.open;
                            self.settings_ui.sync(&self.session);
                        }

                        ui.add_space(8.0);
                        if self.run.as_ref().is_some_and(RunState::is_running) {
                            if ui
                                .add(
                                    egui::Button::new(sans("Cancel", 12.0, self.tokens.text_muted))
                                        .fill(self.tokens.window)
                                        .stroke(egui::Stroke::new(1.0, self.tokens.border)),
                                )
                                .clicked()
                                && let Some(run_state) = &self.run
                            {
                                run_state.cancel();
                            }
                            ui.add_space(10.0);
                            self.run_progress(ui);
                        } else {
                            let can_run = self.session.files.reference().is_some()
                                && !self.session.runnable_metrics().is_empty();
                            let run_button = ui.add_enabled(
                                can_run,
                                egui::Button::new(sans("Run", 12.5, self.tokens.on_accent))
                                    .fill(self.tokens.accent),
                            );
                            if !can_run {
                                run_button.on_hover_text(
                                    "Add a reference and tick a metric that this machine can run.",
                                );
                            } else if run_button.clicked() {
                                self.run = RunState::start(&self.session);
                            }
                        }
                    });
                });
            });
    }

    /// Dims the page behind the Settings panel, and reports a click on that dimming.
    ///
    /// The overlay is a real surface, not paint. It has to take the click, or a button
    /// underneath answers it and the panel stays open over a page that just changed.
    fn dim_behind_settings(&self, context: &egui::Context) -> bool {
        let mut behind = context.viewport_rect();
        behind.max.x -= settings_panel::WIDTH;
        if behind.width() <= 0.0 {
            return false;
        }

        egui::Area::new(egui::Id::new("settings-overlay"))
            .order(egui::Order::Middle)
            .fixed_pos(behind.min)
            .show(context, |ui| {
                let (rect, response) = ui.allocate_exact_size(behind.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(90));
                response.clicked()
            })
            .inner
    }

    /// The metric a lane is on, how far it has reached, and a bar for the share.
    ///
    /// A back end that reports no frame still gets a name and a moving bar, because a
    /// still bar reads as a stall rather than as a run with no counter.
    fn run_progress(&self, ui: &mut egui::Ui) {
        let Some(run_state) = &self.run else {
            return;
        };

        let label = match run_state.progress {
            Some((metric, frame)) => {
                format!("{} · frame {frame}", metric.def().label)
            }
            None => "Running…".to_string(),
        };
        ui.label(mono(label, 11.0, self.tokens.text_secondary));
        ui.add_space(10.0);

        let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 6.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, crate::theme::RADIUS, self.tokens.sunken);
        if let Some(share) = run_state.fraction() {
            let mut filled = rect;
            filled.set_width(rect.width() * share);
            ui.painter()
                .rect_filled(filled, crate::theme::RADIUS, self.tokens.accent);
        }
    }

    /// The left column: the files, and the metric setup.
    fn left_column(&mut self, ui: &mut egui::Ui) {
        ui.set_width(LEFT_COLUMN);

        match files::show(ui, &self.tokens, &self.session) {
            FilesAction::None => {}
            FilesAction::Promote(id) => self.session.promote_to_reference(id),
            FilesAction::Remove(id) => self.session.remove_file(id),
            FilesAction::Move(moved, target) => self.session.files.move_before(moved, target),
            FilesAction::Import(paths) => {
                for path in paths {
                    self.session.add_file(&path);
                }
            }
        }

        ui.add_space(SECTION_GAP);

        match metrics::show(ui, &self.tokens, &self.session, &mut self.metrics_ui) {
            MetricsAction::None => {}
            MetricsAction::Toggle(id, ticked) => self.session.toggle_metric(id, ticked),
            MetricsAction::Preset(name) => self.session.apply_preset(&name),
            MetricsAction::WholeFile(whole) => self.session.selection.whole_file = whole,
            MetricsAction::Range(first, last) => {
                self.session.selection.first_frame = first;
                self.session.selection.last_frame = last;
            }
        }

        if !self.session.probe_problems.is_empty() {
            ui.add_space(12.0);
            ui.label(mono("NOTE", 10.0, self.tokens.warn));
            for (path, problem) in self.session.probe_problems.iter().rev().take(3) {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                widgets::note_line(ui, &self.tokens, &format!("{name}: {problem}"));
            }
        }
    }
}

impl eframe::App for VqaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.take_dropped_files(&context);

        let wanted = Tokens::for_choice(self.session.settings.theme);
        if wanted != self.tokens {
            self.tokens = wanted;
            self.tokens.apply(&context);
        }

        self.title_bar(ui);

        // A run reports its result over a channel from a background thread. Reading it
        // here, once a frame, keeps the interface thread from ever waiting on a process.
        if let Some(run_state) = &mut self.run {
            let still_running = run_state.poll();
            if still_running {
                context.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }

        if self.settings_ui.open {
            let mut settings_ui = std::mem::take(&mut self.settings_ui);
            settings_ui.open = true;
            egui::Panel::right("settings")
                .exact_size(settings_panel::WIDTH)
                .resizable(false)
                .frame(
                    egui::Frame::default()
                        .fill(self.tokens.panel)
                        .stroke(egui::Stroke::new(1.0, self.tokens.border))
                        .inner_margin(egui::Margin::same(14)),
                )
                .show(ui, |ui| {
                    if settings_panel::show(ui, &self.tokens, &mut self.session, &mut settings_ui) {
                        self.session.save_settings();
                    }
                });
            self.settings_ui = settings_ui;
            if self.dim_behind_settings(&context) {
                self.settings_ui.open = false;
            }
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(self.tokens.window)
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| self.left_column(ui));
                        ui.add_space(COLUMN_GAP);
                        let empty_results = HashMap::new();
                        let empty_series = HashMap::new();
                        let view = match &self.run {
                            Some(run_state) => right::RunView {
                                results: &run_state.results,
                                series: &run_state.series,
                                first_frame: run_state.first_frame,
                                frame_rate: run_state.frame_rate,
                            },
                            None => right::RunView {
                                results: &empty_results,
                                series: &empty_series,
                                first_frame: 0,
                                frame_rate: vqa_core::media::Rational::ZERO,
                            },
                        };
                        ui.vertical(|ui| {
                            right::show(ui, &self.tokens, &self.session, &view, &mut self.plot_ui)
                        });
                    });

                    let has_results = self
                        .run
                        .as_ref()
                        .is_some_and(|run_state| !run_state.results.is_empty());
                    if has_results {
                        ui.add_space(SECTION_GAP);
                        let no_notes = Vec::new();
                        let run_notes = self
                            .run
                            .as_ref()
                            .map_or(&no_notes, |run_state| &run_state.notes);
                        notes::show(ui, &self.tokens, run_notes);
                    }
                });
            });
    }
}
