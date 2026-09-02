use std::ffi::OsString;
use std::path::PathBuf;
use vqa_core::backend::{Invocation, LogArtifact, LogFormat, MeasureJob};
use vqa_core::capability::LaneKind;
use vqa_core::corrections::{self, CorrectionId, DetectedCorrections};
use vqa_core::metric::MetricId;

const FFMPEG_FAMILY: [MetricId; 3] = [MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin];

/// Which corrections `plan()` is allowed to apply to the filter graph.
///
/// For tests only. `plan()` always uses `CorrectionToggles::default()`. No setting, no
/// preset and no interface control ever builds one of these by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionToggles {
    pub color_range: bool,
    pub resolution: bool,
    pub frame_count: bool,
}

impl Default for CorrectionToggles {
    fn default() -> Self {
        Self {
            color_range: true,
            resolution: true,
            frame_count: true,
        }
    }
}

pub fn plan(job: &MeasureJob) -> vqa_core::Result<Vec<Invocation>> {
    plan_with_toggles(job, CorrectionToggles::default())
}

/// For tests only. See `CorrectionToggles`.
pub fn plan_with_toggles(
    job: &MeasureJob,
    toggles: CorrectionToggles,
) -> vqa_core::Result<Vec<Invocation>> {
    let metrics: Vec<MetricId> = FFMPEG_FAMILY
        .iter()
        .copied()
        .filter(|metric| job.metrics.contains(metric))
        .collect();

    if metrics.is_empty() {
        return Ok(Vec::new());
    }

    let target_label = job.encode.info.file_name();
    let mut detected =
        corrections::detect_all(&job.reference.info, &job.encode.info, &target_label, None);
    apply_toggles(&mut detected, toggles);
    let frame_range = job.frame_range.or(detected.frame_range);

    if job.fused_passes {
        Ok(vec![fused(job, &metrics, &detected, frame_range)])
    } else {
        Ok(metrics
            .iter()
            .map(|metric| separate(job, *metric, &detected, frame_range))
            .collect())
    }
}

/// Drops a detected correction that a test asked to disable, and drops the frame range
/// that came from it, so the toggle really turns the correction off.
fn apply_toggles(detected: &mut DetectedCorrections, toggles: CorrectionToggles) {
    if !toggles.color_range {
        detected
            .corrections
            .retain(|correction| correction.id != CorrectionId::ColorRange);
    }
    if !toggles.resolution {
        detected
            .corrections
            .retain(|correction| correction.id != CorrectionId::Resolution);
    }
    if !toggles.frame_count {
        detected
            .corrections
            .retain(|correction| correction.id != CorrectionId::FrameCount);
        detected.frame_range = None;
    }
}

fn separate(
    job: &MeasureJob,
    metric: MetricId,
    detected: &DetectedCorrections,
    frame_range: Option<(u64, u64)>,
) -> Invocation {
    let stats_path = job.work_dir.join(log_file_name(metric));
    let format = log_format_of(metric);

    let distorted_pad = "distorted_pad";
    let reference_pad = "reference_pad";
    let reset_distorted = format!(
        "[0:v]{}[{distorted_pad}]",
        distorted_chain(job, detected, frame_range)
    );
    let reset_reference = format!("[1:v]{}[{reference_pad}]", reference_chain(frame_range));
    let branch = filter_branch(metric, distorted_pad, reference_pad, &stats_path);

    let filter_graph = if metric == MetricId::XpsnrMin {
        format!("{reset_reference};{reset_distorted};{branch}")
    } else {
        format!("{reset_distorted};{reset_reference};{branch}")
    };

    Invocation {
        program: PathBuf::from("ffmpeg"),
        args: ffmpeg_args(&job.encode.path, &job.reference.path, &filter_graph),
        env: Vec::new(),
        cwd: None,
        expects: vec![LogArtifact {
            path: stats_path,
            format,
            metrics: vec![metric],
        }],
        lane: LaneKind::Cpu,
    }
}

fn fused(
    job: &MeasureJob,
    metrics: &[MetricId],
    detected: &DetectedCorrections,
    frame_range: Option<(u64, u64)>,
) -> Invocation {
    let count = metrics.len();
    let distorted_pads: Vec<String> = (0..count)
        .map(|index| format!("distorted{index}"))
        .collect();
    let reference_pads: Vec<String> = (0..count)
        .map(|index| format!("reference{index}"))
        .collect();

    let mut filter_graph = format!(
        "[0:v]{},split={count}[{}];[1:v]{},split={count}[{}]",
        distorted_chain(job, detected, frame_range),
        distorted_pads.join("]["),
        reference_chain(frame_range),
        reference_pads.join("]["),
    );

    let mut expects = Vec::with_capacity(count);
    for (index, metric) in metrics.iter().enumerate() {
        let stats_path = job.work_dir.join(log_file_name(*metric));
        filter_graph.push(';');
        filter_graph.push_str(&filter_branch(
            *metric,
            &distorted_pads[index],
            &reference_pads[index],
            &stats_path,
        ));
        expects.push(LogArtifact {
            path: stats_path,
            format: log_format_of(*metric),
            metrics: vec![*metric],
        });
    }

    Invocation {
        program: PathBuf::from("ffmpeg"),
        args: ffmpeg_args(&job.encode.path, &job.reference.path, &filter_graph),
        env: Vec::new(),
        cwd: None,
        expects,
        lane: LaneKind::Cpu,
    }
}

/// The filter chain for the distorted input: an optional trim, always a timestamp
/// reset, then an optional scale and an optional range conversion, in that order.
/// Verified against the real `ffmpeg` on this machine, including the combined chain.
fn distorted_chain(
    job: &MeasureJob,
    detected: &DetectedCorrections,
    frame_range: Option<(u64, u64)>,
) -> String {
    let mut chain = trim_clause(frame_range);
    chain.push_str("setpts=PTS-STARTPTS");

    if detected
        .corrections
        .iter()
        .any(|correction| correction.id == CorrectionId::Resolution)
    {
        chain.push_str(&format!(
            ",scale={}:{}:flags=bicubic",
            job.reference.info.width, job.reference.info.height
        ));
    }

    if detected
        .corrections
        .iter()
        .any(|correction| correction.id == CorrectionId::ColorRange)
    {
        let from = job
            .encode
            .info
            .effective_color_range()
            .ffmpeg_value()
            .unwrap_or("full");
        let to = job
            .reference
            .info
            .effective_color_range()
            .ffmpeg_value()
            .unwrap_or("limited");
        chain.push_str(&format!(
            ",zscale=in_range={from}:out_range={to},format=yuv420p"
        ));
    }

    chain
}

/// The filter chain for the reference input. The reference is never scaled and never
/// range-converted.
fn reference_chain(frame_range: Option<(u64, u64)>) -> String {
    let mut chain = trim_clause(frame_range);
    chain.push_str("setpts=PTS-STARTPTS");
    chain
}

/// `trim=start_frame=X:end_frame=Y,` for a range, or an empty string for the whole file.
/// `end_frame` is the first dropped frame, so an inclusive last frame becomes `last + 1`.
fn trim_clause(frame_range: Option<(u64, u64)>) -> String {
    match frame_range {
        Some((first_frame, last_frame)) => format!(
            "trim=start_frame={first_frame}:end_frame={},",
            last_frame + 1
        ),
        None => String::new(),
    }
}

fn filter_branch(
    metric: MetricId,
    distorted_pad: &str,
    reference_pad: &str,
    stats_path: &std::path::Path,
) -> String {
    let filter_name = ffmpeg_filter_name(metric);
    let stats_path = stats_path.to_string_lossy();
    if metric == MetricId::XpsnrMin {
        format!("[{reference_pad}][{distorted_pad}]{filter_name}=stats_file={stats_path}")
    } else {
        format!("[{distorted_pad}][{reference_pad}]{filter_name}=stats_file={stats_path}")
    }
}

fn ffmpeg_args(
    encode_path: &std::path::Path,
    reference_path: &std::path::Path,
    filter_graph: &str,
) -> Vec<OsString> {
    vec![
        OsString::from("-y"),
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-i"),
        encode_path.as_os_str().to_os_string(),
        OsString::from("-i"),
        reference_path.as_os_str().to_os_string(),
        OsString::from("-lavfi"),
        OsString::from(filter_graph),
        OsString::from("-f"),
        OsString::from("null"),
        OsString::from("-"),
    ]
}

fn ffmpeg_filter_name(metric: MetricId) -> &'static str {
    match metric {
        MetricId::PsnrY => "psnr",
        MetricId::SsimAll => "ssim",
        MetricId::XpsnrMin => "xpsnr",
        _ => unreachable!("only the FFmpeg family reaches this function"),
    }
}

fn log_file_name(metric: MetricId) -> &'static str {
    match metric {
        MetricId::PsnrY => "psnr.log",
        MetricId::SsimAll => "ssim.log",
        MetricId::XpsnrMin => "xpsnr.log",
        _ => unreachable!("only the FFmpeg family reaches this function"),
    }
}

fn log_format_of(metric: MetricId) -> LogFormat {
    match metric {
        MetricId::PsnrY => LogFormat::PsnrStats,
        MetricId::SsimAll => LogFormat::SsimStats,
        MetricId::XpsnrMin => LogFormat::XpsnrStats,
        _ => unreachable!("only the FFmpeg family reaches this function"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use vqa_core::backend::JobInput;
    use vqa_core::media::{ColorRange, MediaInfo, Rational};

    fn media_info(
        name: &str,
        width: u32,
        height: u32,
        range: ColorRange,
        frames: u64,
    ) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from(name),
            bytes: 0,
            codec: "h264".into(),
            profile: None,
            width,
            height,
            pix_fmt: "yuv420p".into(),
            bit_depth: 8,
            color_range: range,
            color_space: Some("bt709".into()),
            frame_rate: Rational { num: 30, den: 1 },
            nb_frames: Some(frames),
            duration_s: Some(frames as f64 / 30.0),
            bit_rate: Some(1_000_000),
        }
    }

    fn job_with(
        metrics: &[MetricId],
        fused_passes: bool,
        reference: MediaInfo,
        encode: MediaInfo,
    ) -> MeasureJob {
        MeasureJob {
            reference: JobInput {
                path: PathBuf::from("reference.mkv"),
                info: reference,
            },
            encode: JobInput {
                path: PathBuf::from("distorted.mkv"),
                info: encode,
            },
            metrics: metrics.iter().copied().collect::<BTreeSet<_>>(),
            frame_range: None,
            fused_passes,
            work_dir: PathBuf::from("work"),
        }
    }

    fn identical_job(metrics: &[MetricId], fused_passes: bool) -> MeasureJob {
        let info = media_info("clip.mkv", 1920, 1080, ColorRange::Tv, 150);
        job_with(metrics, fused_passes, info.clone(), info)
    }

    fn arg_strings(invocation: &Invocation) -> Vec<String> {
        invocation
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn no_ffmpeg_family_metric_gives_no_invocation() {
        let job = identical_job(&[MetricId::Vmaf], false);
        assert!(plan(&job).unwrap().is_empty());
    }

    #[test]
    fn xpsnr_takes_the_reference_first_and_psnr_takes_the_distorted_first() {
        let job = identical_job(&[MetricId::PsnrY, MetricId::XpsnrMin], false);
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 2);

        assert_eq!(
            arg_strings(&invocations[1]),
            vec![
                "-y",
                "-v",
                "error",
                "-i",
                "distorted.mkv",
                "-i",
                "reference.mkv",
                "-lavfi",
                "[1:v]setpts=PTS-STARTPTS[reference_pad];[0:v]setpts=PTS-STARTPTS[distorted_pad];[reference_pad][distorted_pad]xpsnr=stats_file=work/xpsnr.log",
                "-f",
                "null",
                "-",
            ]
        );
        assert!(
            arg_strings(&invocations[0])
                .iter()
                .any(|arg| arg.contains("[distorted_pad][reference_pad]psnr=stats_file="))
        );
    }

    #[test]
    fn identical_media_info_fires_no_correction() {
        let job = identical_job(&[MetricId::PsnrY], false);
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();
        assert!(filter_graph.contains("[0:v]setpts=PTS-STARTPTS[distorted_pad]"));
        assert!(!filter_graph.contains("scale="));
        assert!(!filter_graph.contains("zscale="));
        assert!(!filter_graph.contains("trim="));
    }

    #[test]
    fn a_color_range_mismatch_adds_zscale_to_the_distorted_branch_only() {
        let reference = media_info("reference.mkv", 1920, 1080, ColorRange::Tv, 150);
        let encode = media_info("distorted.mkv", 1920, 1080, ColorRange::Pc, 150);
        let job = job_with(&[MetricId::PsnrY], false, reference, encode);
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(filter_graph.contains("[0:v]setpts=PTS-STARTPTS,zscale=in_range=full:out_range=limited,format=yuv420p[distorted_pad]"));
        assert!(filter_graph.contains("[1:v]setpts=PTS-STARTPTS[reference_pad]"));
    }

    #[test]
    fn a_resolution_mismatch_scales_the_distorted_branch_to_the_reference_size() {
        let reference = media_info("reference.mkv", 3840, 2160, ColorRange::Tv, 150);
        let encode = media_info("distorted.mkv", 1920, 1080, ColorRange::Tv, 150);
        let job = job_with(&[MetricId::PsnrY], false, reference, encode);
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(
            filter_graph
                .contains("[0:v]setpts=PTS-STARTPTS,scale=3840:2160:flags=bicubic[distorted_pad]")
        );
        assert!(filter_graph.contains("[1:v]setpts=PTS-STARTPTS[reference_pad]"));
    }

    #[test]
    fn a_frame_count_mismatch_trims_both_branches_to_the_same_end_frame() {
        let reference = media_info("reference.mkv", 1920, 1080, ColorRange::Tv, 150);
        let encode = media_info("distorted.mkv", 1920, 1080, ColorRange::Tv, 140);
        let job = job_with(&[MetricId::PsnrY], false, reference, encode);
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(
            filter_graph.contains(
                "[0:v]trim=start_frame=0:end_frame=140,setpts=PTS-STARTPTS[distorted_pad]"
            )
        );
        assert!(
            filter_graph.contains(
                "[1:v]trim=start_frame=0:end_frame=140,setpts=PTS-STARTPTS[reference_pad]"
            )
        );
    }

    #[test]
    fn a_manual_frame_range_wins_over_the_automatic_clamp() {
        let reference = media_info("reference.mkv", 1920, 1080, ColorRange::Tv, 150);
        let encode = media_info("distorted.mkv", 1920, 1080, ColorRange::Tv, 140);
        let mut job = job_with(&[MetricId::PsnrY], false, reference, encode);
        job.frame_range = Some((10, 49));
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(filter_graph.contains("trim=start_frame=10:end_frame=50"));
    }

    #[test]
    fn disabling_the_color_range_toggle_removes_the_zscale_filter() {
        let reference = media_info("reference.mkv", 1920, 1080, ColorRange::Tv, 150);
        let encode = media_info("distorted.mkv", 1920, 1080, ColorRange::Pc, 150);
        let job = job_with(&[MetricId::PsnrY], false, reference, encode);

        let toggles = CorrectionToggles {
            color_range: false,
            ..CorrectionToggles::default()
        };
        let invocation = &plan_with_toggles(&job, toggles).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();
        assert!(!filter_graph.contains("zscale="));
    }

    #[test]
    fn a_fused_pass_applies_the_same_corrections_to_every_branch() {
        let reference = media_info("reference.mkv", 3840, 2160, ColorRange::Tv, 150);
        let encode = media_info("distorted.mkv", 1920, 1080, ColorRange::Pc, 150);
        let job = job_with(
            &[MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin],
            true,
            reference,
            encode,
        );
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(filter_graph.starts_with(
            "[0:v]setpts=PTS-STARTPTS,scale=3840:2160:flags=bicubic,zscale=in_range=full:out_range=limited,format=yuv420p,split=3"
        ));
    }
}
