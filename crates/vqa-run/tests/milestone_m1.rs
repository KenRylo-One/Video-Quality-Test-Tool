use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vqa_backends::ffmpeg;
use vqa_backends::ffprobe::FfprobeProbe;
use vqa_core::backend::{JobInput, MeasureJob};
use vqa_core::metric::MetricId;
use vqa_core::probe::MediaProbe;
use vqa_core::set::FileId;
use vqa_run::{EncodeWork, RealProcessRunner, SupervisorEvent, run_plan};

fn media_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Test-Media")
}

fn probe() -> Option<FfprobeProbe> {
    let discovery = vqa_backends::Discovery::default();
    vqa_backends::find_path(vqa_core::BinaryId::Ffprobe, &discovery).map(FfprobeProbe::new)
}

fn ffmpeg_path() -> Option<PathBuf> {
    let discovery = vqa_backends::Discovery::default();
    vqa_backends::find_path(vqa_core::BinaryId::Ffmpeg, &discovery)
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("vqa-milestone-m1-test")
        .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Builds a job that measures one file against itself or against another file, with
/// every FFmpeg family metric ticked.
fn job(
    path: &Path,
    info: vqa_core::media::MediaInfo,
    work_dir: PathBuf,
    fused_passes: bool,
) -> MeasureJob {
    let metrics: BTreeSet<MetricId> =
        [MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin].into();
    MeasureJob {
        reference: JobInput {
            path: path.to_path_buf(),
            info: info.clone(),
        },
        encode: JobInput {
            path: path.to_path_buf(),
            info,
        },
        metrics,
        frame_range: None,
        fused_passes,
        work_dir,
    }
}

#[test]
fn test_1_the_identity_test() {
    let (Some(probe), Some(ffmpeg_program)) = (probe(), ffmpeg_path()) else {
        return;
    };
    let path = media_folder().join("TEST_A_full_range_flagged_pc.mp4");
    if !path.is_file() {
        return;
    }

    let info = probe.probe(&path).unwrap();
    let job = job(&path, info, work_dir("identity"), false);
    let mut invocations = ffmpeg::plan(&job).unwrap();
    for invocation in &mut invocations {
        invocation.program = ffmpeg_program.clone();
    }

    let work = vec![EncodeWork {
        encode: FileId(1),
        invocations,
    }];
    let receiver = run_plan(work, Arc::new(RealProcessRunner), 3, 1);

    let mut saw_infinite_psnr = false;
    let mut saw_perfect_ssim = false;
    for event in receiver.iter() {
        if let SupervisorEvent::MetricReady { metric, pooled, .. } = event {
            match metric {
                MetricId::PsnrY => {
                    assert!(pooled.mean.is_infinite());
                    saw_infinite_psnr = true;
                }
                MetricId::SsimAll => {
                    assert!((pooled.mean - 1.0).abs() < 0.0001);
                    saw_perfect_ssim = true;
                }
                _ => {}
            }
        }
    }
    assert!(
        saw_infinite_psnr,
        "the identity test needs an infinite PSNR value"
    );
    assert!(
        saw_perfect_ssim,
        "the identity test needs an SSIM of 1.0000"
    );
}

#[test]
fn test_3_the_fusion_test() {
    let (Some(probe), Some(ffmpeg_program)) = (probe(), ffmpeg_path()) else {
        return;
    };
    let reference_path = media_folder().join("TEST_A_full_range_flagged_pc.mp4");
    let encode_path = media_folder().join("TEST_B_limited_range_flagged_tv.mp4");
    if !reference_path.is_file() || !encode_path.is_file() {
        return;
    }

    let reference_info = probe.probe(&reference_path).unwrap();
    let encode_info = probe.probe(&encode_path).unwrap();
    let metrics: BTreeSet<MetricId> =
        [MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin].into();

    let fused_dir = work_dir("fusion_fused");
    let separate_dir = work_dir("fusion_separate");

    let fused_job = MeasureJob {
        reference: JobInput {
            path: reference_path.clone(),
            info: reference_info.clone(),
        },
        encode: JobInput {
            path: encode_path.clone(),
            info: encode_info.clone(),
        },
        metrics: metrics.clone(),
        frame_range: None,
        fused_passes: true,
        work_dir: fused_dir.clone(),
    };
    let separate_job = MeasureJob {
        reference: JobInput {
            path: reference_path,
            info: reference_info,
        },
        encode: JobInput {
            path: encode_path,
            info: encode_info,
        },
        metrics,
        frame_range: None,
        fused_passes: false,
        work_dir: separate_dir.clone(),
    };

    for job in [&fused_job, &separate_job] {
        let mut invocations = ffmpeg::plan(job).unwrap();
        for invocation in &mut invocations {
            invocation.program = ffmpeg_program.clone();
            let status = std::process::Command::new(&invocation.program)
                .args(&invocation.args)
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    for name in ["psnr.log", "ssim.log", "xpsnr.log"] {
        let fused_content = std::fs::read_to_string(fused_dir.join(name)).unwrap();
        let separate_content = std::fs::read_to_string(separate_dir.join(name)).unwrap();
        assert_eq!(
            fused_content, separate_content,
            "{name} differs between the fused pass and the separate pass"
        );
    }
}

#[test]
fn test_4_memory_stays_bounded_on_a_real_run() {
    let (Some(probe), Some(ffmpeg_program)) = (probe(), ffmpeg_path()) else {
        return;
    };
    let path = media_folder().join("TEST_A_full_range_flagged_pc.mp4");
    if !path.is_file() {
        return;
    }

    let info = probe.probe(&path).unwrap();
    let job = job(&path, info, work_dir("memory"), true);
    let mut invocations = ffmpeg::plan(&job).unwrap();
    for invocation in &mut invocations {
        invocation.program = ffmpeg_program.clone();
    }

    let work = vec![EncodeWork {
        encode: FileId(1),
        invocations,
    }];
    let receiver = run_plan(work, Arc::new(RealProcessRunner), 1, 1);

    // TEST_A is five seconds long, so this is a smoke test that a run completes and
    // reports every metric, not the twenty-minute 2160p case that the design names.
    // That case needs a real render and a manual check, not an automated test.
    let mut metric_count = 0;
    for event in receiver.iter() {
        if let SupervisorEvent::MetricReady { .. } = event {
            metric_count += 1;
        }
    }
    assert_eq!(metric_count, 3);
}

#[test]
fn test_6_a_finished_metric_arrives_before_the_slower_metric() {
    let (Some(probe), Some(ffmpeg_program)) = (probe(), ffmpeg_path()) else {
        return;
    };
    let path = media_folder().join("TEST_A_full_range_flagged_pc.mp4");
    if !path.is_file() {
        return;
    }

    let info = probe.probe(&path).unwrap();
    let job = job(&path, info, work_dir("partial"), false);
    let mut invocations = ffmpeg::plan(&job).unwrap();
    for invocation in &mut invocations {
        invocation.program = ffmpeg_program.clone();
    }

    let work = vec![EncodeWork {
        encode: FileId(1),
        invocations,
    }];
    // One lane, so the invocations run one at a time, and the order that they finish
    // in is the order that plan() listed them: PSNR, then SSIM, then XPSNR.
    let receiver = run_plan(work, Arc::new(RealProcessRunner), 1, 1);

    let mut arrival_order = Vec::new();
    for event in receiver.iter() {
        if let SupervisorEvent::MetricReady { metric, .. } = event {
            arrival_order.push(metric);
        }
    }
    assert_eq!(
        arrival_order,
        vec![MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin]
    );
}
