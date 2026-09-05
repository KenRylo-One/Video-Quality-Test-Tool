use crate::media::{ColorRange, LumaExtremes, MediaInfo};
use crate::metric::{MetricGroup, MetricId};
use crate::vmaf_model::{self, VmafModel};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CorrectionId {
    ColorRange,
    Resolution,
    FrameCount,
    CambiEncodeSize,
    VmafModel,
}

impl CorrectionId {
    /// A short category name, shown before the correction's own message.
    pub fn label(self) -> &'static str {
        match self {
            Self::ColorRange => "Color range",
            Self::Resolution => "Resolution",
            Self::FrameCount => "Frame count",
            Self::CambiEncodeSize => "CAMBI encode size",
            Self::VmafModel => "VMAF model",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum NoteId {
    RangeFlagDisagreesWithData,
    NearLosslessReference,
    KnownMetricFault,
    /// The frame count fix assumes both files start at the same frame. It cannot tell
    /// a file trimmed at the end from a file trimmed at the start, so it names the
    /// assumption every time it fires.
    FrameCountAlignmentAssumed,
    /// A Vship metric is ticked, and a correction it needs did not reach it.
    VshipCorrectionNotApplied,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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
    CambiEncodeSize {
        width: u32,
        height: u32,
        bit_depth: u8,
    },
    VmafModel {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Correction {
    pub id: CorrectionId,
    pub target_label: String,
    pub message: String,
    pub detail: CorrectionDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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

/// Metrics that run CAMBI, whether standalone or embedded in a VMAF v1 model.
fn wants_cambi(metrics: &BTreeSet<MetricId>) -> bool {
    metrics.contains(&MetricId::Cambi) || metrics.contains(&MetricId::VmafV1Cambi)
}

/// Metrics that need a VMAF v1 model chosen for them.
fn wants_vmaf_v1_model(metrics: &BTreeSet<MetricId>) -> bool {
    metrics.contains(&MetricId::Vmaf) || metrics.contains(&MetricId::VmafV1Cambi)
}

/// Detects a CAMBI measurement running against a scaled encode, and builds the
/// correction that tells CAMBI the encode's true, pre-scale size.
///
/// CAMBI reads banding from the coded picture. An encode already scaled up to match
/// the reference hides its own banding pattern unless CAMBI is told the real size it
/// was coded at. This only fires once the resolution correction has already fired,
/// since only then was the encode actually scaled.
pub fn detect_cambi_encode_size(
    metrics: &BTreeSet<MetricId>,
    resolution: Option<&Correction>,
    encode: &MediaInfo,
    target_label: &str,
) -> Option<Correction> {
    if !wants_cambi(metrics) {
        return None;
    }
    let CorrectionDetail::Resolution {
        pre_scale_width,
        pre_scale_height,
    } = resolution?.detail
    else {
        return None;
    };
    Some(Correction {
        id: CorrectionId::CambiEncodeSize,
        target_label: target_label.to_string(),
        message: format!(
            "{target_label}: CAMBI measured the true encode size, {pre_scale_width}x{pre_scale_height} at {} bit, from before the scale to the reference.",
            encode.bit_depth
        ),
        detail: CorrectionDetail::CambiEncodeSize {
            width: pre_scale_width,
            height: pre_scale_height,
            bit_depth: encode.bit_depth,
        },
    })
}

/// Detects a VMAF v1 measurement, and chooses the model for it.
///
/// The measurement resolution is the reference's own resolution, since the encode is
/// always scaled up to match it. The tool never lets a v1 model be chosen by matching
/// its file name. `models` must already hold only the models for the reference's
/// frame rate bracket, since the standard and the high-frame-rate models live in two
/// separate folders with no field of their own to tell them apart.
pub fn detect_vmaf_model(
    metrics: &BTreeSet<MetricId>,
    models: &[VmafModel],
    reference: &MediaInfo,
    viewing_distance: f32,
    target_label: &str,
) -> Option<Correction> {
    if !wants_vmaf_v1_model(metrics) {
        return None;
    }
    let chosen = vmaf_model::choose_model(models, reference.height, viewing_distance)?;
    let file_name = chosen
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(Correction {
        id: CorrectionId::VmafModel,
        target_label: target_label.to_string(),
        message: format!(
            "{target_label}: chose {file_name} for {} fps at {}p. Viewing distance {:.1}, reference display height {}.",
            reference.frame_rate.label(),
            reference.height,
            chosen.normalized_viewing_distance,
            chosen.reference_display_height
        ),
        detail: CorrectionDetail::VmafModel {
            path: chosen.path.clone(),
        },
    })
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
/// disagree about what fired. `metrics` is the set that will actually run, since C4
/// and C5 only matter when a CAMBI or a VMAF v1 metric is ticked. `vmaf_models` must
/// already hold only the models for the reference's frame rate bracket.
pub fn detect_all(
    reference: &MediaInfo,
    encode: &MediaInfo,
    target_label: &str,
    sample: Option<LumaExtremes>,
    metrics: &BTreeSet<MetricId>,
    vmaf_models: &[VmafModel],
    vmaf_viewing_distance: f32,
) -> DetectedCorrections {
    let mut result = DetectedCorrections::default();

    if let Some(correction) = detect_color_range(reference, encode, target_label) {
        result.corrections.push(correction);
    }
    let resolution = detect_resolution(reference, encode, target_label);
    if let Some(correction) =
        detect_cambi_encode_size(metrics, resolution.as_ref(), encode, target_label)
    {
        result.corrections.push(correction);
    }
    if let Some(correction) = resolution {
        result.corrections.push(correction);
    }
    if let Some(correction) = detect_vmaf_model(
        metrics,
        vmaf_models,
        reference,
        vmaf_viewing_distance,
        target_label,
    ) {
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

/// Corrections a Vship metric needs but cannot get. FFVship reads the container
/// itself, with no filter chain in front of it, so only the frame-count clamp reaches
/// it, through `--start`/`--end`. Fires only when a Vship metric is ticked, and only
/// for a correction this pair of files actually triggered.
pub fn detect_vship_gap_notes(
    metrics: &BTreeSet<MetricId>,
    corrections: &[Correction],
) -> Vec<Note> {
    if !metrics
        .iter()
        .any(|id| id.def().group == MetricGroup::Ffvship)
    {
        return Vec::new();
    }
    corrections
        .iter()
        .filter(|correction| {
            matches!(
                correction.id,
                CorrectionId::ColorRange | CorrectionId::Resolution
            )
        })
        .map(|correction| Note {
            id: NoteId::VshipCorrectionNotApplied,
            message: format!(
                "{}: FFVship measured the files as delivered. The {} correction did not reach it.",
                correction.target_label,
                correction.id.label().to_ascii_lowercase()
            ),
        })
        .collect()
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

        let detected = detect_all(
            &reference,
            &encode,
            "encode.mp4",
            None,
            &BTreeSet::new(),
            &[],
            3.0,
        );
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
        let detected = detect_all(
            &reference,
            &encode,
            "encode.mp4",
            None,
            &BTreeSet::new(),
            &[],
            3.0,
        );

        let lines = detected.display_lines();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("Color range: encode.mp4 is"));
    }

    #[test]
    fn display_lines_puts_every_correction_before_every_note() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "prores", 150);
        let encode = media_info(3840, 2160, ColorRange::Pc, "yuv420p", "h264", 150);
        let detected = detect_all(
            &reference,
            &encode,
            "encode.mp4",
            None,
            &BTreeSet::new(),
            &[],
            3.0,
        );

        let lines = detected.display_lines();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("Color range:"));
        assert!(lines[1].starts_with("Resolution:"));
        assert!(!lines[2].starts_with("Near-lossless"));
        assert!(lines[2].contains("near-lossless"));
    }

    #[test]
    fn a_ticked_vship_metric_with_a_color_range_mismatch_gets_a_gap_note() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Pc, "yuv420p", "h264", 150);
        let metrics = BTreeSet::from([MetricId::Ssimulacra2]);
        let detected = detect_all(&reference, &encode, "encode.mp4", None, &metrics, &[], 3.0);

        let notes = detect_vship_gap_notes(&metrics, &detected.corrections);
        assert!(
            notes
                .iter()
                .any(|note| note.message.contains("color range"))
        );
    }

    #[test]
    fn a_ticked_vship_metric_with_a_resolution_mismatch_gets_a_gap_note() {
        let reference = media_info(3840, 2160, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let metrics = BTreeSet::from([MetricId::Cvvdp]);
        let detected = detect_all(&reference, &encode, "encode.mp4", None, &metrics, &[], 3.0);

        let notes = detect_vship_gap_notes(&metrics, &detected.corrections);
        assert!(notes.iter().any(|note| note.message.contains("resolution")));
    }

    #[test]
    fn identical_files_give_no_gap_note_even_with_a_vship_metric_ticked() {
        let (reference, encode) = identical_pair();
        let metrics = BTreeSet::from([MetricId::Ssimulacra2]);
        let detected = detect_all(&reference, &encode, "encode.mp4", None, &metrics, &[], 3.0);

        assert!(detect_vship_gap_notes(&metrics, &detected.corrections).is_empty());
    }

    #[test]
    fn an_ffmpeg_only_job_gets_no_gap_note_even_with_a_real_mismatch() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Pc, "yuv420p", "h264", 150);
        let metrics = BTreeSet::from([MetricId::PsnrY]);
        let detected = detect_all(&reference, &encode, "encode.mp4", None, &metrics, &[], 3.0);

        assert!(!detected.corrections.is_empty());
        assert!(detect_vship_gap_notes(&metrics, &detected.corrections).is_empty());
    }

    #[test]
    fn a_frame_count_mismatch_is_never_a_gap_note() {
        let reference = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 150);
        let encode = media_info(1920, 1080, ColorRange::Tv, "yuv420p", "h264", 140);
        let metrics = BTreeSet::from([MetricId::Ssimulacra2]);
        let detected = detect_all(&reference, &encode, "encode.mp4", None, &metrics, &[], 3.0);

        assert!(!detected.corrections.is_empty());
        assert!(detect_vship_gap_notes(&metrics, &detected.corrections).is_empty());
    }
}
