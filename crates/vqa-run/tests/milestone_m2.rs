use std::path::{Path, PathBuf};
use vqa_backends::ffmpeg::{self, CorrectionToggles};
use vqa_backends::ffprobe::FfprobeProbe;
use vqa_core::backend::{JobInput, MeasureJob};
use vqa_core::metric::MetricId;
use vqa_core::probe::MediaProbe;

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
        .join("vqa-milestone-m2-test")
        .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the one PSNR invocation that `plan_with_toggles` builds, and reads back the
/// mean PSNR-Y that the pooled result would show.
fn measure_psnr_y(job: &MeasureJob, ffmpeg_program: &Path, toggles: CorrectionToggles) -> f32 {
    let mut invocations = ffmpeg::plan_with_toggles(job, toggles).unwrap();
    let invocation = &mut invocations[0];
    invocation.program = ffmpeg_program.to_path_buf();
    let status = std::process::Command::new(&invocation.program)
        .args(&invocation.args)
        .status()
        .unwrap();
    assert!(status.success());

    let log_path = &invocation.expects[0].path;
    let content = std::fs::read_to_string(log_path).unwrap();
    let last_line = content.lines().next_back().unwrap();
    let value = last_line
        .split_whitespace()
        .find_map(|token| token.strip_prefix("psnr_y:"))
        .unwrap();
    value.parse().unwrap()
}

#[test]
fn test_row_2_the_color_range_correction_gives_a_higher_score_than_no_correction() {
    let (Some(probe), Some(ffmpeg_program)) = (probe(), ffmpeg_path()) else {
        return;
    };
    let reference_path = media_folder().join("TEST_B_limited_range_flagged_tv.mp4");
    let encode_path = media_folder().join("TEST_A_full_range_flagged_pc.mp4");
    if !reference_path.is_file() || !encode_path.is_file() {
        return;
    }

    let reference_info = probe.probe(&reference_path).unwrap();
    let encode_info = probe.probe(&encode_path).unwrap();

    let job = MeasureJob {
        reference: JobInput {
            path: reference_path,
            info: reference_info,
        },
        encode: JobInput {
            path: encode_path,
            info: encode_info,
        },
        metrics: [MetricId::PsnrY].into_iter().collect(),
        frame_range: None,
        fused_passes: false,
        work_dir: work_dir("color_range"),
    };

    let corrected = measure_psnr_y(&job, &ffmpeg_program, CorrectionToggles::default());
    let uncorrected = measure_psnr_y(
        &job,
        &ffmpeg_program,
        CorrectionToggles {
            color_range: false,
            ..CorrectionToggles::default()
        },
    );

    assert!(
        corrected > uncorrected,
        "the color range correction must raise the score: corrected {corrected}, uncorrected {uncorrected}"
    );
}
