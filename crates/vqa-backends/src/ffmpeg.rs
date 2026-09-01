use std::ffi::OsString;
use std::path::PathBuf;
use vqa_core::backend::{Invocation, LogArtifact, LogFormat, MeasureJob};
use vqa_core::capability::LaneKind;
use vqa_core::metric::MetricId;

const FFMPEG_FAMILY: [MetricId; 3] = [MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin];

pub fn plan(job: &MeasureJob) -> vqa_core::Result<Vec<Invocation>> {
    let metrics: Vec<MetricId> = FFMPEG_FAMILY
        .iter()
        .copied()
        .filter(|metric| job.metrics.contains(metric))
        .collect();

    if metrics.is_empty() {
        return Ok(Vec::new());
    }

    if job.fused_passes {
        Ok(vec![fused(job, &metrics)])
    } else {
        Ok(metrics
            .iter()
            .map(|metric| separate(job, *metric))
            .collect())
    }
}

fn separate(job: &MeasureJob, metric: MetricId) -> Invocation {
    let stats_path = job.work_dir.join(log_file_name(metric));
    let format = log_format_of(metric);

    let distorted_pad = "distorted_pad";
    let reference_pad = "reference_pad";
    let reset_distorted = format!("[0:v]setpts=PTS-STARTPTS[{distorted_pad}]");
    let reset_reference = format!("[1:v]setpts=PTS-STARTPTS[{reference_pad}]");
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

fn fused(job: &MeasureJob, metrics: &[MetricId]) -> Invocation {
    let count = metrics.len();
    let distorted_pads: Vec<String> = (0..count)
        .map(|index| format!("distorted{index}"))
        .collect();
    let reference_pads: Vec<String> = (0..count)
        .map(|index| format!("reference{index}"))
        .collect();

    let mut filter_graph = format!(
        "[0:v]setpts=PTS-STARTPTS,split={count}[{}];[1:v]setpts=PTS-STARTPTS,split={count}[{}]",
        distorted_pads.join("]["),
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

    fn media_info(name: &str) -> MediaInfo {
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
            nb_frames: Some(300),
            duration_s: Some(10.0),
            bit_rate: Some(1_000_000),
        }
    }

    fn job_with(metrics: &[MetricId], fused_passes: bool) -> MeasureJob {
        MeasureJob {
            reference: JobInput {
                path: PathBuf::from("reference.mkv"),
                info: media_info("reference.mkv"),
            },
            encode: JobInput {
                path: PathBuf::from("distorted.mkv"),
                info: media_info("distorted.mkv"),
            },
            metrics: metrics.iter().copied().collect::<BTreeSet<_>>(),
            frame_range: None,
            fused_passes,
            work_dir: PathBuf::from("work"),
        }
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
        let job = job_with(&[MetricId::Vmaf], false);
        assert!(plan(&job).unwrap().is_empty());
    }

    #[test]
    fn xpsnr_takes_the_reference_first_and_psnr_takes_the_distorted_first() {
        let job = job_with(&[MetricId::PsnrY, MetricId::XpsnrMin], false);
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
    fn a_separate_pass_gives_one_invocation_for_each_metric() {
        let job = job_with(
            &[MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin],
            false,
        );
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 3);
        for invocation in &invocations {
            assert_eq!(invocation.expects.len(), 1);
        }
    }

    #[test]
    fn a_fused_pass_gives_one_invocation_that_expects_every_log() {
        let job = job_with(
            &[MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin],
            true,
        );
        let invocations = plan(&job).unwrap();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].expects.len(), 3);
    }

    #[test]
    fn every_invocation_ends_with_the_null_output() {
        let job = job_with(&[MetricId::PsnrY], false);
        let invocation = &plan(&job).unwrap()[0];
        let args = arg_strings(invocation);
        assert_eq!(&args[args.len() - 3..], ["-f", "null", "-"]);
    }
}
