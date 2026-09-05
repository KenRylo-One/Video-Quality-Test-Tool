//! The drawing description for one plot.
//!
//! This module decides everything about a plot and draws none of it. It takes the
//! per-frame series and a box size in pixels, and returns every shape already placed
//! inside that box. One description, then a renderer for the screen and, later, one for
//! PNG and one for SVG. The screen and the exported file cannot drift apart, because
//! there is only one set of decisions.
//!
//! Positions are pixels relative to the top left of the box. That is what makes the
//! label collision pass and the decimation testable with no window.

use crate::media::Rational;
use crate::metric::{Direction, MetricId};
use crate::palette::{DASH_PATTERNS, SERIES_SLOTS, SeriesColor, Theme, series_color};
use crate::set::FileId;

/// The padding around the plot area.
const PAD_LEFT: f32 = 50.0;
const PAD_TOP: f32 = 16.0;
const PAD_BOTTOM: f32 = 38.0;
/// The right padding with no direct labels, and with them.
const PAD_RIGHT_PLAIN: f32 = 20.0;
const PAD_RIGHT_LABELLED: f32 = 150.0;

/// Direct labels appear above this many encodes, where hue alone stops separating them.
const DIRECT_LABEL_ABOVE: usize = 3;

/// The smallest gap between two direct labels.
const LABEL_GAP: f32 = 13.0;

/// A direct label longer than this is cut short.
const LABEL_CHARS: usize = 22;

/// The share of the vertical axis that the shaded bad end covers.
const BAD_END_SHARE: f32 = 0.15;

/// The height of one small multiple, drawn above eight encodes.
pub const SMALL_MULTIPLE_HEIGHT: f32 = 110.0;

/// About this many vertical gridlines.
const X_TICK_TARGET: f32 = 6.0;

/// A rectangle in box pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// One horizontal gridline, with the value it marks.
#[derive(Debug, Clone, PartialEq)]
pub struct YTick {
    pub y: f32,
    pub value: f32,
    pub text: String,
}

/// One vertical gridline. It carries a frame number and a timecode, on two rows.
#[derive(Debug, Clone, PartialEq)]
pub struct XTick {
    pub x: f32,
    pub frame: u64,
    pub frame_text: String,
    pub time_text: String,
}

/// One pixel column of a decimated series.
///
/// The band between `min` and `max` is what keeps a single bad frame visible when a
/// whole hour is on screen. A mean-only line hides it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    pub x: f32,
    pub min: f32,
    pub max: f32,
    pub mean: f32,
    /// The frame in this column with the worst value for the metric's direction, so a
    /// click on a spike opens the spike and not its neighbour.
    pub worst_frame: u64,
}

/// The name of one line, drawn past the right edge at the height the line ends on.
#[derive(Debug, Clone, PartialEq)]
pub struct EndLabel {
    pub x: f32,
    pub y: f32,
    pub text: String,
}

/// One encode's line.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesShape {
    pub file: FileId,
    pub color: SeriesColor,
    /// Empty unless the high contrast switch is on.
    pub dash: &'static [f32],
    pub columns: Vec<Column>,
    pub end_label: Option<EndLabel>,
}

/// One encode's whole-clip number, for a metric that gives no per-frame series.
#[derive(Debug, Clone, PartialEq)]
pub struct WholeClipValue {
    pub file: FileId,
    pub color: SeriesColor,
    pub name: String,
    pub value: f32,
}

/// What the plot holds.
///
/// ColorVideoVDP reports one score for the whole clip, because only the last row of
/// FFVship's cumulative output is real. One value is not a line, so it never becomes
/// one.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Lines(Vec<SeriesShape>),
    WholeClip(Vec<WholeClipValue>),
    /// Not one finite value anywhere. The renderer says so in words.
    NothingToDraw,
}

/// Everything one plot draws, already placed.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub metric: MetricId,
    /// Set on a small multiple, where the title names the encode instead of the metric.
    pub title: Option<String>,
    pub size: (f32, f32),
    pub plot: Rect,
    pub bad_tint: Rect,
    pub y_lo: f32,
    pub y_hi: f32,
    pub y_ticks: Vec<YTick>,
    pub x_ticks: Vec<XTick>,
    pub body: Body,
    pub crosshair_x: Option<f32>,
    /// True when the window was fitted to the data because nothing fell inside the
    /// registry's window. An identity comparison does this.
    pub window_was_fitted: bool,
}

/// One encode's series, as the caller holds it.
pub struct SeriesInput<'a> {
    pub file: FileId,
    /// The palette slot, from file identity. `None` above eight encodes.
    pub slot: Option<usize>,
    pub name: &'a str,
    pub values: &'a [f32],
}

/// Everything the scene builder needs.
pub struct PlotRequest<'a> {
    pub metric: MetricId,
    pub series: &'a [SeriesInput<'a>],
    pub theme: Theme,
    pub high_contrast: bool,
    /// The visible share of the series, from 0.0 to 1.0.
    pub x_domain: (f32, f32),
    /// The absolute frame number that sample zero holds.
    pub first_frame: u64,
    pub frame_rate: Rational,
    pub size: (f32, f32),
    pub hover_frame: Option<u64>,
}

/// Builds the plot.
///
/// Returns one scene, or one short scene for each encode above eight slots, where the
/// palette has run out and colour can no longer carry identity.
pub fn build_scenes(request: &PlotRequest) -> Vec<Scene> {
    if request.series.len() > SERIES_SLOTS {
        return request
            .series
            .iter()
            .map(|input| {
                build_one(
                    request,
                    std::slice::from_ref(input),
                    Some(input.name.to_string()),
                    (request.size.0, SMALL_MULTIPLE_HEIGHT),
                )
            })
            .collect();
    }
    vec![build_one(request, request.series, None, request.size)]
}

fn build_one(
    request: &PlotRequest,
    series: &[SeriesInput],
    title: Option<String>,
    size: (f32, f32),
) -> Scene {
    let def = request.metric.def();
    let labelled = series.len() > DIRECT_LABEL_ABOVE && title.is_none();
    let pad_right = if labelled {
        PAD_RIGHT_LABELLED
    } else {
        PAD_RIGHT_PLAIN
    };
    let plot = Rect {
        x: PAD_LEFT,
        y: PAD_TOP,
        w: (size.0 - PAD_LEFT - pad_right).max(1.0),
        h: (size.1 - PAD_TOP - PAD_BOTTOM).max(1.0),
    };

    let (y_lo, y_hi, window_was_fitted) = window_for(series, def.plot_lo, def.plot_hi);
    let bad_tint = bad_end_rect(&plot, def.direction);

    let sample_count = series
        .iter()
        .map(|input| input.values.len())
        .max()
        .unwrap_or(0);
    if sample_count == 0 {
        return empty_scene(request, title, size, plot, bad_tint, y_lo, y_hi);
    }

    if sample_count == 1 {
        let values = whole_clip_values(request, series);
        return Scene {
            metric: request.metric,
            title,
            size,
            plot,
            bad_tint,
            y_lo,
            y_hi,
            y_ticks: Vec::new(),
            x_ticks: Vec::new(),
            body: Body::WholeClip(values),
            crosshair_x: None,
            window_was_fitted,
        };
    }

    let (first_sample, last_sample) = visible_samples(request.x_domain, sample_count);
    let shapes = line_shapes(
        request,
        series,
        &plot,
        y_lo,
        y_hi,
        first_sample,
        last_sample,
    );
    let body = if shapes.is_empty() {
        Body::NothingToDraw
    } else {
        Body::Lines(place_labels(shapes, &plot, labelled))
    };

    Scene {
        metric: request.metric,
        title,
        size,
        plot,
        bad_tint,
        y_lo,
        y_hi,
        y_ticks: y_ticks(&plot, y_lo, y_hi),
        x_ticks: x_ticks(request, &plot, first_sample, last_sample),
        body,
        crosshair_x: crosshair(request, &plot, first_sample, last_sample),
        window_was_fitted,
    }
}

fn empty_scene(
    request: &PlotRequest,
    title: Option<String>,
    size: (f32, f32),
    plot: Rect,
    bad_tint: Rect,
    y_lo: f32,
    y_hi: f32,
) -> Scene {
    Scene {
        metric: request.metric,
        title,
        size,
        plot,
        bad_tint,
        y_lo,
        y_hi,
        y_ticks: Vec::new(),
        x_ticks: Vec::new(),
        body: Body::NothingToDraw,
        crosshair_x: None,
        window_was_fitted: false,
    }
}

/// Chooses the vertical window.
///
/// The registry window is a legibility choice, so real data can sit entirely outside it.
/// A file measured against itself gives infinite PSNR, and that identity test runs in
/// every milestone. When no finite value falls inside the registry window, fit the
/// window to the data instead, so the plot is never blank for a reason the reader
/// cannot see.
fn window_for(series: &[SeriesInput], lo: f32, hi: f32) -> (f32, f32, bool) {
    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    let mut inside = false;
    for input in series {
        for value in input.values.iter().copied().filter(|v| v.is_finite()) {
            lowest = lowest.min(value);
            highest = highest.max(value);
            if value >= lo && value <= hi {
                inside = true;
            }
        }
    }
    if inside || lowest > highest {
        return (lo, hi, false);
    }
    if lowest == highest {
        let pad = (lowest.abs() * 0.05).max(1.0);
        return (lowest - pad, highest + pad, true);
    }
    let pad = (highest - lowest) * 0.1;
    (lowest - pad, highest + pad, true)
}

/// The shaded bad end. The tool never flips an axis, so the tint moves instead.
fn bad_end_rect(plot: &Rect, direction: Direction) -> Rect {
    let depth = plot.h * BAD_END_SHARE;
    match direction {
        Direction::LowerIsBetter => Rect {
            x: plot.x,
            y: plot.y,
            w: plot.w,
            h: depth,
        },
        _ => Rect {
            x: plot.x,
            y: plot.bottom() - depth,
            w: plot.w,
            h: depth,
        },
    }
}

fn visible_samples(domain: (f32, f32), sample_count: usize) -> (usize, usize) {
    let last = sample_count.saturating_sub(1);
    let sample_at = |share: f32| ((share * last as f32).floor().max(0.0) as usize).min(last);
    let first_sample = sample_at(domain.0).min(last.saturating_sub(1));
    (first_sample, sample_at(domain.1).max(first_sample + 1))
}

fn colour_of(request: &PlotRequest, input: &SeriesInput) -> SeriesColor {
    // Above eight encodes the palette has run out. Every small multiple then draws in
    // one colour, and the title carries the identity instead. A cycled palette would
    // give two files the same colour, which is worse than one colour for all.
    let slot = input.slot.unwrap_or(0);
    series_color(request.theme, slot)
        .or_else(|| series_color(request.theme, 0))
        .expect("slot zero is always a colour")
}

fn whole_clip_values(request: &PlotRequest, series: &[SeriesInput]) -> Vec<WholeClipValue> {
    series
        .iter()
        .filter_map(|input| {
            let value = input.values.first().copied()?;
            Some(WholeClipValue {
                file: input.file,
                color: colour_of(request, input),
                name: input.name.to_string(),
                value,
            })
        })
        .collect()
}

fn line_shapes(
    request: &PlotRequest,
    series: &[SeriesInput],
    plot: &Rect,
    y_lo: f32,
    y_hi: f32,
    first_sample: usize,
    last_sample: usize,
) -> Vec<SeriesShape> {
    let columns_across = plot.w.floor().max(1.0) as usize;
    series
        .iter()
        .enumerate()
        .filter_map(|(index, input)| {
            let columns = decimate(
                input.values,
                first_sample,
                last_sample,
                columns_across,
                plot,
                y_lo,
                y_hi,
                request.first_frame,
                request.metric.def().direction,
            );
            if columns.is_empty() {
                return None;
            }
            Some(SeriesShape {
                file: input.file,
                color: colour_of(request, input),
                dash: if request.high_contrast {
                    DASH_PATTERNS[input.slot.unwrap_or(index) % SERIES_SLOTS]
                } else {
                    &[]
                },
                columns,
                end_label: Some(EndLabel {
                    x: plot.right() + 8.0,
                    y: 0.0,
                    text: shorten(input.name),
                }),
            })
        })
        .collect()
}

/// Reduces the visible samples to one column for each pixel.
///
/// Each column keeps the lowest, the highest and the mean of the frames that fall in
/// it. The band between the lowest and the highest is the reason a single bad frame in
/// two hundred thousand is still visible at full zoom out.
#[allow(clippy::too_many_arguments)]
fn decimate(
    values: &[f32],
    first_sample: usize,
    last_sample: usize,
    columns_across: usize,
    plot: &Rect,
    y_lo: f32,
    y_hi: f32,
    first_frame: u64,
    direction: Direction,
) -> Vec<Column> {
    let span = (last_sample - first_sample) as f32;
    let mut columns = Vec::with_capacity(columns_across);
    for column in 0..columns_across {
        let from = first_sample as f32 + span * (column as f32 / columns_across as f32);
        let to = first_sample as f32 + span * ((column + 1) as f32 / columns_across as f32);
        let start = from.floor() as usize;
        let end = (to.floor() as usize).max(start + 1);

        let mut lowest = f32::INFINITY;
        let mut highest = f32::NEG_INFINITY;
        let mut total = 0.0f32;
        let mut counted = 0u32;
        let mut worst = f32::NAN;
        let mut worst_sample = start;
        for (offset, value) in values
            .iter()
            .take(end)
            .skip(start)
            .copied()
            .enumerate()
            .filter(|(_, value)| value.is_finite())
        {
            lowest = lowest.min(value);
            highest = highest.max(value);
            total += value;
            counted += 1;
            if !worst.is_finite() || is_worse(value, worst, direction) {
                worst = value;
                worst_sample = start + offset;
            }
        }
        if counted == 0 {
            continue;
        }
        columns.push(Column {
            x: plot.x + column as f32,
            min: y_for(lowest, plot, y_lo, y_hi),
            max: y_for(highest, plot, y_lo, y_hi),
            mean: y_for(total / counted as f32, plot, y_lo, y_hi),
            worst_frame: first_frame + worst_sample as u64,
        });
    }
    columns
}

/// Whether the first value is the worse of the two for this metric.
///
/// For CAMBI the worse value is the higher one. Getting this backwards sends the frame
/// viewer to the best frame in the column, and nothing on screen would say so.
pub fn is_worse(value: f32, than: f32, direction: Direction) -> bool {
    match direction {
        Direction::LowerIsBetter => value > than,
        _ => value < than,
    }
}

fn y_for(value: f32, plot: &Rect, y_lo: f32, y_hi: f32) -> f32 {
    let span = y_hi - y_lo;
    let share = if span == 0.0 {
        0.5
    } else {
        (value - y_lo) / span
    };
    plot.y + plot.h * (1.0 - share.clamp(0.0, 1.0))
}

/// Puts each direct label at the height its line ends on, then pushes them apart.
///
/// Two lines that finish close together would print one label over the other. The pass
/// spreads them downward to a minimum gap, then lifts the whole run back up when it
/// runs past the bottom of the plot.
fn place_labels(mut shapes: Vec<SeriesShape>, plot: &Rect, labelled: bool) -> Vec<SeriesShape> {
    if !labelled {
        for shape in &mut shapes {
            shape.end_label = None;
        }
        return shapes;
    }

    let mut order: Vec<usize> = (0..shapes.len()).collect();
    let end_of = |shape: &SeriesShape| shape.columns.last().map_or(plot.y, |column| column.mean);
    order.sort_by(|left, right| end_of(&shapes[*left]).total_cmp(&end_of(&shapes[*right])));

    let mut placed: Vec<f32> = order.iter().map(|index| end_of(&shapes[*index])).collect();
    for index in 1..placed.len() {
        if placed[index] - placed[index - 1] < LABEL_GAP {
            placed[index] = placed[index - 1] + LABEL_GAP;
        }
    }
    for index in (1..placed.len()).rev() {
        if placed[index] > plot.bottom() {
            placed[index] = plot.bottom();
            if placed[index - 1] > placed[index] - LABEL_GAP {
                placed[index - 1] = placed[index] - LABEL_GAP;
            }
        }
    }

    for (slot, shape_index) in order.iter().enumerate() {
        if let Some(label) = &mut shapes[*shape_index].end_label {
            label.y = placed[slot];
        }
    }
    shapes
}

fn shorten(name: &str) -> String {
    if name.chars().count() <= LABEL_CHARS {
        return name.to_string();
    }
    let kept: String = name.chars().take(LABEL_CHARS - 2).collect();
    format!("{kept}…")
}

fn y_ticks(plot: &Rect, y_lo: f32, y_hi: f32) -> Vec<YTick> {
    let decimals = if (y_hi - y_lo) <= 1.5 { 2 } else { 0 };
    [0.0f32, 0.5, 1.0]
        .into_iter()
        .map(|share| {
            let value = y_lo + (y_hi - y_lo) * share;
            YTick {
                y: y_for(value, plot, y_lo, y_hi),
                value,
                text: format!("{value:.decimals$}"),
            }
        })
        .collect()
}

/// Chooses about six round frame numbers to mark.
///
/// The step is one, two, five or ten times a power of ten, so the numbers stay readable
/// at every zoom depth instead of landing on whatever the pixel arithmetic gives.
fn x_ticks(
    request: &PlotRequest,
    plot: &Rect,
    first_sample: usize,
    last_sample: usize,
) -> Vec<XTick> {
    let frame_lo = request.first_frame + first_sample as u64;
    let frame_hi = request.first_frame + last_sample as u64;
    let visible = (frame_hi - frame_lo).max(1) as f32;

    let rough = visible / X_TICK_TARGET;
    let magnitude = 10f32.powf(rough.max(1.0).log10().floor());
    let normalised = rough / magnitude;
    let nice = if normalised < 1.5 {
        1.0
    } else if normalised < 3.5 {
        2.0
    } else if normalised < 7.5 {
        5.0
    } else {
        10.0
    };
    let step = ((nice * magnitude).round() as u64).max(1);

    let mut ticks = Vec::new();
    let mut frame = frame_lo.div_ceil(step) * step;
    while frame <= frame_hi {
        let share = (frame - frame_lo) as f32 / visible;
        ticks.push(XTick {
            x: plot.x + share * plot.w,
            frame,
            frame_text: group_digits(frame),
            time_text: timecode(frame, request.frame_rate),
        });
        frame += step;
    }
    ticks
}

fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn timecode(frame: u64, frame_rate: Rational) -> String {
    if frame_rate.num == 0 {
        return String::new();
    }
    let seconds = frame as f64 * frame_rate.den as f64 / frame_rate.num as f64;
    let minutes = (seconds / 60.0).floor() as u64;
    let rest = seconds - minutes as f64 * 60.0;
    format!("{minutes}:{rest:04.1}")
}

fn crosshair(
    request: &PlotRequest,
    plot: &Rect,
    first_sample: usize,
    last_sample: usize,
) -> Option<f32> {
    let frame = request.hover_frame?;
    let frame_lo = request.first_frame + first_sample as u64;
    let frame_hi = request.first_frame + last_sample as u64;
    if frame < frame_lo || frame > frame_hi {
        return None;
    }
    let share = (frame - frame_lo) as f32 / (frame_hi - frame_lo).max(1) as f32;
    Some(plot.x + share * plot.w)
}

/// A colour with an alpha, for a renderer that does not know `egui`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    fn of(color: SeriesColor, a: u8) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
            a,
        }
    }
}

/// Every colour the plot chrome uses.
///
/// The window and the exported file take their values from here, so one theme cannot
/// drift from the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chrome {
    pub grid: Rgba,
    pub axis: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_secondary: Rgba,
    pub warn: Rgba,
    /// The card the plot sits on. An exported file paints it, since the file has no
    /// window behind it.
    pub surface: Rgba,
    pub tint: Rgba,
}

impl Chrome {
    pub const fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self {
                grid: Rgba::opaque(0x20, 0x24, 0x2c),
                axis: Rgba::opaque(0x2b, 0x30, 0x3a),
                text: Rgba::opaque(0xe7, 0xe9, 0xee),
                text_muted: Rgba::opaque(0x7c, 0x82, 0x8e),
                text_secondary: Rgba::opaque(0x9a, 0xa0, 0xac),
                warn: Rgba::opaque(0xc9, 0x88, 0x62),
                surface: Rgba::opaque(0x19, 0x1c, 0x22),
                tint: Rgba::opaque(0xe7, 0xe9, 0xee).with_alpha(TINT_ALPHA),
            },
            Theme::Light => Self {
                grid: Rgba::opaque(0xe4, 0xe2, 0xdc),
                axis: Rgba::opaque(0xd8, 0xd6, 0xd0),
                text: Rgba::opaque(0x0b, 0x0b, 0x0b),
                text_muted: Rgba::opaque(0x6b, 0x69, 0x63),
                text_secondary: Rgba::opaque(0x52, 0x51, 0x4e),
                warn: Rgba::opaque(0xa1, 0x5b, 0x2e),
                surface: Rgba::opaque(0xff, 0xff, 0xff),
                tint: Rgba::opaque(0x0b, 0x0b, 0x0b).with_alpha(TINT_ALPHA),
            },
        }
    }
}

/// Where a piece of text sits relative to the point it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    LeftTop,
    LeftCenter,
    LeftBottom,
    CenterTop,
    CenterCenter,
    RightCenter,
}

/// A surface that `draw` puts shapes on.
///
/// The window, the SVG file and a test recorder all implement this. None of them makes
/// a drawing decision, because every position and colour arrives already chosen.
pub trait Canvas {
    fn rect(&mut self, rect: Rect, fill: Rgba);
    fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: Rgba);
    fn polygon(&mut self, points: &[(f32, f32)], fill: Rgba);
    fn polyline(&mut self, points: &[(f32, f32)], width: f32, color: Rgba);
    fn text(&mut self, at: (f32, f32), align: TextAlign, text: &str, size: f32, color: Rgba);
}

const BAND_ALPHA: u8 = 42;
const TINT_ALPHA: u8 = 12;
const LINE_WIDTH: f32 = 2.0;
const HAIRLINE: f32 = 1.0;
const TICK_SIZE: f32 = 10.0;
const LABEL_SIZE: f32 = 11.0;
const WHOLE_CLIP_SIZE: f32 = 13.0;
const MESSAGE_SIZE: f32 = 12.5;

const NO_SERIES_MESSAGE: &str =
    "No finite value to draw. A file measured against itself does this.";
const WHOLE_CLIP_MESSAGE: &str =
    "One score for the whole clip. This metric has no per-frame series.";

/// Draws one scene, with its top left corner at `origin`.
///
/// This is the only walk of a `Scene` in the whole tool. The screen and the exported
/// file go through it together, which is what stops the two pictures drifting apart.
pub fn draw(scene: &Scene, chrome: &Chrome, origin: (f32, f32), canvas: &mut dyn Canvas) {
    let at = |x: f32, y: f32| (origin.0 + x, origin.1 + y);
    let plot = &scene.plot;

    canvas.rect(
        Rect {
            x: origin.0 + scene.bad_tint.x,
            y: origin.1 + scene.bad_tint.y,
            w: scene.bad_tint.w,
            h: scene.bad_tint.h,
        },
        chrome.tint,
    );

    for tick in &scene.y_ticks {
        canvas.line(
            at(plot.x, tick.y),
            at(plot.right(), tick.y),
            HAIRLINE,
            chrome.grid,
        );
        canvas.text(
            at(plot.x - 8.0, tick.y),
            TextAlign::RightCenter,
            &tick.text,
            LABEL_SIZE,
            chrome.text_muted,
        );
    }

    for tick in &scene.x_ticks {
        canvas.line(
            at(tick.x, plot.y),
            at(tick.x, plot.bottom()),
            HAIRLINE,
            chrome.grid,
        );
        canvas.text(
            at(tick.x, plot.bottom() + 4.0),
            TextAlign::CenterTop,
            &tick.frame_text,
            TICK_SIZE,
            chrome.text_muted,
        );
        canvas.text(
            at(tick.x, plot.bottom() + 17.0),
            TextAlign::CenterTop,
            &tick.time_text,
            TICK_SIZE,
            chrome.text_muted,
        );
    }

    canvas.line(
        at(plot.x, plot.y),
        at(plot.x, plot.bottom()),
        HAIRLINE,
        chrome.axis,
    );
    canvas.line(
        at(plot.x, plot.bottom()),
        at(plot.right(), plot.bottom()),
        HAIRLINE,
        chrome.axis,
    );

    let direction = scene.metric.def().direction;
    canvas.text(
        at(plot.x, plot.y - 4.0),
        TextAlign::LeftBottom,
        direction.label(),
        LABEL_SIZE,
        if direction == Direction::LowerIsBetter {
            chrome.warn
        } else {
            chrome.text_muted
        },
    );

    match &scene.body {
        Body::Lines(shapes) => {
            for shape in shapes {
                draw_band(canvas, shape, origin);
                draw_mean(canvas, shape, origin);
                if let Some(label) = &shape.end_label {
                    canvas.text(
                        at(label.x, label.y),
                        TextAlign::LeftCenter,
                        &label.text,
                        LABEL_SIZE,
                        Rgba::of(shape.color, 255),
                    );
                }
            }
        }
        Body::WholeClip(values) => {
            let mut y = plot.y + 8.0;
            for entry in values {
                canvas.text(
                    at(plot.x + 8.0, y),
                    TextAlign::LeftTop,
                    &format!("{}  {:.3}", entry.name, entry.value),
                    WHOLE_CLIP_SIZE,
                    Rgba::of(entry.color, 255),
                );
                y += 20.0;
            }
            canvas.text(
                at(plot.x + 8.0, plot.bottom() - 8.0),
                TextAlign::LeftBottom,
                WHOLE_CLIP_MESSAGE,
                LABEL_SIZE,
                chrome.text_muted,
            );
        }
        Body::NothingToDraw => {
            canvas.text(
                at(plot.x + plot.w / 2.0, plot.y + plot.h / 2.0),
                TextAlign::CenterCenter,
                NO_SERIES_MESSAGE,
                MESSAGE_SIZE,
                chrome.text_muted,
            );
        }
    }

    if let Some(x) = scene.crosshair_x {
        canvas.line(
            at(x, plot.y),
            at(x, plot.bottom()),
            HAIRLINE,
            chrome.text_secondary,
        );
    }
}

fn draw_band(canvas: &mut dyn Canvas, shape: &SeriesShape, origin: (f32, f32)) {
    if shape.columns.len() < 2 {
        return;
    }
    let mut points: Vec<(f32, f32)> = shape
        .columns
        .iter()
        .map(|column| (origin.0 + column.x, origin.1 + column.max))
        .collect();
    points.extend(
        shape
            .columns
            .iter()
            .rev()
            .map(|column| (origin.0 + column.x, origin.1 + column.min)),
    );
    canvas.polygon(&points, Rgba::of(shape.color, BAND_ALPHA));
}

fn draw_mean(canvas: &mut dyn Canvas, shape: &SeriesShape, origin: (f32, f32)) {
    let points: Vec<(f32, f32)> = shape
        .columns
        .iter()
        .map(|column| (origin.0 + column.x, origin.1 + column.mean))
        .collect();
    let color = Rgba::of(shape.color, 255);
    if shape.dash.is_empty() {
        canvas.polyline(&points, LINE_WIDTH, color);
        return;
    }
    for [from, to] in dashes(&points, shape.dash) {
        canvas.line(from, to, LINE_WIDTH, color);
    }
}

/// Cuts a polyline into the drawn parts of a dash pattern.
///
/// Neither an `egui` stroke nor a plain SVG stroke can express the multi-segment
/// patterns the palette carries, so the line is measured along its own length and split
/// here. Both renderers use this, which is why a dashed line breaks in the same places
/// on screen and in a file. The pattern alternates: the first length is drawn, the
/// second is a gap, and so on, repeating.
pub fn dashes(points: &[(f32, f32)], pattern: &[f32]) -> Vec<[(f32, f32); 2]> {
    let mut segments = Vec::new();
    if points.len() < 2 || pattern.is_empty() {
        return segments;
    }
    let mut step = 0usize;
    let mut left = pattern[0];
    let mut drawing = true;

    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let length = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
        if length <= f32::EPSILON {
            continue;
        }
        let direction = ((to.0 - from.0) / length, (to.1 - from.1) / length);
        let along = |distance: f32| {
            (
                from.0 + direction.0 * distance,
                from.1 + direction.1 * distance,
            )
        };
        let mut done = 0.0f32;

        while done < length {
            let take = left.min(length - done);
            if drawing {
                segments.push([along(done), along(done + take)]);
            }
            done += take;
            left -= take;
            if left <= f32::EPSILON {
                step = (step + 1) % pattern.len();
                left = pattern[step];
                drawing = !drawing;
            }
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: (f32, f32) = (800.0, 260.0);

    fn request<'a>(metric: MetricId, series: &'a [SeriesInput<'a>]) -> PlotRequest<'a> {
        PlotRequest {
            metric,
            series,
            theme: Theme::Dark,
            high_contrast: false,
            x_domain: (0.0, 1.0),
            first_frame: 0,
            frame_rate: Rational { num: 60, den: 1 },
            size: BOX,
            hover_frame: None,
        }
    }

    fn input<'a>(id: u64, slot: usize, name: &'a str, values: &'a [f32]) -> SeriesInput<'a> {
        SeriesInput {
            file: FileId(id),
            slot: Some(slot),
            name,
            values,
        }
    }

    fn lines(scene: &Scene) -> &[SeriesShape] {
        match &scene.body {
            Body::Lines(shapes) => shapes,
            other => panic!("expected lines, found {other:?}"),
        }
    }

    #[test]
    fn a_single_bad_frame_in_216000_survives_decimation() {
        let mut values = vec![45.0f32; 216_000];
        values[123_456] = 21.0;
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        let shapes = lines(&scenes[0]);

        let plot = scenes[0].plot;
        let deepest = shapes[0]
            .columns
            .iter()
            .map(|column| column.min)
            .fold(f32::NEG_INFINITY, f32::max);
        let bad_frame_y = y_for(21.0, &plot, scenes[0].y_lo, scenes[0].y_hi);
        assert!(
            (deepest - bad_frame_y).abs() < 0.5,
            "the band must reach the one bad frame: {deepest} against {bad_frame_y}"
        );

        let mean_low = shapes[0]
            .columns
            .iter()
            .map(|column| column.mean)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            mean_low < deepest - 5.0,
            "a mean-only line would have hidden it"
        );
    }

    #[test]
    fn a_low_is_better_metric_shades_the_top_and_never_flips_the_axis() {
        let values = [4.0f32, 5.0, 6.0, 7.0];
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::Cambi, &series));
        let scene = &scenes[0];

        assert_eq!(scene.bad_tint.y, scene.plot.y);
        assert!(scene.bad_tint.bottom() < scene.plot.bottom());
        assert!(scene.y_lo < scene.y_hi);

        let low = y_for(0.0, &scene.plot, scene.y_lo, scene.y_hi);
        let high = y_for(24.0, &scene.plot, scene.y_lo, scene.y_hi);
        assert!(high < low, "a high value must sit above a low one");
    }

    #[test]
    fn a_higher_is_better_metric_shades_the_bottom() {
        let values = [40.0f32, 41.0, 42.0, 43.0];
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        assert_eq!(scenes[0].bad_tint.bottom(), scenes[0].plot.bottom());
        assert!(scenes[0].bad_tint.y > scenes[0].plot.y);
    }

    #[test]
    fn above_three_encodes_every_line_gets_a_direct_label_and_none_overlap() {
        let values = [40.0f32, 41.0, 42.0, 43.0];
        let series = [
            input(1, 0, "one.mp4", &values),
            input(2, 1, "two.mp4", &values),
            input(3, 2, "three.mp4", &values),
            input(4, 3, "four.mp4", &values),
        ];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        let shapes = lines(&scenes[0]);

        assert!(shapes.iter().all(|shape| shape.end_label.is_some()));

        let mut heights: Vec<f32> = shapes
            .iter()
            .filter_map(|shape| shape.end_label.as_ref().map(|label| label.y))
            .collect();
        heights.sort_by(f32::total_cmp);
        for pair in heights.windows(2) {
            assert!(
                pair[1] - pair[0] >= LABEL_GAP - 0.01,
                "labels overlap: {pair:?}"
            );
        }
    }

    #[test]
    fn three_encodes_get_no_direct_label_and_a_narrow_right_margin() {
        let values = [40.0f32, 41.0, 42.0];
        let series = [
            input(1, 0, "one.mp4", &values),
            input(2, 1, "two.mp4", &values),
            input(3, 2, "three.mp4", &values),
        ];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        assert!(lines(&scenes[0]).iter().all(|s| s.end_label.is_none()));
        assert_eq!(scenes[0].plot.right(), BOX.0 - PAD_RIGHT_PLAIN);
    }

    #[test]
    fn unticking_an_encode_leaves_every_other_colour_unchanged() {
        let values = [40.0f32, 41.0, 42.0];
        let all = [
            input(1, 0, "one.mp4", &values),
            input(2, 1, "two.mp4", &values),
            input(3, 2, "three.mp4", &values),
        ];
        let before = build_scenes(&request(MetricId::PsnrY, &all));

        let fewer = [
            input(1, 0, "one.mp4", &values),
            input(3, 2, "three.mp4", &values),
        ];
        let after = build_scenes(&request(MetricId::PsnrY, &fewer));

        let colour = |scene: &Scene, file: FileId| {
            lines(scene)
                .iter()
                .find(|shape| shape.file == file)
                .map(|shape| shape.color)
                .unwrap()
        };
        assert_eq!(colour(&before[0], FileId(1)), colour(&after[0], FileId(1)));
        assert_eq!(colour(&before[0], FileId(3)), colour(&after[0], FileId(3)));
    }

    #[test]
    fn a_ninth_encode_gives_small_multiples_and_never_a_ninth_colour() {
        let values = [40.0f32, 41.0, 42.0];
        let names: Vec<String> = (0..9).map(|index| format!("encode{index}.mp4")).collect();
        let series: Vec<SeriesInput> = (0..9)
            .map(|index| SeriesInput {
                file: FileId(index as u64 + 1),
                slot: if index < SERIES_SLOTS {
                    Some(index)
                } else {
                    None
                },
                name: &names[index],
                values: &values,
            })
            .collect();

        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        assert_eq!(scenes.len(), 9);
        assert!(scenes.iter().all(|scene| scene.title.is_some()));
        assert!(
            scenes
                .iter()
                .all(|scene| scene.size.1 == SMALL_MULTIPLE_HEIGHT)
        );

        let ninth = lines(&scenes[8]);
        let first = lines(&scenes[0]);
        assert_eq!(
            ninth[0].color, first[0].color,
            "above eight the palette stops carrying identity, and the title takes over"
        );
    }

    #[test]
    fn an_all_infinite_series_still_gives_a_drawable_window() {
        let values = [f32::INFINITY; 8];
        let series = [input(1, 0, "identity.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        assert_eq!(scenes[0].body, Body::NothingToDraw);
        assert!(scenes[0].y_lo < scenes[0].y_hi);
    }

    #[test]
    fn a_series_entirely_above_the_window_fits_the_window_to_the_data() {
        let values = [95.0f32, 96.0, 97.0, 98.0];
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));

        assert!(scenes[0].window_was_fitted);
        assert!(scenes[0].y_lo < 95.0 && scenes[0].y_hi > 98.0);
        assert!(!lines(&scenes[0])[0].columns.is_empty());
    }

    #[test]
    fn a_whole_clip_metric_gives_numbers_and_never_a_line() {
        let values = [7.5f32];
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::Cvvdp, &series));
        match &scenes[0].body {
            Body::WholeClip(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].value, 7.5);
            }
            other => panic!("expected one whole clip value, found {other:?}"),
        }
        assert!(scenes[0].x_ticks.is_empty());
    }

    #[test]
    fn the_x_ticks_land_on_round_frame_numbers() {
        let values = vec![40.0f32; 1500];
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        let ticks = &scenes[0].x_ticks;

        assert!(
            ticks.len() >= 3 && ticks.len() <= 12,
            "{} ticks",
            ticks.len()
        );
        let step = ticks[1].frame - ticks[0].frame;
        assert!(
            ticks
                .windows(2)
                .all(|pair| pair[1].frame - pair[0].frame == step)
        );
        assert!(ticks.iter().all(|tick| tick.frame % step == 0));
    }

    #[test]
    fn a_frame_range_offset_moves_the_tick_numbers_and_the_timecodes() {
        let values = vec![40.0f32; 600];
        let series = [input(1, 0, "encode.mp4", &values)];
        let mut plot_request = request(MetricId::PsnrY, &series);
        plot_request.first_frame = 3000;
        let scenes = build_scenes(&plot_request);

        let first = &scenes[0].x_ticks[0];
        assert!(
            first.frame >= 3000,
            "frame {} is before the range",
            first.frame
        );
        // Sixty frames for each second, so frame 3000 is fifty seconds in, not zero.
        assert_eq!(timecode(3000, Rational { num: 60, den: 1 }), "0:50.0");
        assert_eq!(timecode(3600, Rational { num: 60, den: 1 }), "1:00.0");
        assert_eq!(
            first.time_text,
            timecode(first.frame, plot_request.frame_rate)
        );
    }

    #[test]
    fn the_high_contrast_switch_gives_each_slot_a_different_dash_pattern() {
        let values = [40.0f32, 41.0, 42.0, 43.0];
        let series = [
            input(1, 0, "one.mp4", &values),
            input(2, 1, "two.mp4", &values),
            input(3, 2, "three.mp4", &values),
            input(4, 3, "four.mp4", &values),
        ];
        let mut plot_request = request(MetricId::PsnrY, &series);
        plot_request.high_contrast = true;
        let scenes = build_scenes(&plot_request);
        let shapes = lines(&scenes[0]);

        let patterns: Vec<&[f32]> = shapes.iter().map(|shape| shape.dash).collect();
        for index in 1..patterns.len() {
            assert_ne!(patterns[index], patterns[index - 1]);
        }
    }

    #[test]
    fn the_dash_patterns_are_off_until_the_switch_is_on() {
        let values = [40.0f32, 41.0, 42.0];
        let series = [input(1, 0, "one.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        assert!(lines(&scenes[0])[0].dash.is_empty());
    }

    #[test]
    fn a_long_file_name_is_cut_short_for_the_direct_label() {
        let long = "a_very_long_encode_file_name_indeed.mp4";
        assert!(shorten(long).chars().count() <= LABEL_CHARS);
        assert!(shorten(long).ends_with('…'));
        assert_eq!(shorten("short.mp4"), "short.mp4");
    }

    #[test]
    fn the_crosshair_holds_a_frame_inside_the_zoom_and_drops_one_outside() {
        let values = vec![40.0f32; 1000];
        let series = [input(1, 0, "encode.mp4", &values)];

        let mut inside = request(MetricId::PsnrY, &series);
        inside.hover_frame = Some(500);
        assert!(build_scenes(&inside)[0].crosshair_x.is_some());

        let mut outside = request(MetricId::PsnrY, &series);
        outside.hover_frame = Some(5000);
        assert!(build_scenes(&outside)[0].crosshair_x.is_none());
    }

    #[test]
    fn zooming_in_narrows_the_frames_the_ticks_cover() {
        let values = vec![40.0f32; 2000];
        let series = [input(1, 0, "encode.mp4", &values)];

        let wide = build_scenes(&request(MetricId::PsnrY, &series));
        let mut close = request(MetricId::PsnrY, &series);
        close.x_domain = (0.4, 0.5);
        let near = build_scenes(&close);

        let span = |scene: &Scene| {
            scene.x_ticks.last().unwrap().frame - scene.x_ticks.first().unwrap().frame
        };
        assert!(span(&near[0]) < span(&wide[0]));
    }

    #[test]
    fn every_column_stays_inside_the_plot_area() {
        let values = vec![10.0f32, 90.0, 50.0, 30.0, 70.0];
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));
        let plot = scenes[0].plot;
        for column in &lines(&scenes[0])[0].columns {
            assert!(column.x >= plot.x && column.x <= plot.right());
            assert!(column.max >= plot.y - 0.01 && column.min <= plot.bottom() + 0.01);
        }
    }

    #[derive(Default)]
    struct RecordingCanvas {
        ops: Vec<String>,
    }

    impl Canvas for RecordingCanvas {
        fn rect(&mut self, rect: Rect, _fill: Rgba) {
            self.ops.push(format!("rect {:.1} {:.1}", rect.x, rect.y));
        }

        fn line(&mut self, from: (f32, f32), _to: (f32, f32), _width: f32, _color: Rgba) {
            self.ops.push(format!("line {:.1} {:.1}", from.0, from.1));
        }

        fn polygon(&mut self, points: &[(f32, f32)], _fill: Rgba) {
            self.ops.push(format!("polygon {}", points.len()));
        }

        fn polyline(&mut self, points: &[(f32, f32)], _width: f32, _color: Rgba) {
            self.ops.push(format!("polyline {}", points.len()));
        }

        fn text(
            &mut self,
            at: (f32, f32),
            _align: TextAlign,
            text: &str,
            _size: f32,
            _color: Rgba,
        ) {
            self.ops
                .push(format!("text {:.1} {:.1} {text}", at.0, at.1));
        }
    }

    fn record(scene: &Scene, origin: (f32, f32)) -> Vec<String> {
        let mut canvas = RecordingCanvas::default();
        draw(scene, &Chrome::for_theme(Theme::Dark), origin, &mut canvas);
        canvas.ops
    }

    #[test]
    fn the_walk_draws_the_tint_the_grid_the_axes_the_band_and_the_line() {
        let values: Vec<f32> = (0..400).map(|index| 30.0 + (index % 7) as f32).collect();
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));

        let ops = record(&scenes[0], (0.0, 0.0));

        assert_eq!(ops.iter().filter(|op| op.starts_with("rect")).count(), 1);
        assert_eq!(ops.iter().filter(|op| op.starts_with("polygon")).count(), 1);
        assert_eq!(
            ops.iter().filter(|op| op.starts_with("polyline")).count(),
            1
        );
        assert!(ops.iter().any(|op| op.contains("higher is better")));
        assert!(ops.iter().filter(|op| op.starts_with("line")).count() > 2);
    }

    #[test]
    fn a_scene_drawn_at_an_offset_moves_every_shape_by_that_offset() {
        let values: Vec<f32> = (0..80).map(|index| 30.0 + (index % 5) as f32).collect();
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));

        let at_origin = record(&scenes[0], (0.0, 0.0));
        let moved = record(&scenes[0], (10.0, 20.0));
        let tint = scenes[0].bad_tint;

        assert_eq!(at_origin.len(), moved.len());
        assert_eq!(at_origin[0], format!("rect {:.1} {:.1}", tint.x, tint.y));
        assert_eq!(
            moved[0],
            format!("rect {:.1} {:.1}", tint.x + 10.0, tint.y + 20.0)
        );
    }

    #[test]
    fn a_dashed_series_becomes_line_segments_and_never_one_polyline() {
        // Slot zero is solid on purpose, so a dash pattern needs any other slot.
        let values: Vec<f32> = (0..400).map(|index| 30.0 + (index % 7) as f32).collect();
        let series = [input(1, 1, "encode.mp4", &values)];
        let mut high_contrast = request(MetricId::PsnrY, &series);
        high_contrast.high_contrast = true;

        let solid = record(
            &build_scenes(&request(MetricId::PsnrY, &series))[0],
            (0.0, 0.0),
        );
        let dashed = record(&build_scenes(&high_contrast)[0], (0.0, 0.0));

        assert_eq!(
            solid.iter().filter(|op| op.starts_with("polyline")).count(),
            1
        );
        assert_eq!(
            dashed
                .iter()
                .filter(|op| op.starts_with("polyline"))
                .count(),
            0
        );
        assert!(
            dashed.iter().filter(|op| op.starts_with("line")).count()
                > solid.iter().filter(|op| op.starts_with("line")).count()
        );
    }

    /// Acceptance test 1 of milestone M6, in the part that needs no window: a column
    /// covers many frames, and the one it names is the worst of them.
    #[test]
    fn a_column_names_the_worst_frame_it_covers() {
        let mut values = vec![45.0f32; 4000];
        values[2500] = 21.0;
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::PsnrY, &series));

        let columns = &lines(&scenes[0])[0].columns;
        let holds_the_dip = columns.iter().any(|column| column.worst_frame == 2500);
        assert!(holds_the_dip, "no column names the one bad frame");
        assert!(columns.iter().all(|column| column.worst_frame < 4000));
    }

    #[test]
    fn a_low_is_better_column_names_its_highest_frame_and_a_high_is_better_one_its_lowest() {
        let mut values = vec![5.0f32; 600];
        values[300] = 20.0;
        let series = [input(1, 0, "encode.mp4", &values)];

        let low_better = build_scenes(&request(MetricId::Cambi, &series));
        assert!(
            lines(&low_better[0])[0]
                .columns
                .iter()
                .any(|column| column.worst_frame == 300)
        );

        let mut dipped = vec![45.0f32; 600];
        dipped[300] = 21.0;
        let other = [input(1, 0, "encode.mp4", &dipped)];
        let high_better = build_scenes(&request(MetricId::PsnrY, &other));
        assert!(
            lines(&high_better[0])[0]
                .columns
                .iter()
                .any(|column| column.worst_frame == 300)
        );
    }

    #[test]
    fn a_run_over_part_of_a_file_names_real_frame_numbers_in_its_columns() {
        let values = vec![45.0f32; 400];
        let series = [input(1, 0, "encode.mp4", &values)];
        let mut offset = request(MetricId::PsnrY, &series);
        offset.first_frame = 1200;

        let scenes = build_scenes(&offset);
        let columns = &lines(&scenes[0])[0].columns;

        assert!(columns.iter().all(|column| column.worst_frame >= 1200));
    }

    #[test]
    fn a_low_is_better_metric_draws_its_tint_at_the_top() {
        let values: Vec<f32> = (0..80).map(|index| (index % 12) as f32).collect();
        let series = [input(1, 0, "encode.mp4", &values)];
        let scenes = build_scenes(&request(MetricId::Cambi, &series));

        let ops = record(&scenes[0], (0.0, 0.0));

        assert_eq!(ops[0], "rect 50.0 16.0");
        assert!(ops.iter().any(|op| op.contains("lower is better")));
    }
}
