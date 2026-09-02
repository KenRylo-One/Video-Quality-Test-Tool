use crate::media::{ColorRange, LumaExtremes, MediaInfo};
use crate::metric::MetricId;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionId {
    ColorRange,
    Resolution,
    FrameCount,
}

impl CorrectionId {
    /// A short category name, shown before the correction's own message.
    pub fn label(self) -> &'static str {
        match self {
            Self::ColorRange => "Color range",
            Self::Resolution => "Resolution",
            Self::FrameCount => "Frame count",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteId {
    RangeFlagDisagreesWithData,
    NearLosslessReference,
    KnownMetricFault,
    /// The frame count fix assumes both files start at the same frame. It cannot tell
    /// a file trimmed at the end from a file trimmed at the start, so it names the
    /// assumption every time it fires.
    FrameCountAlignmentAssumed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CorrectionDetail {
    ColorRange {
        from: ColorRange,
        to: ColorRange,
    },
    Resolution {
        pre_scale_width: u32,
        pre_scale_height: u32,
    },
    FrameCount {
        first_frame: u64,
        last_frame: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    pub id: CorrectionId,
    pub target_label: String,
    pub message: String,
    pub detail: CorrectionDetail,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: NoteId,
    pub message: String,
}

/// Codec names that carry a small loss of their own, even at a high bit rate.
const NEAR_LOSSLESS_CODECS: [&str; 3] = ["prores", "dnxhd", "dnxhr"];

/// Detects a color range mismatch, and builds the correction that fixes it.
///
/// The encode is converted to the range of the reference. The reference never changes.
pub fn detect_color_range(
    reference: &MediaInfo,
    encode: &MediaInfo,
    target_label: &str,
) -> Option<Correction> {
    let from = encode.effective_color_range();
    let to = reference.effective_color_range();
    if from == to {
        return None;
    }
    Some(Correction {
        id: CorrectionId::ColorRange,
        target_label: target_label.to_string(),
        message: format!(
            "{target_label} is {} range. The reference is {} range. Converted the encode to {} range for the measurement.",
            from.tag(),
            to.tag(),
            to.tag()
        ),
        detail: CorrectionDetail::ColorRange { from, to },
    })
}

/// Detects a frame size mismatch, and builds the correction that fixes it.
///
/// The encode is always scaled to the size of the reference, up or down. The reference
/// is never scaled.
pub fn detect_resolution(
    reference: &MediaInfo,
    encode: &MediaInfo,
    target_label: &str,
) -> Option<Correction> {
    if reference.width == encode.width && reference.height == encode.height {
        return None;
    }
    Some(Correction {
        id: CorrectionId::Resolution,
        target_label: target_label.to_string(),
        message: format!(
            "{target_label} was scaled from {}x{} to {}x{}, bicubic. The encode is always scaled up. The reference is never scaled down.",
            encode.width, encode.height, reference.width, reference.height
        ),
        detail: CorrectionDetail::Resolution {
            pre_scale_width: encode.width,
            pre_scale_height: encode.height,
        },
    })
}

/// What the frame count correction found.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameCountResult {
    pub correction: Option<Correction>,
    pub note: Option<Note>,
    pub common_range: Option<(u64, u64)>,
}

/// Detects a frame count mismatch, and clamps the measurement to the frames both files
/// share, counted from frame 0.
///
/// This always carries a note, because the tool assumes that the shorter file was
/// trimmed at the end, and it cannot tell that case apart from a file trimmed at the
/// start.
pub fn detect_frame_count(
    reference: &MediaInfo,
    encode: &MediaInfo,
    target_label: &str,
) -> FrameCountResult {
    let (Some(reference_frames), Some(encode_frames)) =
        (reference.frame_count(), encode.frame_count())
    else {
        return FrameCountResult {
            correction: None,
            note: None,
            common_range: None,
        };
    };
    if reference_frames == encode_frames {
        return FrameCountResult {
            correction: None,
            note: None,
            common_range: None,
        };
    }

    let last_frame = reference_frames.min(encode_frames).saturating_sub(1);
    let correction = Correction {
        id: CorrectionId::FrameCount,
        target_label: target_label.to_string(),
        message: format!("Measured frame 0 to frame {last_frame}, the range both files share."),
        detail: CorrectionDetail::FrameCount {
            first_frame: 0,
            last_frame,
        },
    };
    let note = Note {
        id: NoteId::FrameCountAlignmentAssumed,
        message: format!(
            "{target_label} reports a different frame count. The tool assumes both files start at the same frame."
        ),
    };
    FrameCountResult {
        correction: Some(correction),
        note: Some(note),
        common_range: Some((0, last_frame)),
    }
}

/// Detects a color range flag that disagrees with the real pixel data.
///
/// A file flagged full range with every sampled luma value inside the limited-range
/// band is either mislabeled or genuinely low-contrast. The tool cannot tell the two
/// cases apart, so it reports this and corrects nothing.
pub fn detect_range_flag_note(
    flagged: &MediaInfo,
    sample: LumaExtremes,
    target_label: &str,
) -> Option<Note> {
    if flagged.color_range != ColorRange::Pc {
        return None;
    }
    if sample.y_min < 16 || sample.y_max > 235 {
        return None;
    }
    Some(Note {
        id: NoteId::RangeFlagDisagreesWithData,
        message: format!(
            "{target_label} is flagged full range, but every sampled luma value sits inside 16 to 235. \
             The flag can be wrong, or the content can be low contrast. The tool used the flag."
        ),
    })
}

/// Detects a near-lossless reference codec.
///
/// Every score is a little pessimistic against a near-lossless reference, because the
/// reference carries a small loss of its own. Ranking between encodes stays correct.
pub fn detect_near_lossless_note(reference: &MediaInfo) -> Option<Note> {
    let codec = reference.codec.to_ascii_lowercase();
    if !NEAR_LOSSLESS_CODECS.iter().any(|name| codec.contains(name)) {
        return None;
    }
    Some(Note {
        id: NoteId::NearLosslessReference,
        message:
            "The reference uses a near-lossless codec. Every score reads a little pessimistic. \
                   Ranking between encodes stays correct."
                .to_string(),
    })
}

/// One known fault for each metric that has one, plus one fault that applies to all of
/// them. Attached to the metric, not to a file, so it appears once for the whole run.
pub fn known_metric_fault_notes(metrics: &BTreeSet<MetricId>) -> Vec<Note> {
    let mut notes = Vec::new();

    if metrics.contains(&MetricId::Ssimulacra2) {
        notes.push(Note {
            id: NoteId::KnownMetricFault,
            message: "SSIMULACRA 2 has no temporal model. It gives full weight to an error in a fast pan."
                .to_string(),
        });
    }
    if metrics.contains(&MetricId::VmafV0) {
        notes.push(Note {
            id: NoteId::KnownMetricFault,
            message: "VMAF v0 cannot see banding, and it is luma only. Prefer VMAF v1.".to_string(),
        });
    }
    if metrics.contains(&MetricId::XpsnrMin) {
        notes.push(Note {
            id: NoteId::KnownMetricFault,
            message: "The 42 dB XPSNR threshold for visually lossless is a community estimate, not a measured number."
                .to_string(),
        });
    }
    if !metrics.is_empty() {
        notes.push(Note {
            id: NoteId::KnownMetricFault,
            message: "Every metric saturates near lossless. Read the 1st percentile, not the mean, for the worst frames."
                .to_string(),
        });
    }

    notes
}

/// Every correction and note that fires for one reference and encode pair, and the
/// frame range that the frame-count correction settled on, when it fired.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DetectedCorrections {
    pub corrections: Vec<Correction>,
    pub notes: Vec<Note>,
    pub frame_range: Option<(u64, u64)>,
}

impl DetectedCorrections {
    /// Every correction and every note, as one flat list of lines, in the interface's
    /// single Notes list. A correction's line starts with its category name. A note's
    /// line does not, since a note already reads as a plain sentence.
    pub fn display_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.corrections.len() + self.notes.len());
        for correction in &self.corrections {
            lines.push(format!("{}: {}", correction.id.label(), correction.message));
        }
        for note in &self.notes {
            lines.push(note.message.clone());
        }
        lines
    }
}

/// Runs every detector for one reference and encode pair.
///
/// The planner and the interface both call this one function, so they can never
/// disagree about what fired.
pub fn detect_all(
    reference: &MediaInfo,
    encode: &MediaInfo,
    target_label: &str,
    sample: Option<LumaExtremes>,
) -> DetectedCorrections {
    let mut result = DetectedCorrections::default();

    if let Some(correction) = detect_color_range(reference, encode, target_label) {
        result.corrections.push(correction);
    }
    if let Some(correction) = detect_resolution(reference, encode, target_label) {
        result.corrections.push(correction);
    }

    let frame_count = detect_frame_count(reference, encode, target_label);
    if let Some(correction) = frame_count.correction {
        result.corrections.push(correction);
    }
    if let Some(note) = frame_count.note {
        result.notes.push(note);
    }
    result.frame_range = frame_count.common_range;

    if let Some(sample) = sample {
        if let Some(note) = detect_range_flag_note(encode, sample, target_label) {
            result.notes.push(note);
        }
    }
    if let Some(note) = detect_near_lossless_note(reference) {
        result.notes.push(note);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Rational;
    use std::path::PathBuf;

    fn media_info(
        width: u32,
        height: u32,
        range: ColorRange,
        pix_fmt: &str,
        codec: &str,
        frames: u64,
    ) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("test.mp4"),
            bytes: 1024,
            codec: codec.to_string(),
            profile: None,
            width,
            height,
            pix_fmt: pix_fmt.to_string(),
            bit_depth: 8,
            color_range: range,
            color_space: Some("bt709".into()),
            frame_rate: Rational { num: 30, den: 1 },
            nb_frames: Some(frames),
            duration_s: Some(frames as f64 / 30.0),
            bit_rate: Some(1_000_000),
        }
    }

    fn identical_pair() -> (MediaInfo, MediaInfo) {
        let one = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let other = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        (one, other)
    }

    #[test]
    fn identical_files_fire_no_correction_and_no_note() {
        let (reference, encode) = identical_pair();
        assert!(detect_color_range(&reference, &encode, "encode.mp4").is_none());
        assert!(detect_resolution(&reference, &encode, "encode.mp4").is_none());
        assert!(
            detect_frame_count(&reference, &encode, "encode.mp4")
                .correction
                .is_none()
        );

        let detected = detect_all(&reference, &encode, "encode.mp4", None);
        assert!(detected.corrections.is_empty());
        assert!(detected.notes.is_empty());
    }

    #[test]
    fn a_color_range_mismatch_converts_the_encode_to_the_reference_range() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Pc, "yuv420p", "h264", 150);
        let correction = detect_color_range(&reference, &encode, "encode.mp4").unwrap();
        assert_eq!(
            correction.detail,
            CorrectionDetail::ColorRange {
                from: ColorRange::Pc,
                to: ColorRange::Tv
            }
        );
    }

    #[test]
    fn a_yuvj_pixel_format_counts_as_full_range_for_the_correction() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Unknown, "yuvj420p", "h264", 150);
        assert!(detect_color_range(&reference, &encode, "encode.mp4").is_some());
    }

    #[test]
    fn a_resolution_mismatch_scales_the_encode_up_to_the_reference() {
        let reference = media_info(3840, 2160, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let correction = detect_resolution(&reference, &encode, "encode.mp4").unwrap();
        assert_eq!(
            correction.detail,
            CorrectionDetail::Resolution {
                pre_scale_width: 1920,
                pre_scale_height: 1080
            }
        );
    }

    #[test]
    fn a_frame_count_mismatch_clamps_to_the_shorter_file_and_carries_a_note() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 140);
        let result = detect_frame_count(&reference, &encode, "encode.mp4");
        assert_eq!(result.common_range, Some((0, 139)));
        assert!(result.correction.is_some());
        assert!(result.note.is_some());
    }

    #[test]
    fn a_low_contrast_full_range_file_gets_the_range_flag_note_and_no_correction() {
        let flagged = media_info(1920, 1080, ColorRange::Pc, "yuv420p", "h264", 150);
        let sample = LumaExtremes {
            sampled_frames: 30,
            y_min: 40,
            y_max: 200,
        };
        let note = detect_range_flag_note(&flagged, sample, "encode.mp4");
        assert!(note.is_some());
    }

    #[test]
    fn a_genuinely_full_range_file_gets_no_range_flag_note() {
        let flagged = media_info(1920, 1080, ColorRange::Pc, "yuv420p", "h264", 150);
        let sample = LumaExtremes {
            sampled_frames: 30,
            y_min: 0,
            y_max: 255,
        };
        assert!(detect_range_flag_note(&flagged, sample, "encode.mp4").is_none());
    }

    #[test]
    fn a_limited_range_file_gets_no_range_flag_note_whatever_its_data_looks_like() {
        let flagged = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let sample = LumaExtremes {
            sampled_frames: 30,
            y_min: 40,
            y_max: 200,
        };
        assert!(detect_range_flag_note(&flagged, sample, "encode.mp4").is_none());
    }

    #[test]
    fn a_prores_reference_gets_the_near_lossless_note() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv444p10le", "prores", 150);
        assert!(detect_near_lossless_note(&reference).is_some());
    }

    #[test]
    fn an_h264_reference_gets_no_near_lossless_note() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        assert!(detect_near_lossless_note(&reference).is_none());
    }

    #[test]
    fn ticking_no_metric_gives_no_known_fault_notes() {
        assert!(known_metric_fault_notes(&BTreeSet::new()).is_empty());
    }

    #[test]
    fn ssimulacra2_carries_its_own_known_fault_note() {
        let metrics = BTreeSet::from([MetricId::Ssimulacra2]);
        let notes = known_metric_fault_notes(&metrics);
        assert!(
            notes
                .iter()
                .any(|note| note.message.contains("temporal model"))
        );
        assert!(
            notes
                .iter()
                .any(|note| note.message.contains("saturates near lossless"))
        );
    }

    #[test]
    fn display_lines_names_the_category_for_a_correction_and_not_for_a_note() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Pc, "yuv420p", "h264", 150);
        let detected = detect_all(&reference, &encode, "encode.mp4", None);

        let lines = detected.display_lines();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("Color range: encode.mp4 is"));
    }

    #[test]
    fn display_lines_puts_every_correction_before_every_note() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "prores", 150);
        let encode = media_info(3840, 2160, ColorRange::Pc, "yuv420p", "h264", 150);
        let detected = detect_all(&reference, &encode, "encode.mp4", None);

        let lines = detected.display_lines();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("Color range:"));
        assert!(lines[1].starts_with("Resolution:"));
        assert!(!lines[2].starts_with("Near-lossless"));
        assert!(lines[2].contains("near-lossless"));
    }
}
