//! Pulls one frame out of both files, three ways.
//!
//! An exact frame select on a long-GOP file seeks to a keyframe and decodes forward, so
//! this must never run on the interface thread. Everything it writes is cached by run,
//! encode and frame, which is what makes the worse and better controls step quickly.

use resvg::tiny_skia;
use std::path::{Path, PathBuf};
use vqtt_backends::ffmpeg::plan_frame_extract;
use vqtt_core::backend::{Invocation, MeasureJob};
use vqtt_core::{CoreError, Result};

/// The three images of one frame, and the commands that made them.
pub struct ExtractedFrame {
    pub frame: u64,
    pub gain: u32,
    pub reference: PathBuf,
    pub encode: PathBuf,
    pub difference: PathBuf,
    /// Every command, for the record and for a reader who wants to check by hand. A
    /// wrong seek gives the wrong frame and no error message.
    pub commands: Vec<String>,
}

/// One decoded image, ready for a texture.
pub struct Rgba8 {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Writes the three images and returns where they are.
///
/// An image that is already on disk is not made again. The two stills carry no gain in
/// their names, so only the difference is redrawn when the gain changes.
pub fn extract_frame(
    job: &MeasureJob,
    program: &Path,
    frame: u64,
    gain: u32,
) -> Result<ExtractedFrame> {
    std::fs::create_dir_all(&job.work_dir)
        .map_err(|error| CoreError::parse("frame viewer", error.to_string()))?;

    let plan = plan_frame_extract(job, frame, gain)?;
    let mut commands = Vec::new();

    for (invocation, output) in [
        (&plan.reference, &plan.reference_path),
        (&plan.encode, &plan.encode_path),
        (&plan.difference, &plan.difference_path),
    ] {
        commands.push(command_line(program, invocation));
        if output.exists() {
            continue;
        }
        run(program, invocation, output)?;
    }

    Ok(ExtractedFrame {
        frame,
        gain,
        reference: plan.reference_path,
        encode: plan.encode_path,
        difference: plan.difference_path,
        commands,
    })
}

fn run(program: &Path, invocation: &Invocation, output: &Path) -> Result<()> {
    let result = std::process::Command::new(program)
        .args(&invocation.args)
        .output()
        .map_err(|error| CoreError::parse("frame viewer", error.to_string()))?;

    // A select that matches nothing leaves ffmpeg reporting success with no file, so
    // the file is what the check reads and not the exit code.
    if !output.exists() {
        let said = String::from_utf8_lossy(&result.stderr);
        let reason = said.lines().last().unwrap_or("it wrote no image").trim();
        return Err(CoreError::parse(
            "frame viewer",
            format!("frame extraction wrote no image: {reason}"),
        ));
    }
    Ok(())
}

fn command_line(program: &Path, invocation: &Invocation) -> String {
    let mut line = quote(&program.to_string_lossy());
    for arg in &invocation.args {
        line.push(' ');
        line.push_str(&quote(&arg.to_string_lossy()));
    }
    line
}

fn quote(text: &str) -> String {
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.to_string()
    }
}

/// Reads a PNG into plain bytes.
///
/// The rasterizer already carries a PNG decoder, so the tool needs no image crate for
/// this one job.
pub fn read_png(path: &Path) -> Result<Rgba8> {
    let pixmap = tiny_skia::Pixmap::load_png(path)
        .map_err(|error| CoreError::parse("frame viewer", error.to_string()))?;
    Ok(Rgba8 {
        width: pixmap.width(),
        height: pixmap.height(),
        pixels: pixmap.data().to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use vqtt_core::backend::JobInput;
    use vqtt_core::media::{ColorRange, MediaInfo, Rational};
    use vqtt_core::metric::MetricId;

    fn info(name: &str) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from(name),
            bytes: 0,
            codec: "h264".into(),
            profile: None,
            width: 1920,
            height: 1080,
            pix_fmt: "yuv420p".into(),
            bit_depth: 8,
            color_range: ColorRange::Tv,
            color_space: Some("bt709".into()),
            frame_rate: Rational { num: 30, den: 1 },
            nb_frames: Some(150),
            duration_s: Some(5.0),
            bit_rate: None,
        }
    }

    fn job(work_dir: PathBuf) -> MeasureJob {
        MeasureJob {
            reference: JobInput {
                path: PathBuf::from("reference.mp4"),
                info: info("reference.mp4"),
            },
            encode: JobInput {
                path: PathBuf::from("encode.mp4"),
                info: info("encode.mp4"),
            },
            metrics: [MetricId::PsnrY].into_iter().collect::<BTreeSet<_>>(),
            frame_range: None,
            fused_passes: true,
            work_dir,
            vmaf_models: Vec::new(),
            vmaf_viewing_distance: 3.0,
            butteraugli_intensity_nits: 203,
            vship_gpu_threads: 1,
        }
    }

    #[test]
    fn a_binary_that_writes_no_image_gives_an_error_and_never_a_silent_pass() {
        let dir = std::env::temp_dir().join("vqtt-frames-missing");
        let _ = std::fs::remove_dir_all(&dir);

        let outcome = extract_frame(&job(dir), Path::new("no-such-binary"), 4, 2);

        assert!(outcome.is_err());
    }

    #[test]
    fn the_commands_are_reported_so_a_reader_can_repeat_the_extraction() {
        let dir = std::env::temp_dir().join("vqtt-frames-commands");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Every image is already there, so nothing runs and the commands still report.
        let plan = plan_frame_extract(&job(dir.clone()), 4, 2).unwrap();
        for path in [
            &plan.reference_path,
            &plan.encode_path,
            &plan.difference_path,
        ] {
            std::fs::write(path, b"not really a png").unwrap();
        }

        let extracted = extract_frame(&job(dir), Path::new("ffmpeg"), 4, 2).unwrap();

        assert_eq!(extracted.commands.len(), 3);
        assert!(extracted.commands[0].contains("f4_ref.png"));
        assert!(extracted.commands[2].contains("blend=all_mode=difference"));
    }

    #[test]
    fn a_path_with_a_space_in_it_is_quoted_so_the_command_can_be_pasted_back() {
        assert_eq!(
            quote("C:/Program Files/a.exe"),
            "\"C:/Program Files/a.exe\""
        );
        assert_eq!(quote("ffmpeg"), "ffmpeg");
    }
}
