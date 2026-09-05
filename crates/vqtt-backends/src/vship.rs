use std::ffi::OsString;
use std::path::{Path, PathBuf};
use vqtt_core::backend::{Invocation, LogArtifact, LogFormat, MeasureJob};
use vqtt_core::capability::{BinaryId, LaneKind};
use vqtt_core::corrections;
use vqtt_core::metric::MetricId;

/// Every metric this file can build an FFVship invocation for.
const FFVSHIP_FAMILY: [MetricId; 4] = [
    MetricId::Ssimulacra2,
    MetricId::Butteraugli3Norm,
    MetricId::ButteraugliMax,
    MetricId::Cvvdp,
];

/// The two Butteraugli norms one `-m Butteraugli` process gives together.
const BUTTERAUGLI_GROUP: [MetricId; 2] = [MetricId::Butteraugli3Norm, MetricId::ButteraugliMax];

pub fn plan(job: &MeasureJob) -> vqtt_core::Result<Vec<Invocation>> {
    let ticked: Vec<MetricId> = FFVSHIP_FAMILY
        .iter()
        .copied()
        .filter(|metric| job.metrics.contains(metric))
        .collect();
    if ticked.is_empty() {
        return Ok(Vec::new());
    }

    // Only the frame-count clamp is something an FFVship process can apply, through
    // --start/--end. A color range or resolution mismatch cannot reach FFVship yet;
    // the caller reports that gap separately, once per encode.
    let target_label = job.encode.info.file_name();
    let frame_count =
        corrections::detect_frame_count(&job.reference.info, &job.encode.info, &target_label);
    let frame_range = job.frame_range.or(frame_count.common_range);

    let mut invocations = Vec::new();

    if ticked.contains(&MetricId::Ssimulacra2) {
        invocations.push(single(
            job,
            MetricId::Ssimulacra2,
            "SSIMULACRA2",
            "ssimulacra2.json",
            frame_range,
            None,
        ));
    }
    if BUTTERAUGLI_GROUP
        .iter()
        .any(|metric| ticked.contains(metric))
    {
        let metrics: Vec<MetricId> = BUTTERAUGLI_GROUP
            .iter()
            .copied()
            .filter(|metric| ticked.contains(metric))
            .collect();
        invocations.push(butteraugli(job, &metrics, frame_range));
    }
    if ticked.contains(&MetricId::Cvvdp) {
        invocations.push(single(
            job,
            MetricId::Cvvdp,
            "CVVDP",
            "cvvdp.json",
            frame_range,
            None,
        ));
    }

    Ok(invocations)
}

fn single(
    job: &MeasureJob,
    metric: MetricId,
    metric_flag: &str,
    file_name: &str,
    frame_range: Option<(u64, u64)>,
    intensity_nits: Option<u32>,
) -> Invocation {
    let json_path = job.work_dir.join(file_name);
    Invocation {
        program: PathBuf::from("FFVship"),
        args: ffvship_args(
            &job.reference.path,
            &job.encode.path,
            metric_flag,
            &json_path,
            frame_range,
            intensity_nits,
            job.vship_gpu_threads,
        ),
        env: Vec::new(),
        cwd: None,
        expects: vec![LogArtifact {
            path: json_path,
            format: LogFormat::VshipJson,
            metrics: vec![metric],
        }],
        lane: LaneKind::Gpu,
        binary: BinaryId::Ffvship,
    }
}

fn butteraugli(
    job: &MeasureJob,
    metrics: &[MetricId],
    frame_range: Option<(u64, u64)>,
) -> Invocation {
    let json_path = job.work_dir.join("butteraugli.json");
    Invocation {
        program: PathBuf::from("FFVship"),
        args: ffvship_args(
            &job.reference.path,
            &job.encode.path,
            "Butteraugli",
            &json_path,
            frame_range,
            Some(job.butteraugli_intensity_nits),
            job.vship_gpu_threads,
        ),
        env: Vec::new(),
        cwd: None,
        expects: vec![LogArtifact {
            path: json_path,
            format: LogFormat::VshipJson,
            metrics: metrics.to_vec(),
        }],
        lane: LaneKind::Gpu,
        binary: BinaryId::Ffvship,
    }
}

/// The `-m` argument spelling FFVship's own `--help` gives: `SSIMULACRA2`, `Butteraugli`
/// (mixed case, not `BUTTERAUGLI`), `CVVDP`. This is a different string from the
/// registry's `Requirement::VshipMetric` capability-probe names, which only ever match
/// against `--help` output and never sit on a real command line.
fn ffvship_args(
    reference_path: &Path,
    encode_path: &Path,
    metric_flag: &str,
    json_path: &Path,
    frame_range: Option<(u64, u64)>,
    intensity_nits: Option<u32>,
    gpu_threads: u32,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("--source"),
        reference_path.as_os_str().to_os_string(),
        OsString::from("--encoded"),
        encode_path.as_os_str().to_os_string(),
        OsString::from("-m"),
        OsString::from(metric_flag),
        OsString::from("--json"),
        json_path.as_os_str().to_os_string(),
        // Gives the frame counter something to read. `--json` still writes every
        // value, so this only feeds progress and never the result.
        OsString::from("--live-score-output"),
    ];
    if let Some((first, last)) = frame_range {
        args.push(OsString::from("--start"));
        args.push(OsString::from(first.to_string()));
        args.push(OsString::from("--end"));
        args.push(OsString::from(last.to_string()));
    }
    if let Some(nits) = intensity_nits {
        args.push(OsString::from("--intensity-target"));
        args.push(OsString::from(nits.to_string()));
    }
    args.push(OsString::from("--gpu-threads"));
    args.push(OsString::from(gpu_threads.to_string()));
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use vqtt_core::backend::JobInput;
    use vqtt_core::capability::LaneKind;
    use vqtt_core::media::{ColorRange, MediaInfo, Rational};

    fn media_info(name: &str, frames: u64) -> MediaInfo {
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
            nb_frames: Some(frames),
            duration_s: Some(frames as f64 / 30.0),
            bit_rate: Some(1_000_000),
        }
    }

    fn job_with(metrics: &[MetricId], reference: MediaInfo, encode: MediaInfo) -> MeasureJob {
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
            fused_passes: false,
            work_dir: PathBuf::from("work"),
            vmaf_models: Vec::new(),
            vmaf_viewing_distance: 3.0,
            butteraugli_intensity_nits: 203,
            vship_gpu_threads: 3,
        }
    }

    fn identical_job(metrics: &[MetricId]) -> MeasureJob {
        let info = media_info("clip.mkv", 150);
        job_with(metrics, info.clone(), info)
    }

    fn arg_strings(invocation: &Invocation) -> Vec<String> {
        invocation
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn no_ffvship_metric_gives_no_invocation() {
        let job = identical_job(&[MetricId::PsnrY]);
        assert!(plan(&job).unwrap().is_empty());
    }

    #[test]
    fn ssimulacra2_alone_gives_one_invocation_with_the_ssimulacra2_flag() {
        let job = identical_job(&[MetricId::Ssimulacra2]);
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 1);
        assert!(arg_strings(&invocations[0]).contains(&"SSIMULACRA2".to_string()));
    }

    #[test]
    fn butteraugli_3norm_and_max_fuse_into_one_invocation() {
        let job = identical_job(&[MetricId::Butteraugli3Norm, MetricId::ButteraugliMax]);
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 1);
        assert_eq!(
            invocations[0].expects[0].metrics,
            vec![MetricId::Butteraugli3Norm, MetricId::ButteraugliMax]
        );
    }

    #[test]
    fn only_one_butteraugli_norm_ticked_still_names_only_that_metric_in_expects() {
        let job = identical_job(&[MetricId::Butteraugli3Norm]);
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 1);
        assert!(arg_strings(&invocations[0]).contains(&"Butteraugli".to_string()));
        assert_eq!(
            invocations[0].expects[0].metrics,
            vec![MetricId::Butteraugli3Norm]
        );
    }

    #[test]
    fn cvvdp_alone_gives_one_invocation_with_the_cvvdp_flag() {
        let job = identical_job(&[MetricId::Cvvdp]);
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 1);
        assert!(arg_strings(&invocations[0]).contains(&"CVVDP".to_string()));
    }

    #[test]
    fn source_is_the_reference_and_encoded_is_the_distorted_encode() {
        let job = identical_job(&[MetricId::Ssimulacra2]);
        let invocations = plan(&job).unwrap();
        let args = arg_strings(&invocations[0]);
        let source_index = args.iter().position(|arg| arg == "--source").unwrap();
        let encoded_index = args.iter().position(|arg| arg == "--encoded").unwrap();
        assert_eq!(args[source_index + 1], "reference.mkv");
        assert_eq!(args[encoded_index + 1], "distorted.mkv");
    }

    #[test]
    fn the_metric_flag_uses_the_real_cli_spelling_not_the_registry_capability_string() {
        let job = identical_job(&[MetricId::Butteraugli3Norm]);
        let invocations = plan(&job).unwrap();
        let args = arg_strings(&invocations[0]);
        assert!(args.contains(&"Butteraugli".to_string()));
        assert!(!args.contains(&"BUTTERAUGLI".to_string()));
    }

    #[test]
    fn a_frame_count_mismatch_adds_start_and_end() {
        let reference = media_info("reference.mkv", 150);
        let encode = media_info("distorted.mkv", 140);
        let job = job_with(&[MetricId::Ssimulacra2], reference, encode);
        let invocations = plan(&job).unwrap();
        let args = arg_strings(&invocations[0]);
        assert!(args.contains(&"--start".to_string()));
        assert!(args.contains(&"--end".to_string()));
    }

    #[test]
    fn a_manual_frame_range_wins_over_the_automatic_clamp() {
        let mut job = identical_job(&[MetricId::Ssimulacra2]);
        job.frame_range = Some((10, 49));
        let invocations = plan(&job).unwrap();
        let args = arg_strings(&invocations[0]);
        let start_index = args.iter().position(|arg| arg == "--start").unwrap();
        let end_index = args.iter().position(|arg| arg == "--end").unwrap();
        assert_eq!(args[start_index + 1], "10");
        assert_eq!(args[end_index + 1], "49");
    }

    #[test]
    fn the_butteraugli_intensity_target_is_only_passed_to_the_butteraugli_process() {
        let job = identical_job(&[
            MetricId::Ssimulacra2,
            MetricId::ButteraugliMax,
            MetricId::Cvvdp,
        ]);
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 3);
        for invocation in &invocations {
            let args = arg_strings(invocation);
            let carries_intensity = args.contains(&"--intensity-target".to_string());
            let is_butteraugli = args.contains(&"Butteraugli".to_string());
            assert_eq!(carries_intensity, is_butteraugli);
        }
    }

    #[test]
    fn every_invocation_carries_the_settings_gpu_thread_count() {
        let mut job = identical_job(&[MetricId::Ssimulacra2, MetricId::Cvvdp]);
        job.vship_gpu_threads = 1;
        for invocation in plan(&job).unwrap() {
            let args = arg_strings(&invocation);
            let index = args.iter().position(|arg| arg == "--gpu-threads").unwrap();
            assert_eq!(args[index + 1], "1");
        }
    }

    #[test]
    fn every_invocation_asks_for_a_live_score_so_the_gpu_lane_can_report_frames() {
        let job = identical_job(&[MetricId::Ssimulacra2, MetricId::Butteraugli3Norm]);
        for invocation in plan(&job).unwrap() {
            let args = arg_strings(&invocation);
            assert!(args.iter().any(|arg| arg == "--live-score-output"));
            assert!(
                args.iter().any(|arg| arg == "--json"),
                "the values still come from the json file, never from the live output"
            );
        }
    }

    #[test]
    fn every_invocation_runs_in_the_gpu_lane_and_names_the_ffvship_binary() {
        let job = identical_job(&[
            MetricId::Ssimulacra2,
            MetricId::ButteraugliMax,
            MetricId::Cvvdp,
        ]);
        for invocation in plan(&job).unwrap() {
            assert_eq!(invocation.lane, LaneKind::Gpu);
            assert_eq!(invocation.binary, BinaryId::Ffvship);
        }
    }

    #[test]
    fn each_metric_process_writes_a_distinct_json_file_name() {
        let job = identical_job(&[
            MetricId::Ssimulacra2,
            MetricId::ButteraugliMax,
            MetricId::Cvvdp,
        ]);
        let invocations = plan(&job).unwrap();
        let paths: BTreeSet<PathBuf> = invocations
            .iter()
            .map(|invocation| invocation.expects[0].path.clone())
            .collect();
        assert_eq!(paths.len(), invocations.len());
    }
}
