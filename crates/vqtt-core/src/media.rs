//! What the tool knows about one media file.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// The color range flag that the container carries.
///
/// A mismatch between the reference and an encode shifts every pixel by about 7% of the
/// range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorRange {
    /// Limited range, 16 to 235 at 8 bit. `ffprobe` reports `tv`.
    Tv,
    /// Full range, 0 to 255 at 8 bit. `ffprobe` reports `pc`.
    Pc,
    /// The file carries no flag.
    Unknown,
}

impl ColorRange {
    /// Reads the `color_range` field of `ffprobe`.
    pub fn from_ffprobe(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "tv" | "limited" | "mpeg" => Self::Tv,
            "pc" | "full" | "jpeg" => Self::Pc,
            _ => Self::Unknown,
        }
    }

    /// The short tag for the file row. The design shows `tv` and `pc`.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Tv => "tv",
            Self::Pc => "pc",
            Self::Unknown => "unset",
        }
    }

    /// The value that the FFmpeg `scale` and `format` filters take.
    pub fn ffmpeg_value(self) -> Option<&'static str> {
        match self {
            Self::Tv => Some("limited"),
            Self::Pc => Some("full"),
            Self::Unknown => None,
        }
    }
}

impl fmt::Display for ColorRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// A frame rate, held as the fraction that `ffprobe` reports.
///
/// The tool never rounds this value for a decision. 60000/1001 is not 60.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rational {
    /// The numerator.
    pub num: u64,
    /// The denominator.
    pub den: u64,
}

impl Rational {
    /// A frame rate of zero. Used when the file reports none.
    pub const ZERO: Self = Self { num: 0, den: 1 };

    /// Reads a `num/den` string, as `r_frame_rate` gives it.
    pub fn parse(text: &str) -> Option<Self> {
        let (num, den) = text.trim().split_once('/')?;
        let num: u64 = num.trim().parse().ok()?;
        let den: u64 = den.trim().parse().ok()?;
        if den == 0 {
            return None;
        }
        Some(Self { num, den })
    }

    /// The value as a floating point number.
    pub fn as_f64(self) -> f64 {
        if self.den == 0 {
            0.0
        } else {
            self.num as f64 / self.den as f64
        }
    }

    /// The short label for the file row: `30`, `59.94`, `23.976`.
    pub fn label(self) -> String {
        let value = self.as_f64();
        if (value - value.round()).abs() < 1e-6 {
            return format!("{}", value.round() as i64);
        }
        let text = format!("{value:.3}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }

    /// True for 50 fps and above. This chooses an `_hfr` VMAF model when the tool picks
    /// the model for a measurement.
    pub fn is_high_frame_rate(self) -> bool {
        self.as_f64() >= 49.0
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

/// Reads the bit depth out of an FFmpeg pixel format name.
///
/// The name is the only place that carries the depth. `ffprobe` reports no depth field
/// in the stream record that the tool reads.
pub fn bit_depth_from_pix_fmt(pix_fmt: &str) -> u8 {
    let base = pix_fmt
        .strip_suffix("le")
        .or_else(|| pix_fmt.strip_suffix("be"))
        .unwrap_or(pix_fmt);

    // Planar formats carry the depth after the last `p`: yuv420p10, p010.
    if let Some(index) = base.rfind('p') {
        let tail = &base[index + 1..];
        if !tail.is_empty()
            && tail.bytes().all(|b| b.is_ascii_digit())
            && let Ok(depth) = tail.parse::<u8>()
            && (8..=16).contains(&depth)
        {
            return depth;
        }
    }

    // Gray formats carry the depth directly: gray10, gray12.
    if let Some(tail) = base.strip_prefix("gray")
        && let Ok(depth) = tail.parse::<u8>()
        && (8..=16).contains(&depth)
    {
        return depth;
    }

    // Packed formats carry the total for every component: rgb24, rgba64, bgr48.
    for (prefix, components) in [
        ("rgba", 4u8),
        ("bgra", 4),
        ("argb", 4),
        ("abgr", 4),
        ("rgb", 3),
        ("bgr", 3),
    ] {
        if let Some(tail) = base.strip_prefix(prefix)
            && let Ok(total) = tail.parse::<u16>()
        {
            let depth = total / u16::from(components);
            if (8..=16).contains(&depth) {
                return depth as u8;
            }
        }
    }

    8
}

/// True for the deprecated JPEG pixel formats, which are full range whatever the flag says.
pub fn pix_fmt_is_full_range(pix_fmt: &str) -> bool {
    pix_fmt.starts_with("yuvj")
}

/// What `ffprobe` reports about one file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    /// The path that the user gave.
    pub path: PathBuf,
    /// The size of the file on disk.
    pub bytes: u64,
    /// The video codec name, for example `h264`.
    pub codec: String,
    /// The codec profile, for example `4444`.
    pub profile: Option<String>,
    /// The coded width.
    pub width: u32,
    /// The coded height.
    pub height: u32,
    /// The pixel format name, for example `yuv420p10le`.
    pub pix_fmt: String,
    /// The bit depth, read out of the pixel format name.
    pub bit_depth: u8,
    /// The color range flag.
    pub color_range: ColorRange,
    /// The color space name, for example `bt709`.
    pub color_space: Option<String>,
    /// The frame rate, as a fraction.
    pub frame_rate: Rational,
    /// The frame count, when the container reports one.
    pub nb_frames: Option<u64>,
    /// The duration in seconds.
    pub duration_s: Option<f64>,
    /// The video bit rate in bits for each second.
    pub bit_rate: Option<u64>,
}

impl MediaInfo {
    /// The `3840x2160` label for the file row.
    pub fn resolution_label(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    /// The pixel count of one frame.
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// The frame count, from the container or from the duration.
    pub fn frame_count(&self) -> Option<u64> {
        if let Some(count) = self.nb_frames
            && count > 0
        {
            return Some(count);
        }
        let duration = self.duration_s?;
        let rate = self.frame_rate.as_f64();
        if duration > 0.0 && rate > 0.0 {
            Some((duration * rate).round() as u64)
        } else {
            None
        }
    }

    /// The bit rate label for the file row: `100 Mbps`, `17 kbps`.
    pub fn bitrate_label(&self) -> String {
        match self.bit_rate {
            None => "unknown".to_string(),
            Some(bits) if bits >= 10_000_000 => format!("{} Mbps", bits / 1_000_000),
            Some(bits) if bits >= 1_000_000 => format!("{:.1} Mbps", bits as f64 / 1e6),
            Some(bits) if bits >= 1_000 => format!("{} kbps", bits / 1_000),
            Some(bits) => format!("{bits} bps"),
        }
    }

    /// The file name without the folder. The interface shows this.
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    /// True when the pixel data is full range, whatever the flag says.
    pub fn pix_fmt_is_full_range(&self) -> bool {
        pix_fmt_is_full_range(&self.pix_fmt)
    }

    /// The color range that the pixel data really has.
    ///
    /// A `yuvj` pixel format is full range whatever the flag says. An unset flag then
    /// means limited, which is the convention for every other YUV format and what a
    /// decoder assumes. Reading unset as full would convert an already limited encode a
    /// second time and move every level by about 7% of the range.
    pub fn effective_color_range(&self) -> ColorRange {
        if self.pix_fmt_is_full_range() {
            ColorRange::Pc
        } else if self.color_range == ColorRange::Unknown {
            ColorRange::Tv
        } else {
            self.color_range
        }
    }
}

/// The real luma minimum and maximum of a sample of frames.
///
/// The flag can disagree with the data. The tool reports that case, and never
/// corrects it, since it cannot tell a wrong flag from low-contrast content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LumaExtremes {
    /// How many frames the sample held.
    pub sampled_frames: u32,
    /// The lowest luma value in the sample.
    pub y_min: u16,
    /// The highest luma value in the sample.
    pub y_max: u16,
}

/// How many frames to read for the luma sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSample {
    /// The largest number of frames to read.
    pub max_frames: u32,
}

impl Default for FrameSample {
    fn default() -> Self {
        Self { max_frames: 60 }
    }
}

/// The number of bytes that each end of the quick fingerprint covers.
pub const FINGERPRINT_CHUNK_BYTES: u64 = 8 * 1024 * 1024;

/// A quick file fingerprint.
///
/// A SHA-256 hash of a 50 GB reference costs minutes on every run. This covers the size,
/// the first 8 MiB and the last 8 MiB instead. It detects a different file, a truncated
/// file and a re-render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// The size of the file.
    pub bytes: u64,
    /// The SHA-256 hash of the first chunk.
    pub head: String,
    /// The SHA-256 hash of the last chunk.
    pub tail: String,
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "qf1:{}:{}:{}", self.bytes, self.head, self.tail)
    }
}

impl std::str::FromStr for Fingerprint {
    type Err = crate::CoreError;

    fn from_str(text: &str) -> crate::Result<Self> {
        let mut parts = text.split(':');
        let version = parts.next().unwrap_or_default();
        if version != "qf1" {
            return Err(crate::CoreError::parse(
                "fingerprint",
                format!("unknown version {version}"),
            ));
        }
        let bytes = parts
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| crate::CoreError::parse("fingerprint", "no size"))?;
        let head = parts
            .next()
            .ok_or_else(|| crate::CoreError::parse("fingerprint", "no head hash"))?;
        let tail = parts
            .next()
            .ok_or_else(|| crate::CoreError::parse("fingerprint", "no tail hash"))?;
        Ok(Self {
            bytes,
            head: head.to_string(),
            tail: tail.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_color_range_flag() {
        assert_eq!(ColorRange::from_ffprobe("pc"), ColorRange::Pc);
        assert_eq!(ColorRange::from_ffprobe("tv"), ColorRange::Tv);
        assert_eq!(ColorRange::from_ffprobe("TV"), ColorRange::Tv);
        assert_eq!(ColorRange::from_ffprobe(""), ColorRange::Unknown);
        assert_eq!(ColorRange::from_ffprobe("unknown"), ColorRange::Unknown);
    }

    #[test]
    fn reads_the_bit_depth_out_of_the_pixel_format() {
        assert_eq!(bit_depth_from_pix_fmt("yuv420p"), 8);
        assert_eq!(bit_depth_from_pix_fmt("yuvj420p"), 8);
        assert_eq!(bit_depth_from_pix_fmt("yuv420p10le"), 10);
        assert_eq!(bit_depth_from_pix_fmt("yuv444p10le"), 10);
        assert_eq!(bit_depth_from_pix_fmt("yuv422p12le"), 12);
        assert_eq!(bit_depth_from_pix_fmt("p010le"), 10);
        assert_eq!(bit_depth_from_pix_fmt("gray10le"), 10);
        assert_eq!(bit_depth_from_pix_fmt("rgb24"), 8);
        assert_eq!(bit_depth_from_pix_fmt("rgba64le"), 16);
        assert_eq!(bit_depth_from_pix_fmt("nv12"), 8);
    }

    #[test]
    fn an_unset_flag_on_a_yuv_format_means_limited_range() {
        assert_eq!(ColorRange::Unknown.ffmpeg_value(), None);
        assert_eq!(ColorRange::Tv.ffmpeg_value(), Some("limited"));
        assert_eq!(ColorRange::Pc.ffmpeg_value(), Some("full"));
        // The flag itself keeps saying unset, because the Files section must not claim a
        // flag the file does not carry.
        assert_eq!(ColorRange::Unknown.tag(), "unset");
    }

    #[test]
    fn names_the_jpeg_pixel_formats_as_full_range() {
        assert!(pix_fmt_is_full_range("yuvj420p"));
        assert!(!pix_fmt_is_full_range("yuv420p"));
    }

    #[test]
    fn labels_a_frame_rate_without_rounding_the_decision() {
        assert_eq!(Rational::parse("30/1").unwrap().label(), "30");
        assert_eq!(Rational::parse("60000/1001").unwrap().label(), "59.94");
        assert_eq!(Rational::parse("30000/1001").unwrap().label(), "29.97");
        assert_eq!(Rational::parse("24000/1001").unwrap().label(), "23.976");
        assert_eq!(Rational::parse("0/0"), None);
    }

    #[test]
    fn names_fifty_frames_a_second_as_high_frame_rate() {
        assert!(Rational::parse("60000/1001").unwrap().is_high_frame_rate());
        assert!(Rational::parse("50/1").unwrap().is_high_frame_rate());
        assert!(!Rational::parse("30000/1001").unwrap().is_high_frame_rate());
    }

    #[test]
    fn reads_back_a_fingerprint() {
        let print = Fingerprint {
            bytes: 51539607552,
            head: "3c8a".into(),
            tail: "9d40".into(),
        };
        let text = print.to_string();
        assert_eq!(text, "qf1:51539607552:3c8a:9d40");
        assert_eq!(text.parse::<Fingerprint>().unwrap(), print);
    }
}
