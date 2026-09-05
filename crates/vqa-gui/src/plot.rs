//! Draws a `Scene` with egui.
//!
//! This renderer decides nothing. Every position already came from
//! `vqa_core::plot`, so the screen and the exported file cannot drift apart.

use crate::theme::Tokens;
use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};
use vqa_core::metric::Direction;
use vqa_core::palette::SeriesColor;
use vqa_core::plot::{Body, Column, Scene};

/// The alpha of the minimum-to-maximum band, behind the mean line.
const BAND_ALPHA: u8 = 42;

/// The alpha of the tint over the bad end of the axis.
const TINT_ALPHA: u8 = 12;

const LINE_WIDTH: f32 = 2.0;
const TICK_SIZE: f32 = 10.0;
const LABEL_SIZE: f32 = 11.0;

fn color_of(series: SeriesColor, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(series.r, series.g, series.b, alpha)
}

fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::MONO_FAMILY.into()))
}

/// Draws one scene into `origin`, which is the top left of the scene's own box.
pub fn paint(painter: &Painter, tokens: &Tokens, scene: &Scene, origin: Pos2) {
    let at = |x: f32, y: f32| Pos2::new(origin.x + x, origin.y + y);
    let plot = &scene.plot;

    painter.rect_filled(
        Rect::from_min_size(
            at(scene.bad_tint.x, scene.bad_tint.y),
            Vec2::new(scene.bad_tint.w, scene.bad_tint.h),
        ),
        0.0,
        Color32::from_rgba_unmultiplied(
            tokens.text.r(),
            tokens.text.g(),
            tokens.text.b(),
            TINT_ALPHA,
        ),
    );

    for tick in &scene.y_ticks {
        painter.line_segment(
            [at(plot.x, tick.y), at(plot.right(), tick.y)],
            Stroke::new(1.0, tokens.plot_grid),
        );
        painter.text(
            at(plot.x - 8.0, tick.y),
            Align2::RIGHT_CENTER,
            &tick.text,
            font(LABEL_SIZE),
            tokens.text_muted,
        );
    }

    for tick in &scene.x_ticks {
        painter.line_segment(
            [at(tick.x, plot.y), at(tick.x, plot.bottom())],
            Stroke::new(1.0, tokens.plot_grid),
        );
        painter.text(
            at(tick.x, plot.bottom() + 4.0),
            Align2::CENTER_TOP,
            &tick.frame_text,
            font(TICK_SIZE),
            tokens.text_muted,
        );
        painter.text(
            at(tick.x, plot.bottom() + 17.0),
            Align2::CENTER_TOP,
            &tick.time_text,
            font(TICK_SIZE),
            tokens.text_muted,
        );
    }

    let axis = Stroke::new(1.0, tokens.border);
    painter.line_segment([at(plot.x, plot.y), at(plot.x, plot.bottom())], axis);
    painter.line_segment(
        [at(plot.x, plot.bottom()), at(plot.right(), plot.bottom())],
        axis,
    );

    let direction = scene.metric.def().direction;
    painter.text(
        at(plot.x, plot.y - 4.0),
        Align2::LEFT_BOTTOM,
        direction.label(),
        font(LABEL_SIZE),
        if direction == Direction::LowerIsBetter {
            tokens.warn
        } else {
            tokens.text_muted
        },
    );

    match &scene.body {
        Body::Lines(shapes) => {
            for shape in shapes {
                paint_band(
                    painter,
                    &shape.columns,
                    color_of(shape.color, BAND_ALPHA),
                    origin,
                );
                paint_mean(
                    painter,
                    &shape.columns,
                    color_of(shape.color, 255),
                    shape.dash,
                    origin,
                );
                if let Some(label) = &shape.end_label {
                    painter.text(
                        at(label.x, label.y),
                        Align2::LEFT_CENTER,
                        &label.text,
                        font(LABEL_SIZE),
                        color_of(shape.color, 255),
                    );
                }
            }
        }
        Body::WholeClip(values) => {
            let mut y = plot.y + 8.0;
            for entry in values {
                painter.text(
                    at(plot.x + 8.0, y),
                    Align2::LEFT_TOP,
                    format!("{}  {:.3}", entry.name, entry.value),
                    font(13.0),
                    color_of(entry.color, 255),
                );
                y += 20.0;
            }
            painter.text(
                at(plot.x + 8.0, plot.bottom() - 8.0),
                Align2::LEFT_BOTTOM,
                "One score for the whole clip. This metric has no per-frame series.",
                font(LABEL_SIZE),
                tokens.text_muted,
            );
        }
        Body::NothingToDraw => {
            painter.text(
                at(plot.x + plot.w / 2.0, plot.y + plot.h / 2.0),
                Align2::CENTER_CENTER,
                "No finite value to draw. A file measured against itself does this.",
                font(12.5),
                tokens.text_muted,
            );
        }
    }

    if let Some(x) = scene.crosshair_x {
        painter.line_segment(
            [at(x, plot.y), at(x, plot.bottom())],
            Stroke::new(1.0, tokens.text_secondary),
        );
    }
}

fn paint_band(painter: &Painter, columns: &[Column], fill: Color32, origin: Pos2) {
    if columns.len() < 2 {
        return;
    }
    let mut points: Vec<Pos2> = columns
        .iter()
        .map(|column| Pos2::new(origin.x + column.x, origin.y + column.max))
        .collect();
    points.extend(
        columns
            .iter()
            .rev()
            .map(|column| Pos2::new(origin.x + column.x, origin.y + column.min)),
    );
    painter.add(egui::Shape::convex_polygon(points, fill, Stroke::NONE));
}

fn paint_mean(painter: &Painter, columns: &[Column], color: Color32, dash: &[f32], origin: Pos2) {
    let points: Vec<Pos2> = columns
        .iter()
        .map(|column| Pos2::new(origin.x + column.x, origin.y + column.mean))
        .collect();
    let stroke = Stroke::new(LINE_WIDTH, color);
    if dash.is_empty() {
        painter.add(egui::Shape::line(points, stroke));
        return;
    }
    for [from, to] in dashes(&points, dash) {
        painter.line_segment([from, to], stroke);
    }
}

/// Cuts a polyline into the on parts of a dash pattern.
///
/// `egui` strokes have no dash support, and the palette carries eight patterns of two
/// and four lengths each, so the line is measured along its own length and split by
/// hand. The pattern alternates: the first length is drawn, the second is a gap, and
/// so on, repeating.
fn dashes(points: &[Pos2], pattern: &[f32]) -> Vec<[Pos2; 2]> {
    let mut segments = Vec::new();
    if points.len() < 2 || pattern.is_empty() {
        return segments;
    }
    let mut step = 0usize;
    let mut left = pattern[0];
    let mut drawing = true;

    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let length = from.distance(to);
        if length <= f32::EPSILON {
            continue;
        }
        let direction = (to - from) / length;
        let mut done = 0.0f32;

        while done < length {
            let take = left.min(length - done);
            if drawing {
                segments.push([from + direction * done, from + direction * (done + take)]);
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

    fn line(from: f32, to: f32) -> Vec<Pos2> {
        vec![Pos2::new(from, 0.0), Pos2::new(to, 0.0)]
    }

    #[test]
    fn a_solid_pattern_makes_no_segments() {
        assert!(dashes(&line(0.0, 100.0), &[]).is_empty());
    }

    #[test]
    fn a_dash_pattern_draws_the_on_lengths_and_skips_the_off_ones() {
        let segments = dashes(&line(0.0, 20.0), &[6.0, 4.0]);
        assert_eq!(segments.len(), 2);
        assert!((segments[0][0].x - 0.0).abs() < 0.01);
        assert!((segments[0][1].x - 6.0).abs() < 0.01);
        assert!((segments[1][0].x - 10.0).abs() < 0.01);
        assert!((segments[1][1].x - 16.0).abs() < 0.01);
    }

    #[test]
    fn a_four_length_pattern_keeps_its_order_across_a_corner() {
        let points = vec![
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 10.0),
        ];
        let segments = dashes(&points, &[8.0, 3.0, 2.0, 3.0]);
        assert!(!segments.is_empty());
        let drawn: f32 = segments.iter().map(|pair| pair[0].distance(pair[1])).sum();
        assert!(drawn < 20.0, "a dashed line is shorter than a solid one");
        assert!(drawn > 0.0);
    }

    #[test]
    fn a_zero_length_line_gives_no_segment_and_does_not_hang() {
        let points = vec![Pos2::new(5.0, 5.0), Pos2::new(5.0, 5.0)];
        assert!(dashes(&points, &[4.0, 4.0]).is_empty());
    }
}
