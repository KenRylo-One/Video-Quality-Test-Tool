//! Section 5: the frame viewer.
//!
//! Every metric has known faults, so a measurement ends with a person looking at the
//! pixels. This opens as an accordion below the plot and closes again, which is what
//! keeps the single-window model honest.
//!
//! Two rules here come from the subject matter and not from taste. The surround is flat
//! neutral gray and the same in both themes, and no tint, filter or opacity ever
//! touches the three images. A colored surround changes how a person judges an image,
//! and judging images is the whole purpose of this panel.

use crate::theme::{FRAME_VIEWER_GRAY, Tokens};
use crate::widgets::{card, mono, sans};
use egui::{ColorImage, TextureHandle, Ui};
use std::path::Path;
use std::sync::mpsc::Receiver;
use vqtt_core::frames::{FrameValue, step_from};
use vqtt_core::metric::MetricId;
use vqtt_core::set::FileId;

/// The gains the difference image offers.
const GAINS: [u32; 5] = [1, 2, 4, 8, 16];

/// The height of one image slot.
const SLOT_HEIGHT: f32 = 150.0;

/// What came back from the worker thread.
type Extraction = vqtt_core::Result<vqtt_run::ExtractedFrame>;

/// One loaded image, and where it came from.
struct Slot {
    label: &'static str,
    texture: Option<TextureHandle>,
}

/// The state of the viewer.
#[derive(Default)]
pub struct FrameViewer {
    pub open: bool,
    /// The frame on screen, and the encode and metric it was opened from.
    pub frame: Option<u64>,
    pub encode: Option<FileId>,
    pub metric: Option<MetricId>,
    pub gain: u32,
    /// True when the images are drawn at their own pixel size rather than fitted.
    pub actual_size: bool,
    /// The worst-first order of the active series, for the worse and better controls.
    order: Vec<FrameValue>,
    slots: Vec<Slot>,
    /// The extraction running now. The window never waits on it.
    pending: Option<Receiver<Extraction>>,
    loading_frame: Option<u64>,
    problem: Option<String>,
    commands: Vec<String>,
}

/// What the viewer asks the window to do.
pub enum Ask {
    Nothing,
    /// Extract this frame at this gain.
    Extract(u64, u32),
}

impl FrameViewer {
    /// Opens the viewer on one frame, and remembers the order to step through.
    pub fn open_at(
        &mut self,
        encode: FileId,
        metric: MetricId,
        frame: u64,
        order: Vec<FrameValue>,
    ) {
        let changed = self.encode != Some(encode) || self.metric != Some(metric);
        self.open = true;
        self.encode = Some(encode);
        self.metric = Some(metric);
        self.frame = Some(frame);
        self.order = order;
        if self.gain == 0 {
            self.gain = 4;
        }
        if changed {
            self.slots.clear();
        }
    }

    /// The value the active metric measured at the frame on screen.
    pub fn value(&self) -> Option<f32> {
        let frame = self.frame?;
        self.order
            .iter()
            .find(|entry| entry.frame == frame)
            .map(|entry| entry.value)
    }

    /// Where this frame sits in the worst-first order, counting from one.
    pub fn rank(&self) -> Option<usize> {
        let frame = self.frame?;
        self.order
            .iter()
            .position(|entry| entry.frame == frame)
            .map(|at| at + 1)
    }

    /// Takes the result of a finished extraction, if one arrived.
    pub fn poll(&mut self, ui: &Ui) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let Ok(outcome) = receiver.try_recv() else {
            return;
        };
        self.pending = None;
        self.loading_frame = None;

        match outcome {
            Ok(extracted) => {
                self.problem = None;
                self.commands = extracted.commands;
                self.slots = vec![
                    load(ui, "reference", &extracted.reference),
                    load(ui, "encode", &extracted.encode),
                    load(ui, "difference", &extracted.difference),
                ];
            }
            Err(error) => {
                self.problem = Some(error.to_string());
                self.slots.clear();
            }
        }
    }

    /// Hands the worker channel over, so the window can start an extraction.
    pub fn expect(&mut self, frame: u64, receiver: Receiver<Extraction>) {
        self.pending = Some(receiver);
        self.loading_frame = Some(frame);
        self.problem = None;
    }

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    fn step(&mut self, step: i64) {
        let Some(frame) = self.frame else {
            return;
        };
        if let Some(next) = step_from(&self.order, frame, step)
            && next != frame
        {
            self.frame = Some(next);
            self.slots.clear();
        }
    }
}

fn load(ui: &Ui, label: &'static str, path: &Path) -> Slot {
    let texture = vqtt_run::read_png(path).ok().map(|image| {
        let size = [image.width as usize, image.height as usize];
        ui.ctx().load_texture(
            format!("frame-{label}"),
            ColorImage::from_rgba_unmultiplied(size, &image.pixels),
            egui::TextureOptions::LINEAR,
        )
    });
    Slot { label, texture }
}

/// Draws the viewer and reports what the reader asked for.
pub fn show(ui: &mut Ui, tokens: &Tokens, viewer: &mut FrameViewer, note: Option<&str>) -> Ask {
    let mut ask = Ask::Nothing;
    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        header(ui, tokens, viewer);
        if !viewer.open {
            return;
        }

        ui.add_space(8.0);
        images(ui, viewer);
        ui.add_space(8.0);
        ask = controls(ui, tokens, viewer);

        if let Some(problem) = &viewer.problem {
            ui.add_space(6.0);
            ui.label(sans(problem, 11.5, tokens.warn));
        }
        if let Some(note) = note {
            ui.add_space(6.0);
            ui.label(sans(note, 11.0, tokens.note_text));
        }
    });
    ask
}

fn header(ui: &mut Ui, tokens: &Tokens, viewer: &mut FrameViewer) {
    let response = ui
        .horizontal(|ui| {
            ui.set_width(ui.available_width());
            ui.label(mono(
                if viewer.open { "▾" } else { "▸" },
                11.0,
                tokens.text_muted,
            ));
            ui.label(sans("Frame viewer", 15.0, tokens.text).strong());

            if let (Some(frame), Some(metric)) = (viewer.frame, viewer.metric) {
                ui.add_space(8.0);
                ui.label(mono(format!("frame {frame}"), 12.0, tokens.text_secondary));
                if let Some(value) = viewer.value() {
                    ui.label(mono(
                        format!("· {} {value:.3}", metric.def().label),
                        12.0,
                        tokens.text_secondary,
                    ));
                }
                if let Some(rank) = viewer.rank() {
                    ui.label(mono(format!("· worst #{rank}"), 11.0, tokens.text_muted));
                }
            }
        })
        .response;

    let clicked = ui
        .interact(
            response.rect,
            ui.id().with("frame-viewer-open"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked();
    if clicked {
        viewer.open = !viewer.open;
    }
}

/// The three images, on a flat neutral gray that is the same in both themes.
fn images(ui: &mut Ui, viewer: &FrameViewer) {
    egui::Frame::default()
        .fill(FRAME_VIEWER_GRAY)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let slot_width = ((ui.available_width() - 20.0) / 3.0).max(60.0);

            ui.horizontal(|ui| {
                if viewer.slots.is_empty() {
                    for label in ["reference", "encode", "difference"] {
                        placeholder(ui, label, slot_width, viewer.is_loading());
                    }
                    return;
                }
                for slot in &viewer.slots {
                    match &slot.texture {
                        // No tint, no filter and no opacity. The theme never reaches
                        // inside these three frames.
                        Some(texture) => {
                            let size = fit(texture.size_vec2(), slot_width, viewer.actual_size);
                            ui.add(egui::Image::new(texture).fit_to_exact_size(size));
                        }
                        None => placeholder(ui, slot.label, slot_width, false),
                    }
                }
            });
        });
}

fn fit(size: egui::Vec2, slot_width: f32, actual: bool) -> egui::Vec2 {
    if actual {
        return size;
    }
    let scale = (slot_width / size.x).min(SLOT_HEIGHT / size.y).min(1.0);
    size * scale
}

fn placeholder(ui: &mut Ui, label: &str, width: f32, loading: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, SLOT_HEIGHT), egui::Sense::hover());
    let text = if loading { "reading…" } else { label };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::new(
            11.0,
            egui::FontFamily::Name(crate::fonts::MONO_FAMILY.into()),
        ),
        egui::Color32::from_rgb(0x33, 0x33, 0x33),
    );
}

fn controls(ui: &mut Ui, tokens: &Tokens, viewer: &mut FrameViewer) -> Ask {
    let mut ask = Ask::Nothing;
    let before = (viewer.frame, viewer.gain);

    ui.horizontal_wrapped(|ui| {
        ui.label(sans("gain", 11.0, tokens.text_muted));
        for gain in GAINS {
            let chosen = viewer.gain == gain;
            let label = sans(
                format!("x{gain}"),
                11.0,
                if chosen {
                    tokens.on_accent
                } else {
                    tokens.text
                },
            );
            let button =
                egui::Button::new(label).fill(if chosen { tokens.accent } else { tokens.sunken });
            if ui.add(button).clicked() {
                viewer.gain = gain;
            }
        }

        ui.add_space(10.0);
        ui.checkbox(
            &mut viewer.actual_size,
            sans("1:1", 11.0, tokens.text_secondary),
        )
        .on_hover_text("Draws each image at its own pixel size.");

        ui.add_space(10.0);
        if ui
            .add(egui::Button::new(sans("← worse", 11.0, tokens.text)))
            .on_hover_text("The next frame that measured worse.")
            .clicked()
        {
            viewer.step(-1);
        }
        if ui
            .add(egui::Button::new(sans("better →", 11.0, tokens.text)))
            .on_hover_text("The next frame that measured better.")
            .clicked()
        {
            viewer.step(1);
        }
    });

    if before != (viewer.frame, viewer.gain)
        && let Some(frame) = viewer.frame
    {
        // A new gain redraws only the difference, because the two stills carry no gain
        // in their names and are already on disk.
        viewer.slots.clear();
        ask = Ask::Extract(frame, viewer.gain);
    }

    if !viewer.commands.is_empty() {
        ui.add_space(4.0);
        ui.collapsing(
            sans("the commands that made these", 11.0, tokens.text_muted),
            |ui| {
                for command in &viewer.commands {
                    ui.label(mono(command, 10.0, tokens.text_muted));
                }
            },
        );
    }
    ask
}

#[cfg(test)]
mod tests {
    use super::*;
    use vqtt_core::frames::worst_frames;
    use vqtt_core::metric::Direction;

    fn viewer_on(values: &[f32], direction: Direction, frame: u64) -> FrameViewer {
        let mut viewer = FrameViewer::default();
        let metric = if direction == Direction::LowerIsBetter {
            MetricId::Cambi
        } else {
            MetricId::PsnrY
        };
        viewer.open_at(FileId(1), metric, frame, worst_frames(values, 0, direction));
        viewer
    }

    #[test]
    fn worse_steps_towards_the_worse_frame_for_a_low_is_better_metric() {
        let mut viewer = viewer_on(&[0.0, 12.0, 3.0], Direction::LowerIsBetter, 0);

        // The order is 12.0, 3.0, 0.0, so frame 0 is the best and worse walks back.
        viewer.step(-1);
        assert_eq!(viewer.frame, Some(2));
        viewer.step(-1);
        assert_eq!(viewer.frame, Some(1));
    }

    #[test]
    fn stepping_past_the_worst_frame_stays_there() {
        let mut viewer = viewer_on(&[40.0, 30.0, 50.0], Direction::HigherIsBetter, 1);

        viewer.step(-1);
        assert_eq!(viewer.frame, Some(1), "frame 1 is already the worst");
    }

    #[test]
    fn the_header_reads_the_value_and_the_rank_of_the_frame_on_screen() {
        let viewer = viewer_on(&[40.0, 30.0, 50.0], Direction::HigherIsBetter, 0);

        assert_eq!(viewer.value(), Some(40.0));
        assert_eq!(viewer.rank(), Some(2));
    }

    #[test]
    fn opening_another_encode_drops_the_images_of_the_one_before() {
        let mut viewer = viewer_on(&[40.0, 30.0], Direction::HigherIsBetter, 0);
        viewer.slots = vec![Slot {
            label: "reference",
            texture: None,
        }];

        viewer.open_at(
            FileId(2),
            MetricId::PsnrY,
            0,
            worst_frames(&[40.0], 0, Direction::HigherIsBetter),
        );

        assert!(viewer.slots.is_empty());
    }

    #[test]
    fn a_frame_that_no_series_holds_reports_no_value_and_no_rank() {
        let mut viewer = viewer_on(&[40.0], Direction::HigherIsBetter, 0);
        viewer.frame = Some(99);

        assert_eq!(viewer.value(), None);
        assert_eq!(viewer.rank(), None);
    }
}
