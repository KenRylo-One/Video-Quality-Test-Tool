use std::ffi::OsString;
use std::path::PathBuf;
use vqa_core::backend::{Invocation, LogArtifact, LogFormat, MeasureJob};
use vqa_core::capability::LaneKind;
use vqa_core::corrections::{self, CorrectionDetail, CorrectionId, DetectedCorrections};
use vqa_core::metric::MetricId;

const FFMPEG_FAMILY: [MetricId; 3] = [MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin];

/// The metrics that ride on a VMAF v1 model, fused into the same pass as the model
/// they need chosen for them.
const VMAF_V1_GROUP: [MetricId; 2] = [MetricId::Vmaf, MetricId::VmafV1Cambi];

/// Metrics that attach as an extra `feature=` clause onto whichever base pass runs, or
/// get their own pass with the default model when no base pass is ticked. Each one
/// names its own `libvmaf` feature key.
const LIBVMAF_EXTRA_FEATURES: [(MetricId, &str); 4] = [
    (MetricId::Cambi, "cambi"),
    (MetricId::PsnrHvs, "psnr_hvs"),
    (MetricId::Ciede2000, "ciede"),
    (MetricId::MsSsim, "float_ms_ssim"),
];

/// Every metric this file can build a `libvmaf` invocation for.
const LIBVMAF_FAMILY: [MetricId; 8] = [
    MetricId::Vmaf,
    MetricId::VmafV0,
    MetricId::VmafNegV0,
    MetricId::Cambi,
    MetricId::VmafV1Cambi,
    MetricId::PsnrHvs,
    MetricId::Ciede2000,
    MetricId::MsSsim,
];

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
    let libvmaf_metrics: Vec<MetricId> = LIBVMAF_FAMILY
        .iter()
        .copied()
        .filter(|metric| job.metrics.contains(metric))
        .collect();

    if metrics.is_empty() && libvmaf_metrics.is_empty() {
        return Ok(Vec::new());
    }

    let target_label = job.encode.info.file_name();
    let mut detected = corrections::detect_all(
        &job.reference.info,
        &job.encode.info,
        &target_label,
        None,
        &job.metrics,
        &job.vmaf_models,
        job.vmaf_viewing_distance,
    );
    apply_toggles(&mut detected, toggles);
    let frame_range = job.frame_range.or(detected.frame_range);

    let mut invocations = Vec::new();

    if !metrics.is_empty() {
        if job.fused_passes {
            invocations.push(fused(job, &metrics, &detected, frame_range));
        } else {
            invocations.extend(
                metrics
                    .iter()
                    .map(|metric| separate(job, *metric, &detected, frame_range)),
            );
        }
    }

    invocations.extend(libvmaf_invocations(
        job,
        &libvmaf_metrics,
        &detected,
        frame_range,
    ));

    Ok(invocations)
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

/// Metrics that can safely ride as a `feature=` extra on a pass that already runs a
/// VMAF v1 model. Standalone CAMBI is deliberately excluded: a v1 model already
/// computes its own embedded CAMBI at a different speedup, and the two must never
/// share a graph or a CSV column.
const V1_SAFE_EXTRA_FEATURES: [MetricId; 3] =
    [MetricId::PsnrHvs, MetricId::Ciede2000, MetricId::MsSsim];

/// Builds one `libvmaf` invocation for each group of metrics that cannot share a
/// pass with another group. A VMAF v1 score and its embedded CAMBI number always
/// share a model, so they share a pass. `VmafV0` and `VmafNegV0` each need their own
/// model options, so each gets its own pass. Standalone CAMBI always gets its own
/// pass, since a v1 model run already carries a different CAMBI number of its own.
/// Every remaining feature attaches to the v1 pass when it runs, or gets one pass of
/// its own with the FFmpeg default model when it does not.
fn libvmaf_invocations(
    job: &MeasureJob,
    ticked: &[MetricId],
    detected: &DetectedCorrections,
    frame_range: Option<(u64, u64)>,
) -> Vec<Invocation> {
    let mut invocations = Vec::new();

    let v1_metrics: Vec<MetricId> = VMAF_V1_GROUP
        .iter()
        .copied()
        .filter(|id| ticked.contains(id))
        .collect();
    let mut v1_safe_extras: Vec<MetricId> = V1_SAFE_EXTRA_FEATURES
        .iter()
        .copied()
        .filter(|id| ticked.contains(id))
        .collect();

    if !v1_metrics.is_empty() {
        if let Some(invocation) = libvmaf_pass(
            job,
            &v1_metrics,
            &std::mem::take(&mut v1_safe_extras),
            detected,
            frame_range,
            "vmaf_v1",
        ) {
            invocations.push(invocation);
        }
    }

    if ticked.contains(&MetricId::VmafV0) {
        if let Some(invocation) = libvmaf_pass(
            job,
            &[MetricId::VmafV0],
            &[],
            detected,
            frame_range,
            "vmaf_v0",
        ) {
            invocations.push(invocation);
        }
    }

    if ticked.contains(&MetricId::VmafNegV0) {
        if let Some(invocation) = libvmaf_pass(
            job,
            &[MetricId::VmafNegV0],
            &[],
            detected,
            frame_range,
            "vmaf_neg_v0",
        ) {
            invocations.push(invocation);
        }
    }

    if ticked.contains(&MetricId::Cambi) {
        if let Some(invocation) =
            libvmaf_pass(job, &[], &[MetricId::Cambi], detected, frame_range, "cambi")
        {
            invocations.push(invocation);
        }
    }

    // v1_safe_extras is left over when no v1 pass ran to carry it.
    if !v1_safe_extras.is_empty() {
        if let Some(invocation) = libvmaf_pass(
            job,
            &[],
            &v1_safe_extras,
            detected,
            frame_range,
            "vmaf_extra",
        ) {
            invocations.push(invocation);
        }
    }

    invocations
}

/// One `libvmaf` pass. `primary` names the metric or metrics that need `model=` set,
/// at most the two members of `VMAF_V1_GROUP`, since every other primary metric runs
/// alone. `extras` names features that attach as a `feature=` clause with no model
/// selection of their own. Returns nothing when `primary` asks for a VMAF v1 model
/// and none was found, since the tool never blocks a run over a missing model. It
/// drops that metric instead.
fn libvmaf_pass(
    job: &MeasureJob,
    primary: &[MetricId],
    extras: &[MetricId],
    detected: &DetectedCorrections,
    frame_range: Option<(u64, u64)>,
    file_stem: &str,
) -> Option<Invocation> {
    let wants_v1 = primary.iter().any(|id| VMAF_V1_GROUP.contains(id));
    let mut cwd = None;
    let mut model_option = String::new();

    if wants_v1 {
        let model_path = detected.corrections.iter().find_map(|correction| {
            if correction.id != CorrectionId::VmafModel {
                return None;
            }
            match &correction.detail {
                CorrectionDetail::VmafModel { path } => Some(path.clone()),
                _ => None,
            }
        })?;
        let file_name = model_path.file_name()?.to_string_lossy().into_owned();
        cwd = model_path.parent().map(|parent| parent.to_path_buf());
        model_option = format!(":model=path='{file_name}'");
    } else if primary.contains(&MetricId::VmafNegV0) {
        // Verified against the real `ffmpeg` on this machine: a `feature=` override of
        // `enhn_gain_limit` is rejected as an unknown top-level option, because the
        // colon inside its value collides with the filter's own colon-separated option
        // list. FFmpeg's own bundled NEG model avoids the whole problem.
        model_option = ":model=version=vmaf_v0.6.1neg".to_string();
    } else if primary.contains(&MetricId::VmafV0) {
        model_option = ":model=version=vmaf_v0.6.1".to_string();
    }

    let cambi_size_option = detected
        .corrections
        .iter()
        .find_map(|correction| match &correction.detail {
            CorrectionDetail::CambiEncodeSize {
                width,
                height,
                bit_depth,
            } => Some(format!(
                ":cambi.enc_width={width}:cambi.enc_height={height}:cambi.enc_bitdepth={bit_depth}"
            )),
            _ => None,
        })
        .unwrap_or_default();

    let extra_names: Vec<&str> = LIBVMAF_EXTRA_FEATURES
        .iter()
        .filter(|(id, _)| extras.contains(id))
        .map(|(_, key)| *key)
        .collect();
    let extra_feature_option = if extra_names.is_empty() {
        String::new()
    } else {
        format!(
            ":feature={}",
            extra_names
                .iter()
                .map(|name| format!("name={name}"))
                .collect::<Vec<_>>()
                .join("|")
        )
    };

    let needs_10bit =
        wants_v1 || primary.contains(&MetricId::VmafV1Cambi) || extras.contains(&MetricId::Cambi);

    let stats_path = job.work_dir.join(format!("{file_stem}.csv"));
    let distorted_pad = "distorted_pad";
    let reference_pad = "reference_pad";
    let bit_depth_clause = if needs_10bit {
        ",format=yuv420p10le"
    } else {
        ""
    };
    let filter_graph = format!(
        "[0:v]{}{bit_depth_clause}[{distorted_pad}];[1:v]{}{bit_depth_clause}[{reference_pad}];[{distorted_pad}][{reference_pad}]libvmaf=log_fmt=csv:log_path={}{model_option}{cambi_size_option}{extra_feature_option}",
        distorted_chain(job, detected, frame_range),
        reference_chain(frame_range),
        stats_path.to_string_lossy(),
    );

    let mut metrics: Vec<MetricId> = primary.to_vec();
    metrics.extend(extras.iter().copied());

    Some(Invocation {
        program: PathBuf::from("ffmpeg"),
        args: ffmpeg_args(&job.encode.path, &job.reference.path, &filter_graph),
        env: Vec::new(),
        cwd,
        expects: vec![LogArtifact {
            path: stats_path,
            format: LogFormat::VmafCsv,
            metrics,
        }],
        lane: LaneKind::Cpu,
    })
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
            vmaf_models: Vec::new(),
            vmaf_viewing_distance: 3.0,
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

    fn v1_model(folder: &str, name: &str) -> vqa_core::vmaf_model::VmafModel {
        vqa_core::vmaf_model::VmafModel {
            path: PathBuf::from(folder).join(name),
            is_v1: true,
            reference_display_height: 1080,
            normalized_viewing_distance: 3.0,
        }
    }

    #[test]
    fn a_vmaf_v1_pass_reads_the_distorted_input_first_like_psnr() {
        let info = media_info("clip.mkv", 1920, 1080, ColorRange::Tv, 150);
        let mut job = job_with(&[MetricId::Vmaf], false, info.clone(), info);
        job.vmaf_models = vec![v1_model("model/vmaf_v1.0.16", "vmaf_v1.0.16_3d0h.json")];
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(
            filter_graph.starts_with("[0:v]setpts=PTS-STARTPTS,format=yuv420p10le[distorted_pad]")
        );
        assert!(
            filter_graph.contains("[1:v]setpts=PTS-STARTPTS,format=yuv420p10le[reference_pad]")
        );
        assert!(
            filter_graph.contains("[distorted_pad][reference_pad]libvmaf="),
            "libvmaf must read the distorted pad first, like psnr and ssim: {filter_graph}"
        );
    }

    #[test]
    fn a_chosen_vmaf_v1_model_sets_a_bare_file_name_and_a_working_directory() {
        let info = media_info("clip.mkv", 1920, 1080, ColorRange::Tv, 150);
        let mut job = job_with(&[MetricId::Vmaf], false, info.clone(), info);
        job.vmaf_models = vec![v1_model("model/vmaf_v1.0.16", "vmaf_v1.0.16_3d0h.json")];
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(
            filter_graph.contains("model=path='vmaf_v1.0.16_3d0h.json'"),
            "the option string must carry no folder, so a Windows drive letter never reaches it: {filter_graph}"
        );
        assert!(
            !filter_graph.contains("model/vmaf_v1.0.16"),
            "the folder belongs in cwd, never in the option string: {filter_graph}"
        );
        assert_eq!(invocation.cwd, Some(PathBuf::from("model/vmaf_v1.0.16")));
    }

    #[test]
    fn no_vmaf_v1_model_found_gives_no_invocation_for_that_pass() {
        let job = identical_job(&[MetricId::Vmaf], false);
        assert!(plan(&job).unwrap().is_empty());
    }

    #[test]
    fn cambi_in_a_v1_model_and_standalone_cambi_never_share_a_pass() {
        let info = media_info("clip.mkv", 1920, 1080, ColorRange::Tv, 150);
        let mut job = job_with(
            &[MetricId::VmafV1Cambi, MetricId::Cambi],
            false,
            info.clone(),
            info,
        );
        job.vmaf_models = vec![v1_model("model/vmaf_v1.0.16", "vmaf_v1.0.16_3d0h.json")];
        let invocations = plan(&job).unwrap();

        assert_eq!(invocations.len(), 2);
        let metric_sets: Vec<Vec<MetricId>> = invocations
            .iter()
            .map(|invocation| {
                invocation
                    .expects
                    .iter()
                    .flat_map(|artifact| artifact.metrics.clone())
                    .collect()
            })
            .collect();
        assert!(metric_sets.contains(&vec![MetricId::VmafV1Cambi]));
        assert!(metric_sets.contains(&vec![MetricId::Cambi]));
    }

    #[test]
    fn cambi_ticked_against_a_scaled_encode_adds_the_true_encode_size() {
        let reference = media_info("reference.mkv", 3840, 2160, ColorRange::Tv, 150);
        let encode = media_info("distorted.mkv", 1920, 1080, ColorRange::Tv, 150);
        let job = job_with(&[MetricId::Cambi], false, reference, encode);
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(
            filter_graph
                .contains("cambi.enc_width=1920:cambi.enc_height=1080:cambi.enc_bitdepth=8")
        );
    }

    #[test]
    fn cambi_ticked_against_an_unscaled_encode_adds_no_encode_size_option() {
        let job = identical_job(&[MetricId::Cambi], false);
        let invocation = &plan(&job).unwrap()[0];
        let filter_graph = arg_strings(invocation).into_iter().nth(8).unwrap();

        assert!(!filter_graph.contains("cambi.enc_width"));
    }

    #[test]
    fn vmaf_v0_and_vmaf_neg_v0_each_get_their_own_named_model() {
        let job = identical_job(&[MetricId::VmafV0, MetricId::VmafNegV0], false);
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 2);

        let graphs: Vec<String> = invocations
            .iter()
            .map(|invocation| arg_strings(invocation).into_iter().nth(8).unwrap())
            .collect();
        assert!(
            graphs
                .iter()
                .any(|graph| graph.contains("model=version=vmaf_v0.6.1neg"))
        );
        assert!(
            graphs
                .iter()
                .any(|graph| graph.contains("model=version=vmaf_v0.6.1") && !graph.contains("neg"))
        );
    }

    #[test]
    fn psnr_hvs_ciede_and_ms_ssim_attach_to_the_v1_pass_when_it_runs() {
        let info = media_info("clip.mkv", 1920, 1080, ColorRange::Tv, 150);
        let mut job = job_with(
            &[
                MetricId::Vmaf,
                MetricId::PsnrHvs,
                MetricId::Ciede2000,
                MetricId::MsSsim,
            ],
            false,
            info.clone(),
            info,
        );
        job.vmaf_models = vec![v1_model("model/vmaf_v1.0.16", "vmaf_v1.0.16_3d0h.json")];
        let invocations = plan(&job).unwrap();

        assert_eq!(
            invocations.len(),
            1,
            "every extra feature must ride on the one v1 pass"
        );
        let filter_graph = arg_strings(&invocations[0]).into_iter().nth(8).unwrap();
        assert!(filter_graph.contains("feature=name=psnr_hvs|name=ciede|name=float_ms_ssim"));
    }

    #[test]
    fn psnr_hvs_alone_gets_its_own_pass_with_no_model_needed() {
        let job = identical_job(&[MetricId::PsnrHvs], false);
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 1);
        let filter_graph = arg_strings(&invocations[0]).into_iter().nth(8).unwrap();
        assert!(filter_graph.contains("feature=name=psnr_hvs"));
        assert!(!filter_graph.contains("model="));
    }
}
