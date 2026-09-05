//! The two CSV files of a run.
//!
//! A wide per-frame file beats one file for each metric. A spreadsheet reads it once,
//! and every metric shares one time axis.

use crate::record::RunOutcome;
use std::io::{BufWriter, Write};
use std::path::Path;
use vqtt_core::metric::MetricId;
use vqtt_core::pooling::Pooled;

/// The header of the pooled file.
const SUMMARY_HEADER: &str = "run_id,encode,bitrate_bps,frames,metric,mean,harmonic_mean,median,p1,p5,p10,p25,p75,p90,p95,p99,min,max,stdev";

/// One metric's values, in frame order.
pub struct MetricColumn<'a> {
    pub metric: MetricId,
    pub values: &'a [f32],
}

/// Writes one row for each frame, and one column for each metric.
///
/// The `frame` column is the frame index of the reference, so a run over part of a file
/// still reads in the numbers the rest of the tool shows. A metric with no value for a
/// frame writes an empty cell, never a zero, because a zero is a measurement.
pub fn write_frame_csv(
    output_path: &Path,
    first_frame: u64,
    columns: &[MetricColumn],
) -> vqtt_core::Result<()> {
    let file = std::fs::File::create(output_path).map_err(io_error)?;
    let mut writer = BufWriter::new(file);

    write!(writer, "frame").map_err(io_error)?;
    for column in columns {
        write!(writer, ",{}", column.metric.key()).map_err(io_error)?;
    }
    writeln!(writer).map_err(io_error)?;

    let frame_count = columns
        .iter()
        .map(|column| column.values.len())
        .max()
        .unwrap_or(0);
    for frame in 0..frame_count {
        write!(writer, "{}", first_frame + frame as u64).map_err(io_error)?;
        for column in columns {
            write!(writer, ",").map_err(io_error)?;
            if let Some(value) = column.values.get(frame) {
                write!(writer, "{value}").map_err(io_error)?;
            }
        }
        writeln!(writer).map_err(io_error)?;
    }

    writer.flush().map_err(io_error)
}

/// One row for each encode and metric.
pub fn write_summary_csv(
    output_path: &Path,
    run_id: &str,
    rows: &[SummaryRow],
) -> vqtt_core::Result<()> {
    let file = std::fs::File::create(output_path).map_err(io_error)?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "{SUMMARY_HEADER}").map_err(io_error)?;
    for row in rows {
        let pooled = &row.pooled;
        let harmonic = match pooled.harmonic_mean {
            Some(value) => value.to_string(),
            None => String::new(),
        };
        let bitrate = match row.bitrate_bps {
            Some(value) => value.to_string(),
            None => String::new(),
        };
        writeln!(
            writer,
            "{run_id},{},{bitrate},{},{},{},{harmonic},{},{},{},{},{},{},{},{},{},{},{},{}",
            escape(&row.encode),
            row.frames,
            row.metric.key(),
            pooled.mean,
            pooled.median,
            pooled.p1,
            pooled.p5,
            pooled.p10,
            pooled.p25,
            pooled.p75,
            pooled.p90,
            pooled.p95,
            pooled.p99,
            pooled.min,
            pooled.max,
            pooled.stdev,
        )
        .map_err(io_error)?;
    }

    writer.flush().map_err(io_error)
}

pub struct SummaryRow {
    pub encode: String,
    pub bitrate_bps: Option<u64>,
    pub frames: usize,
    pub metric: MetricId,
    pub pooled: Pooled,
}

/// Writes every command line, with what it cost and how it ended.
pub fn write_command_log(output_path: &Path, outcome: &RunOutcome) -> vqtt_core::Result<()> {
    let file = std::fs::File::create(output_path).map_err(io_error)?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "# vqtt command log, run {}", outcome.run_id).map_err(io_error)?;
    for record in &outcome.invocations {
        writeln!(writer).map_err(io_error)?;
        writeln!(writer, "[{}] lane {}", record.seq, record.lane).map_err(io_error)?;
        if let Some(cwd) = &record.cwd {
            writeln!(writer, "cwd {}", cwd.display()).map_err(io_error)?;
        }
        writeln!(writer, "{}", record.command_line()).map_err(io_error)?;
        match record.exit_code {
            Some(code) => writeln!(writer, "exit {code} after {} ms", record.wall_ms),
            None => writeln!(
                writer,
                "stopped without an exit code after {} ms",
                record.wall_ms
            ),
        }
        .map_err(io_error)?;
    }

    writer.flush().map_err(io_error)
}

/// A field a spreadsheet would split on. Only the label can hold a comma.
fn escape(text: &str) -> String {
    if text.contains(',') || text.contains('"') {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn io_error(error: std::io::Error) -> vqtt_core::CoreError {
    vqtt_core::CoreError::parse("csv", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::InvocationRecord;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use vqtt_core::palette::Theme;

    fn out(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("vqtt-csv-writer-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn two_metrics_of_the_same_length_give_one_row_for_each_frame() {
        let path = out("wide_a.csv");
        write_frame_csv(
            &path,
            0,
            &[
                MetricColumn {
                    metric: MetricId::PsnrY,
                    values: &[40.0, 41.0],
                },
                MetricColumn {
                    metric: MetricId::SsimAll,
                    values: &[0.91, 0.92],
                },
            ],
        )
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "frame,psnr_y,ssim_all\n0,40,0.91\n1,41,0.92\n");
    }

    #[test]
    fn a_short_metric_leaves_an_empty_cell_and_not_a_zero() {
        let path = out("wide_b.csv");
        write_frame_csv(
            &path,
            0,
            &[
                MetricColumn {
                    metric: MetricId::PsnrY,
                    values: &[40.0, 41.0],
                },
                MetricColumn {
                    metric: MetricId::SsimAll,
                    values: &[0.91],
                },
            ],
        )
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "frame,psnr_y,ssim_all\n0,40,0.91\n1,41,\n");
    }

    #[test]
    fn a_run_over_part_of_a_file_numbers_its_rows_from_the_first_measured_frame() {
        let path = out("wide_c.csv");
        write_frame_csv(
            &path,
            1200,
            &[MetricColumn {
                metric: MetricId::PsnrY,
                values: &[40.0, 41.0],
            }],
        )
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "frame,psnr_y\n1200,40\n1201,41\n");
    }

    #[test]
    fn a_blocked_harmonic_mean_writes_an_empty_cell_and_the_label_holds_its_comma() {
        let path = out("summary_a.csv");
        let pooled = vqtt_core::pooling::pool(
            &[1.0, 2.0, 3.0],
            vqtt_core::metric::HarmonicMean::Blocked("no"),
        )
        .unwrap();
        write_summary_csv(
            &path,
            "run-1",
            &[SummaryRow {
                encode: "a,b".to_string(),
                bitrate_bps: Some(5000),
                frames: 3,
                metric: MetricId::Cambi,
                pooled,
            }],
        )
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let row = content.lines().nth(1).unwrap();
        assert!(content.starts_with(SUMMARY_HEADER));
        assert!(row.starts_with("run-1,\"a,b\",5000,3,cambi,"));
        assert!(
            row.contains(",,"),
            "the blocked harmonic mean is an empty cell"
        );
    }

    #[test]
    fn the_command_log_names_every_invocation_with_its_exit_code_and_wall_time() {
        let path = out("commands_a.txt");
        let outcome = RunOutcome {
            run_id: "run-1".into(),
            started: "now".into(),
            finished: "later".into(),
            metrics: Vec::new(),
            frame_range: None,
            first_frame: 0,
            results: HashMap::new(),
            series: HashMap::new(),
            corrections: Vec::new(),
            notes: Vec::new(),
            invocations: vec![
                InvocationRecord {
                    seq: 1,
                    lane: "cpu",
                    program: PathBuf::from("ffmpeg"),
                    args: vec!["-i".into(), "a.mov".into()],
                    cwd: None,
                    exit_code: Some(0),
                    wall_ms: 900,
                },
                InvocationRecord {
                    seq: 2,
                    lane: "gpu",
                    program: PathBuf::from("FFVship"),
                    args: vec!["--source".into()],
                    cwd: Some(PathBuf::from("D:/models")),
                    exit_code: None,
                    wall_ms: 40,
                },
            ],
            vmaf_model: None,
            theme: Theme::Dark,
        };

        write_command_log(&path, &outcome).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("ffmpeg -i a.mov"));
        assert!(content.contains("exit 0 after 900 ms"));
        assert!(content.contains("cwd D:/models"));
        assert!(content.contains("stopped without an exit code after 40 ms"));
    }
}
