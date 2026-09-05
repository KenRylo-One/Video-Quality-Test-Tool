//! The `ffprobe` media probe.

use crate::error::{BackendError, Result};
use serde_json::Value;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use vqtt_core::media::{
    ColorRange, FrameSample, LumaExtremes, MediaInfo, Rational, bit_depth_from_pix_fmt,
};
use vqtt_core::probe::MediaProbe;

/// Reads media information with `ffprobe`.
#[derive(Debug, Clone)]
pub struct FfprobeProbe {
    program: PathBuf,
}

impl FfprobeProbe {
    /// Builds a probe that calls this program.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// The program that this probe calls.
    pub fn program(&self) -> &Path {
        &self.program
    }
}

/// The `ffprobe` arguments that read one file.
///
/// This function is pure, so a golden test can compare the argument list with no binary.
pub fn probe_args(path: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-select_streams"),
        OsString::from("v:0"),
        OsString::from("-show_entries"),
        OsString::from(
            "stream=codec_name,profile,width,height,pix_fmt,color_range,color_space,r_frame_rate,avg_frame_rate,nb_frames,duration,bit_rate",
        ),
        OsString::from("-show_entries"),
        OsString::from("format=duration,bit_rate,size"),
        OsString::from("-of"),
        OsString::from("json"),
        path.as_os_str().to_os_string(),
    ]
}

/// Escapes a path for use inside a `lavfi` filter graph.
///
/// A Windows path holds a drive colon, and a colon separates filter options, so the path
/// separator is turned around first to keep only one character needing escape. That
/// colon still needs two backslashes and not one: the filtergraph's option-list scanner
/// consumes the first level, and the value parser underneath consumes the second.
/// Verified against a real `ffprobe -f lavfi movie=...,signalstats` invocation, since the
/// ffmpeg documentation states one level for every special character alike.
pub fn escape_lavfi(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut escaped = String::with_capacity(text.len() + 8);
    for character in text.chars() {
        if character == ':' {
            escaped.push_str("\\\\");
        } else if matches!(character, ',' | '\'' | '[' | ']' | ';' | '=') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// The `ffprobe` arguments that report the luma minimum and maximum of each frame.
pub fn luma_args(path: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-f"),
        OsString::from("lavfi"),
        OsString::from(format!("movie={},signalstats", escape_lavfi(path))),
        OsString::from("-show_entries"),
        OsString::from("frame_tags=lavfi.signalstats.YMIN,lavfi.signalstats.YMAX"),
        OsString::from("-of"),
        OsString::from("csv=p=0"),
    ]
}

/// Reads one `YMIN,YMAX` line of the luma report.
pub fn parse_luma_line(line: &str) -> Option<(u16, u16)> {
    let (min, max) = line.trim().split_once(',')?;
    Some((min.trim().parse().ok()?, max.trim().parse().ok()?))
}

/// Reads the JSON record that `ffprobe` wrote.
///
/// This function is pure. Every field of `ffprobe` can arrive as a number or as a string,
/// and a missing field is normal, so every read is tolerant.
pub fn parse_probe_json(json: &str, path: &Path, file_bytes: u64) -> Result<MediaInfo> {
    let root: Value = serde_json::from_str(json).map_err(|error| BackendError::Parse {
        program: "ffprobe".into(),
        detail: error.to_string(),
    })?;

    let stream = root
        .get("streams")
        .and_then(Value::as_array)
        .and_then(|streams| streams.first())
        .ok_or_else(|| {
            BackendError::from(vqtt_core::CoreError::NoVideoStream(
                path.display().to_string(),
            ))
        })?;

    let format = root.get("format");

    let pix_fmt = text(stream.get("pix_fmt")).unwrap_or_else(|| "unknown".to_string());
    let frame_rate = text(stream.get("r_frame_rate"))
        .and_then(|value| Rational::parse(&value))
        .or_else(|| text(stream.get("avg_frame_rate")).and_then(|value| Rational::parse(&value)))
        .unwrap_or(Rational::ZERO);

    let bytes = format
        .and_then(|format| number(format.get("size")))
        .unwrap_or(file_bytes);

    Ok(MediaInfo {
        path: path.to_path_buf(),
        bytes,
        codec: text(stream.get("codec_name")).unwrap_or_else(|| "unknown".to_string()),
        profile: text(stream.get("profile")),
        width: number(stream.get("width")).unwrap_or(0) as u32,
        height: number(stream.get("height")).unwrap_or(0) as u32,
        bit_depth: bit_depth_from_pix_fmt(&pix_fmt),
        pix_fmt,
        color_range: text(stream.get("color_range"))
            .map(|value| ColorRange::from_ffprobe(&value))
            .unwrap_or(ColorRange::Unknown),
        color_space: text(stream.get("color_space")),
        frame_rate,
        nb_frames: number(stream.get("nb_frames")),
        duration_s: real(stream.get("duration"))
            .or_else(|| format.and_then(|f| real(f.get("duration")))),
        bit_rate: number(stream.get("bit_rate"))
            .or_else(|| format.and_then(|f| number(f.get("bit_rate")))),
    })
}

/// Reads a field that holds text, and drops the `N/A` that `ffprobe` writes.
fn text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "N/A" || trimmed == "unknown" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Reads a field that holds a whole number, as a number or as text.
fn number(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Reads a field that holds a real number, as a number or as text.
fn real(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

impl MediaProbe for FfprobeProbe {
    type Error = BackendError;

    fn probe(&self, path: &Path) -> Result<MediaInfo> {
        let output = Command::new(&self.program)
            .args(probe_args(path))
            .output()
            .map_err(|source| BackendError::Spawn {
                program: self.program.display().to_string(),
                source,
            })?;

        if !output.status.success() {
            return Err(BackendError::Exit {
                program: self.program.display().to_string(),
                code: match output.status.code() {
                    Some(code) => code.to_string(),
                    None => "signal".to_string(),
                },
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        let file_bytes = std::fs::metadata(path).map(|data| data.len()).unwrap_or(0);
        parse_probe_json(&String::from_utf8_lossy(&output.stdout), path, file_bytes)
    }

    fn luma_extremes(&self, path: &Path, sample: FrameSample) -> Result<LumaExtremes> {
        let mut child = Command::new(&self.program)
            .args(luma_args(path))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| BackendError::Spawn {
                program: self.program.display().to_string(),
                source,
            })?;

        let mut y_min = u16::MAX;
        let mut y_max = 0_u16;
        let mut read = 0_u32;

        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                let Some((low, high)) = parse_luma_line(&line) else {
                    continue;
                };
                y_min = y_min.min(low);
                y_max = y_max.max(high);
                read += 1;
                if read >= sample.max_frames {
                    break;
                }
            }
        }

        // The tool asked for a sample, so it stops the process rather than reading the
        // whole file.
        let _ = child.kill();
        let _ = child.wait();

        if read == 0 {
            return Err(BackendError::Parse {
                program: self.program.display().to_string(),
                detail: "signalstats reported no frame".into(),
            });
        }

        Ok(LumaExtremes {
            sampled_frames: read,
            y_min,
            y_max,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_A: &str = r#"{
        "streams": [{
            "codec_name": "h264",
            "width": 512,
            "height": 256,
            "pix_fmt": "yuvj420p",
            "color_range": "pc",
            "color_space": "bt709",
            "r_frame_rate": "30/1",
            "avg_frame_rate": "30/1",
            "duration": "5.000000",
            "bit_rate": "17076",
            "nb_frames": "150"
        }],
        "format": { "duration": "5.000000", "size": "12135", "bit_rate": "19416" }
    }"#;

    #[test]
    fn reads_the_full_range_flag_of_test_a() {
        let info = parse_probe_json(TEST_A, Path::new("TEST_A.mp4"), 0).unwrap();
        assert_eq!(info.color_range, ColorRange::Pc);
        assert_eq!(info.color_range.tag(), "pc");
        assert_eq!(info.width, 512);
        assert_eq!(info.height, 256);
        assert_eq!(info.bit_depth, 8);
        assert_eq!(info.frame_rate.label(), "30");
        assert_eq!(info.nb_frames, Some(150));
        assert_eq!(info.bytes, 12135);
        assert!(info.pix_fmt_is_full_range());
    }

    #[test]
    fn a_missing_field_does_not_stop_the_probe() {
        let json = r#"{ "streams": [{ "codec_name": "h264", "width": 1920, "height": 1080 }] }"#;
        let info = parse_probe_json(json, Path::new("a.mp4"), 4096).unwrap();
        assert_eq!(info.color_range, ColorRange::Unknown);
        assert_eq!(info.pix_fmt, "unknown");
        assert_eq!(info.frame_rate, Rational::ZERO);
        assert_eq!(info.bytes, 4096);
        assert_eq!(info.bitrate_label(), "unknown");
    }

    #[test]
    fn a_file_with_no_video_stream_reports_that() {
        let json = r#"{ "streams": [] }"#;
        let error = parse_probe_json(json, Path::new("audio.m4a"), 0).unwrap_err();
        assert!(error.to_string().contains("no video stream"), "{error}");
    }

    #[test]
    fn the_probe_argument_list_never_changes_by_accident() {
        let args: Vec<String> = probe_args(Path::new("/clips/a.mp4"))
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,profile,width,height,pix_fmt,color_range,color_space,r_frame_rate,avg_frame_rate,nb_frames,duration,bit_rate",
                "-show_entries",
                "format=duration,bit_rate,size",
                "-of",
                "json",
                "/clips/a.mp4",
            ]
        );
    }

    #[test]
    fn a_windows_drive_colon_needs_a_second_level_of_escaping() {
        assert_eq!(
            escape_lavfi(Path::new(r"D:\clips\a b.mp4")),
            r"D\\:/clips/a b.mp4"
        );
        assert_eq!(
            escape_lavfi(Path::new("/clips/a,b.mp4")),
            r"/clips/a\,b.mp4"
        );
        assert_eq!(escape_lavfi(Path::new("/clips/a.mp4")), "/clips/a.mp4");
    }

    #[test]
    fn reads_one_line_of_the_luma_report() {
        assert_eq!(parse_luma_line("16,235"), Some((16, 235)));
        assert_eq!(parse_luma_line("0,255\n"), Some((0, 255)));
        assert_eq!(parse_luma_line(""), None);
    }
}
