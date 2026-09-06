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
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use vqtt_core::frames::{FrameValue, step_from};
use vqtt_core::metric::MetricId;
use vqtt_core::set::FileId;

/// The gains the difference image offers.
const GAINS: [u32; 5] = [1, 2, 4, 8, 16];

/// The height of one image slot.
const SLOT_HEIGHT: f32 = 150.0;

/// How wide a band around the wipe line takes a drag.
const WIPE_GRAB: f32 = 14.0;

/// The width of the grip drawn on the wipe line.
const WIPE_GRIP: f32 = 4.0;

/// The size of the spinner that stands in for an image still being made.
const SPINNER: f32 = 22.0;

/// What came back from the worker thread.
type Extraction = vqtt_core::Result<vqtt_run::ExtractedFrame>;

/// One loaded image, and where it came from.
struct Slot {
    label: &'static str,
    path: PathBuf,
    texture: Option<TextureHandle>,
}

/// Which tile "save PNG" acts on.
///
/// Defaults to the encode, since that is the tile the score on screen was measured
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    Reference,
    #[default]
    Encode,
    Difference,
}

impl Focus {
    fn label(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Encode => "encode",
            Self::Difference => "difference",
        }
    }
}

/// The state of the viewer.
pub struct FrameViewer {
    pub open: bool,
    /// The frame on screen, and the encode and metric it was opened from.
    pub frame: Option<u64>,
    pub encode: Option<FileId>,
    pub metric: Option<MetricId>,
    pub gain: u32,
    /// True when the images are drawn at their own pixel size rather than fitted.
    pub actual_size: bool,
    /// True when the reference and the encode share one pane behind a draggable line,
    /// instead of sitting in two tiles side by side.
    pub wipe: bool,
    /// Where the wipe line sits, from 0.0 (all reference) to 1.0 (all encode).
    pub wipe_position: f32,
    /// The tile "save PNG" acts on. Set by clicking a tile, or a side of the wipe line.
    pub focused: Focus,
    /// The worst-first order of the active series, for the worse and better controls.
    order: Vec<FrameValue>,
    slots: Vec<Slot>,
    /// The extraction running now. The window never waits on it.
    pending: Option<Receiver<Extraction>>,
    loading_frame: Option<u64>,
    problem: Option<String>,
    save_status: Option<String>,
    commands: Vec<String>,
}

impl Default for FrameViewer {
    fn default() -> Self {
        Self {
            open: false,
            frame: None,
            encode: None,
            metric: None,
            gain: 0,
            actual_size: false,
            wipe: false,
            wipe_position: 0.5,
            focused: Focus::default(),
            order: Vec::new(),
            slots: Vec::new(),
            pending: None,
            loading_frame: None,
            problem: None,
            save_status: None,
            commands: Vec::new(),
        }
    }
}

/// What the viewer asks the window to do.
#[derive(Debug, PartialEq)]
pub enum Ask {
    Nothing,
    /// Extract this frame at this gain.
    Extract(u64, u32),
    /// Copy this cached still to the export folder. `label` and `gain` name the file;
    /// `gain` is only meaningful when `label` is "difference".
    Save {
        path: PathBuf,
        label: &'static str,
        frame: u64,
        gain: u32,
    },
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
    pub fn poll(&mut self, ctx: &egui::Context) {
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
                    load(ctx, "reference", &extracted.reference),
                    load(ctx, "encode", &extracted.encode),
                    load(ctx, "difference", &extracted.difference),
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

    /// Drops the difference image and keeps the two stills.
    ///
    /// Only the difference carries the gain, so a new gain redraws that one alone. The
    /// two stills are already on disk and stay on screen while it is made.
    fn drop_difference(&mut self) {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.label == "difference")
        {
            slot.texture = None;
        }
    }

    /// Records what a save action did, so it shows next to the controls.
    pub fn report_save(&mut self, result: Result<PathBuf, String>) {
        self.save_status = Some(match result {
            Ok(path) => format!("Saved to {}.", path.display()),
            Err(error) => format!("The save did not finish: {error}"),
        });
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

fn load(ctx: &egui::Context, label: &'static str, path: &Path) -> Slot {
    let texture = vqtt_run::read_png(path).ok().map(|image| {
        let size = [image.width as usize, image.height as usize];
        ctx.load_texture(
            format!("frame-{label}"),
            ColorImage::from_rgba_unmultiplied(size, &image.pixels),
            egui::TextureOptions::LINEAR,
        )
    });
    Slot {
        label,
        path: path.to_path_buf(),
        texture,
    }
}

/// The size that every still is drawn at.
///
/// The tool scales each encode up to the reference, so the reference is the measurement
/// resolution and the shape the viewer holds before it has fetched a frame.
pub fn reference_size(files: &vqtt_core::set::ComparisonSet) -> Option<egui::Vec2> {
    files
        .reference()
        .map(|file| egui::vec2(file.info.width as f32, file.info.height as f32))
}

/// Draws the viewer and reports what the reader asked for.
/// `frame` is the size of the reference, which every still is drawn at.
pub fn show(
    ui: &mut Ui,
    tokens: &Tokens,
    viewer: &mut FrameViewer,
    note: Option<&str>,
    frame: Option<egui::Vec2>,
) -> Ask {
    let mut ask = Ask::Nothing;
    card(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        header(ui, tokens, viewer);
        if !viewer.open {
            return;
        }

        ui.add_space(8.0);
        images(ui, tokens, viewer, frame);
        ui.add_space(8.0);
        ask = controls(ui, tokens, viewer);

        if let Some(problem) = &viewer.problem {
            ui.add_space(6.0);
            ui.label(sans(problem, 11.5, tokens.warn));
        }
        if let Some(status) = &viewer.save_status {
            ui.add_space(6.0);
            ui.label(sans(status, 11.0, tokens.text_secondary));
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
            crate::widgets::caret_icon(ui, 11.0, tokens.text_muted, viewer.open);
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
fn images(ui: &mut Ui, tokens: &Tokens, viewer: &mut FrameViewer, frame: Option<egui::Vec2>) {
    egui::Frame::default()
        .fill(FRAME_VIEWER_GRAY)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let available = ui.available_width();
            let slot_width = ((available - 20.0) / 3.0).max(60.0);
            let gap = ui.spacing().item_spacing.x;
            let one = slot_size(frame, slot_width, viewer.actual_size);
            let wide = slot_size(frame, slot_width * 2.0, viewer.actual_size);
            let total = if viewer.wipe {
                wide.x + gap + one.x
            } else {
                one.x * 3.0 + gap * 2.0
            };
            // The row sits in the middle of its surround rather than against the left
            // edge, so a frame that is narrower than its box is not lopsided.
            let lead = ((available - total) / 2.0).max(0.0);

            // At 1:1 a 2160p still is far wider than the window, so the row pans
            // sideways rather than pushing the rest of the page out of shape.
            egui::ScrollArea::horizontal()
                .id_salt("frame-viewer-images")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(lead);
                        if viewer.slots.is_empty() {
                            for label in ["reference", "encode", "difference"] {
                                placeholder(ui, label, one, viewer.is_loading());
                            }
                            return;
                        }
                        if viewer.wipe {
                            wipe_pane(ui, tokens, viewer, wide, one);
                        } else {
                            for index in 0..viewer.slots.len() {
                                tile(ui, tokens, viewer, index, one);
                            }
                        }
                    });
                });
        });
}

/// One image, click to focus it for "save PNG", with a border on the one that is.
///
/// No tint, no filter and no opacity. The theme never reaches inside these three
/// frames, so the focus border is the only chrome allowed to sit on top of one.
fn tile(ui: &mut Ui, tokens: &Tokens, viewer: &mut FrameViewer, index: usize, size: egui::Vec2) {
    let label = viewer.slots[index].label;
    let Some(texture) = viewer.slots[index].texture.as_ref() else {
        placeholder(ui, label, size, viewer.is_loading());
        return;
    };
    let response = ui.add(
        egui::Image::new(texture)
            .fit_to_exact_size(size)
            .sense(egui::Sense::click()),
    );

    let this_focus = focus_of(label);
    if response.clicked()
        && let Some(focus) = this_focus
    {
        viewer.focused = focus;
    }
    if this_focus == Some(viewer.focused) {
        ui.painter().rect_stroke(
            response.rect,
            0.0,
            egui::Stroke::new(2.0, tokens.accent),
            egui::StrokeKind::Inside,
        );
    }
}

/// Where the wipe line lands after a horizontal drag, clamped to the pane.
fn wipe_after_drag(position: f32, delta_x: f32, pane_width: f32) -> f32 {
    (position + delta_x / pane_width.max(1.0)).clamp(0.0, 1.0)
}

/// What "save PNG" asks for, given the tile that is focused right now.
///
/// `None` when there is nothing to save yet, so the caller can no-op a stray click.
fn save_ask(viewer: &FrameViewer) -> Option<Ask> {
    let frame = viewer.frame?;
    let slot = viewer
        .slots
        .iter()
        .find(|slot| slot.label == viewer.focused.label())?;
    Some(Ask::Save {
        path: slot.path.clone(),
        label: viewer.focused.label(),
        frame,
        gain: viewer.gain,
    })
}

fn focus_of(label: &str) -> Option<Focus> {
    match label {
        "reference" => Some(Focus::Reference),
        "encode" => Some(Focus::Encode),
        "difference" => Some(Focus::Difference),
        _ => None,
    }
}

/// The reference and the encode sharing one pane behind a draggable line.
///
/// Both stills share one pixel size, because the encode chain always scales to the
/// reference before the difference is drawn, so the line needs no coordinate mapping.
fn wipe_pane(
    ui: &mut Ui,
    tokens: &Tokens,
    viewer: &mut FrameViewer,
    wide: egui::Vec2,
    one: egui::Vec2,
) {
    // Owned copies, not borrows, so `viewer` is free to mutate while this reads.
    let reference = viewer
        .slots
        .iter()
        .find(|slot| slot.label == "reference")
        .and_then(|slot| slot.texture.as_ref())
        .map(|texture| texture.id());
    let encode = viewer
        .slots
        .iter()
        .find(|slot| slot.label == "encode")
        .and_then(|slot| slot.texture.as_ref())
        .map(|texture| texture.id());

    match (reference, encode) {
        (Some(reference_id), Some(encode_id)) => {
            let (rect, pane) = ui.allocate_exact_size(wide, egui::Sense::click());
            let painter = ui.painter().clone();
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(reference_id, rect, uv, egui::Color32::WHITE);

            let divider_x = rect.left() + rect.width() * viewer.wipe_position;
            let right =
                egui::Rect::from_min_max(egui::pos2(divider_x, rect.top()), rect.right_bottom());
            painter
                .with_clip_rect(right)
                .image(encode_id, rect, uv, egui::Color32::WHITE);
            painter.line_segment(
                [
                    egui::pos2(divider_x, rect.top()),
                    egui::pos2(divider_x, rect.bottom()),
                ],
                egui::Stroke::new(2.0, tokens.accent),
            );
            // A grip on the line, so the one part that takes a drag looks like it.
            let grip = egui::Rect::from_center_size(
                egui::pos2(divider_x, rect.center().y),
                egui::vec2(WIPE_GRIP, WIPE_GRIP * 6.0),
            );
            painter.rect_filled(grip, 2.0, tokens.accent);

            // The line takes the drag, not the whole pane. Dragging the picture itself
            // moved the line from anywhere, which read as the image being dragged.
            let band = egui::Rect::from_min_max(
                egui::pos2(divider_x - WIPE_GRAB / 2.0, rect.top()),
                egui::pos2(divider_x + WIPE_GRAB / 2.0, rect.bottom()),
            );
            let line = ui
                .interact(
                    band,
                    ui.id().with(("wipe-line", viewer.frame)),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);

            if line.dragged() {
                viewer.wipe_position =
                    wipe_after_drag(viewer.wipe_position, line.drag_delta().x, rect.width());
            } else if pane.clicked()
                && let Some(pointer) = pane.interact_pointer_pos()
            {
                viewer.focused = if pointer.x < divider_x {
                    Focus::Reference
                } else {
                    Focus::Encode
                };
            }
        }
        _ => placeholder(ui, "reference", wide, viewer.is_loading()),
    }

    // `poll()` always fills all three slots together, so index 2 is the difference
    // whenever `wipe_pane` runs at all (the caller already handled the empty case).
    tile(ui, tokens, viewer, 2, one);
}

fn fit(size: egui::Vec2, slot_width: f32, actual: bool) -> egui::Vec2 {
    if actual {
        return size;
    }
    let scale = (slot_width / size.x).min(SLOT_HEIGHT / size.y).min(1.0);
    size * scale
}

/// The box one image sits in, taken from the shape of the reference frame.
///
/// Every still is the size of the reference, because the encode is scaled to it before
/// the difference is drawn. Reading the shape from the file rather than from a loaded
/// texture is what lets the viewer draw the frames in the right shape before it has
/// any, and it keeps the box the same size once they arrive.
fn slot_size(frame: Option<egui::Vec2>, slot_width: f32, actual: bool) -> egui::Vec2 {
    match frame {
        Some(frame) if frame.x > 0.0 && frame.y > 0.0 => fit(frame, slot_width, actual),
        _ => egui::vec2(slot_width, SLOT_HEIGHT),
    }
}

/// The gray box that stands in for an image, with a spinner while one is being made.
///
/// The spinner turns rather than sits, because a still of a long-GOP file can take a
/// few seconds and a caption that does not move reads as a hang.
fn placeholder(ui: &mut Ui, label: &str, size: egui::Vec2, loading: bool) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let ink = egui::Color32::from_rgb(0x33, 0x33, 0x33);
    let font = egui::FontId::new(
        11.0,
        egui::FontFamily::Name(crate::fonts::MONO_FAMILY.into()),
    );

    // The outline stands where the picture will be, so the shape of the frame is on
    // screen before the frame is, and nothing moves when it arrives.
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, ink),
        egui::StrokeKind::Inside,
    );

    if !loading {
        ui.painter()
            .text(rect.center(), egui::Align2::CENTER_CENTER, label, font, ink);
        return;
    }

    // The spinner is painted, not added as a widget. A widget placed at its own rect
    // moves the cursor of the row it sits in, which pulled the next box back over this
    // one and left the three overlapping.
    spinner(
        ui.painter(),
        rect.center() - egui::vec2(0.0, SPINNER * 0.5),
        SPINNER * 0.5,
        ink,
        ui.input(|input| input.time),
    );
    ui.ctx().request_repaint();
    ui.painter().text(
        rect.center() + egui::vec2(0.0, SPINNER * 0.8),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        ink,
    );
}

/// An arc that turns with the clock, for a frame that is still being made.
fn spinner(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    color: egui::Color32,
    time: f64,
) {
    const STEPS: usize = 24;
    let start = (time * 2.4) as f32;
    let sweep = std::f32::consts::PI * 1.3;
    let points: Vec<egui::Pos2> = (0..=STEPS)
        .map(|step| {
            let angle = start + sweep * step as f32 / STEPS as f32;
            center + egui::vec2(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, color)));
}

fn controls(ui: &mut Ui, tokens: &Tokens, viewer: &mut FrameViewer) -> Ask {
    let mut ask = Ask::Nothing;
    let before = (viewer.frame, viewer.gain);
    let mut save_clicked = false;

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
        .on_hover_text(
            "Draws each image at its own pixel size. A fitted image hides the small \
faults you are looking for.",
        );
        ui.checkbox(&mut viewer.wipe, sans("wipe", 11.0, tokens.text_secondary))
            .on_hover_text(
                "Puts the reference and the encode in one pane. Drag the line to move \
the split.",
            );
        if ui
            .add(egui::Button::new(sans("save PNG", 11.0, tokens.text)))
            .on_hover_text("Saves the tile in view to the export folder.")
            .clicked()
        {
            save_clicked = true;
        }

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
        if before.0 == viewer.frame {
            viewer.drop_difference();
        } else {
            viewer.slots.clear();
        }
        ask = Ask::Extract(frame, viewer.gain);
    } else if save_clicked && let Some(save) = save_ask(viewer) {
        ask = save;
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
            path: PathBuf::from("f0_ref.png"),
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

    #[test]
    fn focus_of_maps_each_slot_label_and_nothing_else() {
        assert_eq!(focus_of("reference"), Some(Focus::Reference));
        assert_eq!(focus_of("encode"), Some(Focus::Encode));
        assert_eq!(focus_of("difference"), Some(Focus::Difference));
        assert_eq!(focus_of("nope"), None);
    }

    #[test]
    fn a_new_viewer_focuses_the_encode_first() {
        assert_eq!(FrameViewer::default().focused, Focus::Encode);
    }

    #[test]
    fn a_wipe_drag_never_leaves_the_pane() {
        assert_eq!(wipe_after_drag(0.5, 1000.0, 100.0), 1.0);
        assert_eq!(wipe_after_drag(0.5, -1000.0, 100.0), 0.0);
        assert!((wipe_after_drag(0.5, 10.0, 100.0) - 0.6).abs() < 1e-6);
    }

    fn slot(label: &'static str, path: &str) -> Slot {
        Slot {
            label,
            path: PathBuf::from(path),
            texture: None,
        }
    }

    #[test]
    fn save_ask_names_whichever_tile_is_focused() {
        let mut viewer = viewer_on(&[40.0, 30.0], Direction::HigherIsBetter, 0);
        viewer.gain = 8;
        viewer.slots = vec![
            slot("reference", "f0_ref.png"),
            slot("encode", "f0_enc.png"),
            slot("difference", "f0_diff_x8.png"),
        ];

        viewer.focused = Focus::Reference;
        assert_eq!(
            save_ask(&viewer),
            Some(Ask::Save {
                path: PathBuf::from("f0_ref.png"),
                label: "reference",
                frame: 0,
                gain: 8,
            })
        );

        viewer.focused = Focus::Difference;
        assert_eq!(
            save_ask(&viewer),
            Some(Ask::Save {
                path: PathBuf::from("f0_diff_x8.png"),
                label: "difference",
                frame: 0,
                gain: 8,
            })
        );
    }

    /// The empty viewer and the loaded one read the same function, so the outline a
    /// reader sees before the frames arrive is the box the frames land in.
    #[test]
    fn the_empty_box_keeps_the_shape_of_the_reference_frame() {
        let wide = slot_size(Some(egui::vec2(1920.0, 1080.0)), 300.0, false);
        assert!(
            (wide.x / wide.y - 16.0 / 9.0).abs() < 0.01,
            "a 16:9 reference gives a 16:9 box, not {wide:?}"
        );

        let tall = slot_size(Some(egui::vec2(1080.0, 1920.0)), 300.0, false);
        assert!(tall.y > tall.x, "a portrait reference gives a portrait box");
        assert!(tall.y <= SLOT_HEIGHT, "the box stays inside the row height");
    }

    #[test]
    fn a_reference_of_no_size_falls_back_to_the_plain_box() {
        assert_eq!(
            slot_size(None, 300.0, false),
            egui::vec2(300.0, SLOT_HEIGHT)
        );
        assert_eq!(
            slot_size(Some(egui::vec2(0.0, 0.0)), 300.0, false),
            egui::vec2(300.0, SLOT_HEIGHT),
            "a zero size never divides by zero"
        );
    }

    #[test]
    fn one_to_one_draws_the_reference_at_its_own_size() {
        let frame = egui::vec2(1920.0, 1080.0);
        assert_eq!(slot_size(Some(frame), 300.0, true), frame);
    }

    #[test]
    fn save_ask_is_nothing_before_a_frame_has_loaded() {
        let viewer = FrameViewer {
            slots: vec![slot("encode", "f0_enc.png")],
            ..FrameViewer::default()
        };

        assert_eq!(save_ask(&viewer), None);
    }
}
