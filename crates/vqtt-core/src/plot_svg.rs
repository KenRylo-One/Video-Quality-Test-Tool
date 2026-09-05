//! Writes a `Scene` as SVG.
//!
//! This is a sink for `plot::draw`, not a second drawing routine. It decides nothing
//! beyond how one shape is spelled in XML.

use crate::palette::Theme;
use crate::plot::{Canvas, Chrome, Rect, Rgba, Scene, TextAlign, draw};

/// The font stack the window uses, with a fallback for a reader that does not have it.
const FONT_STACK: &str = "IBM Plex Mono, ui-monospace, monospace";

#[derive(Default)]
pub struct SvgCanvas {
    body: String,
}

impl SvgCanvas {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_body(self) -> String {
        self.body
    }

    fn stroke(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: Rgba) {
        self.body.push_str(&format!(
            "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{}\"{} stroke-width=\"{width}\"/>\n",
            from.0,
            from.1,
            to.0,
            to.1,
            hex(color),
            opacity(color, "stroke-opacity"),
        ));
    }
}

impl Canvas for SvgCanvas {
    fn rect(&mut self, rect: Rect, fill: Rgba) {
        self.body.push_str(&format!(
            "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{}\"{}/>\n",
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            hex(fill),
            opacity(fill, "fill-opacity"),
        ));
    }

    fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: Rgba) {
        self.stroke(from, to, width, color);
    }

    fn polygon(&mut self, points: &[(f32, f32)], fill: Rgba) {
        self.body.push_str(&format!(
            "<polygon points=\"{}\" fill=\"{}\"{}/>\n",
            point_list(points),
            hex(fill),
            opacity(fill, "fill-opacity"),
        ));
    }

    fn polyline(&mut self, points: &[(f32, f32)], width: f32, color: Rgba) {
        self.body.push_str(&format!(
            "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\"{} stroke-width=\"{width}\" stroke-linejoin=\"round\"/>\n",
            point_list(points),
            hex(color),
            opacity(color, "stroke-opacity"),
        ));
    }

    fn text(&mut self, at: (f32, f32), align: TextAlign, text: &str, size: f32, color: Rgba) {
        let (anchor, baseline) = anchors(align);
        self.body.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-family=\"{FONT_STACK}\" font-size=\"{size}\" fill=\"{}\"{} text-anchor=\"{anchor}\" dominant-baseline=\"{baseline}\">{}</text>\n",
            at.0,
            at.1,
            hex(color),
            opacity(color, "fill-opacity"),
            escape(text),
        ));
    }
}

/// One scene as a complete SVG document, on the theme's own background.
pub fn to_svg(scene: &Scene, theme: Theme) -> String {
    let chrome = Chrome::for_theme(theme);
    let mut canvas = SvgCanvas::new();
    draw(scene, &chrome, (0.0, 0.0), &mut canvas);

    let (width, height) = scene.size;
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
viewBox=\"0 0 {width:.0} {height:.0}\">\n\
<rect width=\"{width:.0}\" height=\"{height:.0}\" fill=\"{}\"/>\n{}</svg>\n",
        hex(chrome.surface),
        canvas.into_body(),
    )
}

fn hex(color: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn opacity(color: Rgba, attribute: &str) -> String {
    if color.a == 255 {
        return String::new();
    }
    format!(" {attribute}=\"{:.3}\"", color.a as f32 / 255.0)
}

fn point_list(points: &[(f32, f32)]) -> String {
    let mut out = String::with_capacity(points.len() * 14);
    for (index, (x, y)) in points.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{x:.2},{y:.2}"));
    }
    out
}

fn anchors(align: TextAlign) -> (&'static str, &'static str) {
    match align {
        TextAlign::LeftTop => ("start", "hanging"),
        TextAlign::LeftCenter => ("start", "middle"),
        TextAlign::LeftBottom => ("start", "alphabetic"),
        TextAlign::CenterTop => ("middle", "hanging"),
        TextAlign::CenterCenter => ("middle", "middle"),
        TextAlign::RightCenter => ("end", "middle"),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Rational;
    use crate::metric::MetricId;
    use crate::plot::{PlotRequest, SeriesInput, build_scenes};
    use crate::set::FileId;

    /// Slot one, not slot zero: the palette keeps slot zero solid even in high contrast.
    fn scene_of(values: &[f32], high_contrast: bool) -> Scene {
        let inputs = vec![SeriesInput {
            file: FileId(1),
            slot: Some(1),
            name: "encode one",
            values,
        }];
        let request = PlotRequest {
            metric: MetricId::PsnrY,
            series: &inputs,
            theme: Theme::Dark,
            high_contrast,
            x_domain: (0.0, 1.0),
            first_frame: 0,
            frame_rate: Rational { num: 60, den: 1 },
            size: (600.0, 260.0),
            hover_frame: None,
        };
        build_scenes(&request).remove(0)
    }

    fn ramp() -> Vec<f32> {
        (0..300).map(|index| 30.0 + (index % 10) as f32).collect()
    }

    #[test]
    fn a_series_gives_one_polyline_a_band_and_both_tick_rows() {
        let svg = to_svg(&scene_of(&ramp(), false), Theme::Dark);

        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert_eq!(svg.matches("<polyline").count(), 1);
        assert_eq!(svg.matches("<polygon").count(), 1);
        assert!(svg.contains("<rect"));
        assert!(svg.contains("higher is better"));
        assert!(svg.contains("dominant-baseline=\"hanging\""));
    }

    #[test]
    fn a_dashed_series_draws_segments_and_a_solid_one_draws_one_line() {
        let solid = to_svg(&scene_of(&ramp(), false), Theme::Dark);
        let dashed = to_svg(&scene_of(&ramp(), true), Theme::Dark);

        assert_eq!(solid.matches("<polyline").count(), 1);
        assert_eq!(dashed.matches("<polyline").count(), 0);
        assert!(dashed.matches("<line").count() > solid.matches("<line").count());
    }

    #[test]
    fn a_scene_with_nothing_to_draw_still_gives_a_document_that_says_so() {
        let svg = to_svg(&scene_of(&[], false), Theme::Dark);

        assert!(svg.contains("No finite value to draw"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn a_partly_clear_colour_carries_its_opacity_and_an_opaque_one_does_not() {
        let svg = to_svg(&scene_of(&ramp(), false), Theme::Dark);

        assert!(svg.contains("fill-opacity="));
        assert!(!svg.contains("fill-opacity=\"1.000\""));
    }

    #[test]
    fn a_name_with_markup_in_it_is_escaped() {
        let inputs = vec![SeriesInput {
            file: FileId(1),
            slot: Some(0),
            name: "a<b>&c",
            values: &[1.0],
        }];
        let request = PlotRequest {
            metric: MetricId::Cvvdp,
            series: &inputs,
            theme: Theme::Dark,
            high_contrast: false,
            x_domain: (0.0, 1.0),
            first_frame: 0,
            frame_rate: Rational { num: 60, den: 1 },
            size: (600.0, 260.0),
            hover_frame: None,
        };
        let svg = to_svg(&build_scenes(&request).remove(0), Theme::Dark);

        assert!(svg.contains("a&lt;b&gt;&amp;c"));
        assert!(!svg.contains("<b>"));
    }
}
