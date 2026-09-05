//! Draws a `Scene` with egui.
//!
//! Every position and colour arrives from `vqtt_core::plot::draw`, which the exported
//! SVG walks as well. This file only spells one shape in `egui` terms.

use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};
use vqtt_core::palette::Theme;
use vqtt_core::plot::{Canvas, Chrome, Rgba, Scene, TextAlign, draw};

struct EguiCanvas<'a> {
    painter: &'a Painter,
}

fn color_of(color: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a)
}

fn point_of(point: (f32, f32)) -> Pos2 {
    Pos2::new(point.0, point.1)
}

fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::MONO_FAMILY.into()))
}

fn align_of(align: TextAlign) -> Align2 {
    match align {
        TextAlign::LeftTop => Align2::LEFT_TOP,
        TextAlign::LeftCenter => Align2::LEFT_CENTER,
        TextAlign::LeftBottom => Align2::LEFT_BOTTOM,
        TextAlign::CenterTop => Align2::CENTER_TOP,
        TextAlign::CenterCenter => Align2::CENTER_CENTER,
        TextAlign::RightCenter => Align2::RIGHT_CENTER,
    }
}

impl Canvas for EguiCanvas<'_> {
    fn rect(&mut self, rect: vqtt_core::plot::Rect, fill: Rgba) {
        self.painter.rect_filled(
            Rect::from_min_size(Pos2::new(rect.x, rect.y), Vec2::new(rect.w, rect.h)),
            0.0,
            color_of(fill),
        );
    }

    fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: Rgba) {
        self.painter.line_segment(
            [point_of(from), point_of(to)],
            Stroke::new(width, color_of(color)),
        );
    }

    fn polygon(&mut self, points: &[(f32, f32)], fill: Rgba) {
        self.painter.add(egui::Shape::convex_polygon(
            points.iter().copied().map(point_of).collect(),
            color_of(fill),
            Stroke::NONE,
        ));
    }

    fn polyline(&mut self, points: &[(f32, f32)], width: f32, color: Rgba) {
        self.painter.add(egui::Shape::line(
            points.iter().copied().map(point_of).collect(),
            Stroke::new(width, color_of(color)),
        ));
    }

    fn text(&mut self, at: (f32, f32), align: TextAlign, text: &str, size: f32, color: Rgba) {
        self.painter.text(
            point_of(at),
            align_of(align),
            text,
            font(size),
            color_of(color),
        );
    }
}

/// Draws one scene into `origin`, which is the top left of the scene's own box.
pub fn paint(painter: &Painter, theme: Theme, scene: &Scene, origin: Pos2) {
    let mut canvas = EguiCanvas { painter };
    draw(
        scene,
        &Chrome::for_theme(theme),
        (origin.x, origin.y),
        &mut canvas,
    );
}
