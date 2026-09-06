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
use vqtt_core::metric::MetricId;
use vqtt_run::Session;

/// The width of the left column. It does not stretch.
const LEFT_COLUMN: f32 = 400.0;

/// The gap between the two columns.
const COLUMN_GAP: f32 = 32.0;

/// The gap between the sections of the left column.
const SECTION_GAP: f32 = 36.0;

/// The height of the title bar. It never scrolls.
const TITLE_BAR: f32 = 46.0;

/// The name in the title bar, and the name of the window.
pub const APP_NAME: &str = "Video Quality Test Tool";

/// The window.
pub struct VqttApp {
    session: Session,
    tokens: Tokens,
    metrics_ui: MetricsUi,
    settings_ui: SettingsUi,
    plot_ui: PlotUi,
    run: Option<RunState>,
    /// A binary search running on a worker thread.
    scan: Option<std::sync::mpsc::Receiver<vqtt_run::BinaryScan>>,
    /// What the last export did, as one line for the Notes section.
    export_report: Option<String>,
    frame_viewer: crate::frame_viewer::FrameViewer,
}

impl VqttApp {
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
            scan: None,
            export_report: None,
            frame_viewer: crate::frame_viewer::FrameViewer::default(),
        }
    }

    /// Starts an extraction on a worker thread. An exact frame select on a long-GOP
    /// file seeks to a keyframe and decodes forward, so the window never waits on it.
    fn extract_frame(&mut self, frame: u64, gain: u32) {
        let Some(encode) = self.frame_viewer.encode else {
            return;
        };
        let Some(run_state) = &self.run else {
            return;
        };
        let Some(job) = run_state.job_for(encode) else {
            return;
        };
        let Some(program) = self
            .session
            .inventory
            .get(vqtt_core::capability::BinaryId::Ffmpeg)
            .map(|found| found.path.clone())
        else {
            return;
        };

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(vqtt_run::extract_frame(&job, &program, frame, gain));
        });
        self.frame_viewer.expect(frame, receiver);
    }

    /// The note the viewer carries when the active tab is a Vship metric.
    ///
    /// FFVship reads the files itself, so the corrections never reach it. The picture
    /// on screen is corrected and the score beside it was measured on pixels that were
    /// not.
    fn viewer_note(&self) -> Option<&'static str> {
        let metric = self.frame_viewer.metric?;
        let by_vship =
            metric.def().providers.first().is_some_and(|provider| {
                provider.binary == vqtt_core::capability::BinaryId::Ffvship
            });
        by_vship.then_some(
            "FFVship reads the files itself, so the colour range and resolution corrections \
did not reach this score. The images below are corrected and the number is not.",
        )
    }

    /// The Notes lines, plus whatever the last export had to say.
    fn note_lines(&self) -> Vec<String> {
        let mut lines = self
            .run
            .as_ref()
            .map(RunState::note_lines)
            .unwrap_or_default();
        if let Some(report) = &self.export_report {
            lines.push(report.clone());
        }
        lines
    }

    /// The folder Settings holds, or the one this system keeps documents in.
    ///
    /// An export never stops to ask. Settings holds the path when the user wants
    /// another one, and an empty setting means the system default, which is what the
    /// field in Settings says.
    fn resolve_export_folder(&mut self) -> Option<std::path::PathBuf> {
        let folder = match &self.session.settings.export_folder {
            Some(folder) => folder.clone(),
            None => vqtt_run::dirs::folder(vqtt_run::dirs::Kind::Exports)?,
        };
        std::fs::create_dir_all(&folder).ok()?;
        Some(folder)
    }

    /// Writes the run to a folder the user chooses, and remembers it for next time.
    fn export_run(&mut self) {
        let Some(outcome) = self
            .run
            .as_ref()
            .map(|run_state| run_state.outcome(self.tokens.theme))
        else {
            return;
        };
        let Some(parent) = self.resolve_export_folder() else {
            return;
        };

        self.export_report = Some(
            match vqtt_run::write_run(&parent, &self.session, &outcome, crate::fonts::FACES) {
                Ok(exported) => format!(
                    "Wrote {} files to {}.",
                    exported.files.len(),
                    exported.folder.display()
                ),
                Err(error) => format!("The export did not finish: {error}"),
            },
        );
    }

    /// Copies one cached frame-viewer still to the export folder.
    ///
    /// The still is already on disk from extraction, so this is a copy and never a
    /// re-render.
    fn save_frame_png(
        &mut self,
        path: std::path::PathBuf,
        label: &'static str,
        frame: u64,
        gain: u32,
    ) {
        let Some(folder) = self.resolve_export_folder() else {
            return;
        };
        let metric_key = self.frame_viewer.metric.map_or("metric", MetricId::key);
        let encode_name = self
            .frame_viewer
            .encode
            .and_then(|id| self.session.files.get(id))
            .map(|file| file.label.as_str())
            .unwrap_or("encode");
        let filename = vqtt_run::frame_png_filename(frame, metric_key, encode_name, label, gain);
        let target = folder.join(filename);

        let result = std::fs::copy(&path, &target)
            .map(|_| target)
            .map_err(|error| error.to_string());
        self.frame_viewer.report_save(result);
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
                        let gear = egui::Frame::default()
                            .fill(self.tokens.window)
                            .stroke(egui::Stroke::new(1.0, self.tokens.border))
                            .corner_radius(crate::theme::RADIUS)
                            .inner_margin(egui::Margin::same(4))
                            .show(ui, |ui| widgets::gear_icon(ui, 16.0, self.tokens.text))
                            .inner;
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

    /// Starts a binary search on a worker thread, and takes the answer when it lands.
    ///
    /// Hashing a full FFmpeg build is over a hundred megabytes of reading, and a
    /// graphics back end starts its own device before it prints a version. Neither can
    /// happen on the interface thread without the window going silent.
    fn drive_binary_scan(&mut self, context: &egui::Context) {
        if let Some((id, path)) = self.settings_ui.rescan.take() {
            self.session.settings.set_binary_path(id, path);
            self.session.save_settings();

            let settings = self.session.settings.clone();
            let cache = self.session.cache_snapshot();
            let (sender, receiver) = std::sync::mpsc::channel();
            let signal = context.clone();
            std::thread::spawn(move || {
                let _ = sender.send(vqtt_run::scan_binaries(&settings, cache));
                signal.request_repaint();
            });
            self.scan = Some(receiver);
            self.settings_ui.scanning = true;
        }

        let Some(receiver) = &self.scan else {
            return;
        };
        match receiver.try_recv() {
            Ok(scan) => {
                self.session.apply_scan(scan);
                self.scan = None;
                self.settings_ui.scanning = false;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(std::time::Duration::from_millis(120));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.scan = None;
                self.settings_ui.scanning = false;
            }
        }
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
    /// The metric named here is the one the run is waiting on, never one that already
    /// gave its value. A reader who sees a finished metric's name on a stalled run
    /// blames the wrong back end.
    fn run_progress(&self, ui: &mut egui::Ui) {
        let Some(run_state) = &self.run else {
            return;
        };

        let label = match run_state.current() {
            Some((metric, frame, 0)) => format!("{} · frame {frame}", metric.def().label),
            Some((metric, frame, others)) => {
                format!("{} · frame {frame} · {others} more", metric.def().label)
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

impl eframe::App for VqttApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.take_dropped_files(&context);

        let wanted = Tokens::for_choice(self.session.settings.theme);
        if wanted != self.tokens {
            self.tokens = wanted;
            self.tokens.apply(&context);
        }

        self.title_bar(ui);
        self.drive_binary_scan(&context);

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

        // The extraction lands on a worker thread, so the images are taken here, once a
        // frame, before anything draws them.
        self.frame_viewer.poll(&context);
        if self.frame_viewer.is_loading() {
            context.request_repaint_after(std::time::Duration::from_millis(120));
        }
        let note = self.viewer_note();

        let mut asked_to_export = false;
        let mut open_frame = None;
        let mut wants_frame = None;
        let mut wants_save = None;
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
                                is_running: run_state.is_running(),
                            },
                            None => right::RunView {
                                results: &empty_results,
                                series: &empty_series,
                                first_frame: 0,
                                frame_rate: vqtt_core::media::Rational::ZERO,
                                is_running: false,
                            },
                        };
                        let asked = ui
                            .vertical(|ui| {
                                right::show(
                                    ui,
                                    &self.tokens,
                                    &self.session,
                                    &view,
                                    &mut self.plot_ui,
                                    &mut self.frame_viewer,
                                    note,
                                )
                            })
                            .inner;
                        match asked.request {
                            right::Request::Export => asked_to_export = true,
                            right::Request::OpenFrame(encode, metric, frame) => {
                                open_frame = Some((encode, metric, frame));
                            }
                            right::Request::Nothing => {}
                        }
                        match asked.viewer {
                            crate::frame_viewer::Ask::Extract(frame, gain) => {
                                wants_frame = Some((frame, gain));
                            }
                            crate::frame_viewer::Ask::Save {
                                path,
                                label,
                                frame,
                                gain,
                            } => wants_save = Some((path, label, frame, gain)),
                            crate::frame_viewer::Ask::Nothing => {}
                        }
                    });

                    // A run that measured nothing still has notes worth reading. That
                    // is the run where the reader most needs to know what went wrong.
                    if let Some(run_state) = &self.run
                        && run_state.has_something_to_say()
                    {
                        ui.add_space(SECTION_GAP);
                        notes::show(ui, &self.tokens, &self.note_lines());
                    }
                });
            });

        // These write files, start threads or open a picker, so they run after the
        // frame rather than inside the closure that is still borrowing the session.
        if asked_to_export {
            self.export_run();
        }
        if let Some((encode, metric, frame)) = open_frame
            && let Some(run_state) = &self.run
        {
            let order = run_state.worst_frames(encode, metric);
            self.frame_viewer.open_at(encode, metric, frame, order);
            wants_frame = Some((frame, self.frame_viewer.gain.max(4)));
        }
        if let Some((frame, gain)) = wants_frame {
            self.extract_frame(frame, gain);
        }
        if let Some((path, label, frame, gain)) = wants_save {
            self.save_frame_png(path, label, frame, gain);
        }
    }
}
