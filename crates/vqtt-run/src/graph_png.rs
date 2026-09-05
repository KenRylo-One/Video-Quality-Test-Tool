//! Turns a plot into PNG bytes.
//!
//! It rasterizes the SVG that `vqtt_core::plot_svg` already wrote, rather than drawing
//! the scene a second time. The two files cannot then disagree.

use resvg::tiny_skia;
use resvg::usvg;
use vqtt_core::{CoreError, Result};

/// Renders an SVG document at `scale` times its own size.
///
/// The fonts arrive as bytes because this crate ships no assets. The caller passes the
/// same faces the window draws with, or the text falls back to whatever the renderer
/// finds.
pub fn png_from_svg(svg: &str, fonts: &[&[u8]], scale: f32) -> Result<Vec<u8>> {
    let mut database = usvg::fontdb::Database::new();
    for face in fonts {
        database.load_font_data(face.to_vec());
    }

    let mut options = usvg::Options {
        fontdb: std::sync::Arc::new(database),
        ..usvg::Options::default()
    };
    options.font_family = "IBM Plex Mono".to_string();

    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|error| CoreError::parse("graph png", error.to_string()))?;

    let size = tree.size();
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| CoreError::parse("graph png", "the image size is not usable".to_string()))?;

    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    pixmap
        .encode_png()
        .map_err(|error| CoreError::parse("graph png", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vqtt_core::media::Rational;
    use vqtt_core::metric::MetricId;
    use vqtt_core::palette::Theme;
    use vqtt_core::plot::{PlotRequest, SeriesInput, build_scenes};
    use vqtt_core::plot_svg::to_svg;
    use vqtt_core::set::FileId;

    fn plot_svg() -> String {
        let values: Vec<f32> = (0..300).map(|index| 30.0 + (index % 9) as f32).collect();
        let inputs = vec![SeriesInput {
            file: FileId(1),
            slot: Some(0),
            name: "encode one",
            values: &values,
        }];
        let request = PlotRequest {
            metric: MetricId::PsnrY,
            series: &inputs,
            theme: Theme::Dark,
            high_contrast: false,
            x_domain: (0.0, 1.0),
            first_frame: 0,
            frame_rate: Rational { num: 60, den: 1 },
            size: (600.0, 260.0),
            hover_frame: None,
        };
        to_svg(&build_scenes(&request)[0], Theme::Dark)
    }

    /// The eight-byte PNG signature, so the test reads the format and not the length.
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    #[test]
    fn a_plot_becomes_a_png_of_the_size_the_scene_asked_for() {
        let png = png_from_svg(&plot_svg(), &[], 1.0).unwrap();

        assert_eq!(png[..8], PNG_MAGIC);
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((width, height), (600, 260));
    }

    #[test]
    fn a_scale_of_two_gives_an_image_of_twice_the_size() {
        let png = png_from_svg(&plot_svg(), &[], 2.0).unwrap();

        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((width, height), (1200, 520));
    }

    #[test]
    fn a_document_that_is_not_svg_gives_an_error_and_does_not_panic() {
        assert!(png_from_svg("not an svg at all", &[], 1.0).is_err());
    }
}
