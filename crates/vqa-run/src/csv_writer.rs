use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use vqa_backends::parse::stats_file::parse_stats_file;
use vqa_core::backend::{BufferSink, LogFormat};
use vqa_core::metric::MetricId;

pub struct MetricColumn {
    pub metric: MetricId,
    pub log_path: PathBuf,
    pub log_format: LogFormat,
}

pub fn write_frame_csv(output_path: &Path, columns: &[MetricColumn]) -> vqa_core::Result<()> {
    let mut series = Vec::with_capacity(columns.len());
    for column in columns {
        series.push(read_series(column)?);
    }

    let file = std::fs::File::create(output_path)
        .map_err(|error| vqa_core::CoreError::parse("frame csv", error.to_string()))?;
    let mut writer = BufWriter::new(file);

    write_header(&mut writer, columns)?;

    let frame_count = series.iter().map(Vec::len).max().unwrap_or(0);
    for frame in 0..frame_count {
        write!(writer, "{frame}").map_err(io_error)?;
        for values in &series {
            write!(writer, ",").map_err(io_error)?;
            if let Some(value) = values.get(frame) {
                write!(writer, "{value}").map_err(io_error)?;
            }
        }
        writeln!(writer).map_err(io_error)?;
    }

    Ok(())
}

fn read_series(column: &MetricColumn) -> vqa_core::Result<Vec<f32>> {
    let file = std::fs::File::open(&column.log_path)
        .map_err(|error| vqa_core::CoreError::parse("frame csv", error.to_string()))?;
    let mut sink = BufferSink::default();
    parse_stats_file(
        BufReader::new(file),
        column.log_format,
        column.metric,
        &mut sink,
    )?;
    Ok(sink.values)
}

fn write_header(writer: &mut impl Write, columns: &[MetricColumn]) -> vqa_core::Result<()> {
    write!(writer, "frame").map_err(io_error)?;
    for column in columns {
        write!(writer, ",{}", column.metric.key()).map_err(io_error)?;
    }
    writeln!(writer).map_err(io_error)
}

fn io_error(error: std::io::Error) -> vqa_core::CoreError {
    vqa_core::CoreError::parse("frame csv", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_log(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("vqa-csv-writer-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn two_metrics_of_the_same_length_give_one_row_for_each_frame() {
        let psnr_log = write_log("psnr_a.log", "n:1 psnr_y:40.0\nn:2 psnr_y:41.0\n");
        let ssim_log = write_log(
            "ssim_a.log",
            "n:1 Y:0.9 U:0.9 V:0.9 All:0.91 (10.0)\nn:2 Y:0.9 U:0.9 V:0.9 All:0.92 (10.0)\n",
        );
        let output_path = std::env::temp_dir()
            .join("vqa-csv-writer-test")
            .join("out_a.csv");

        write_frame_csv(
            &output_path,
            &[
                MetricColumn {
                    metric: MetricId::PsnrY,
                    log_path: psnr_log,
                    log_format: LogFormat::PsnrStats,
                },
                MetricColumn {
                    metric: MetricId::SsimAll,
                    log_path: ssim_log,
                    log_format: LogFormat::SsimStats,
                },
            ],
        )
        .unwrap();

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(content, "frame,psnr_y,ssim_all\n0,40,0.91\n1,41,0.92\n");
    }

    #[test]
    fn a_short_metric_leaves_an_empty_cell_and_not_a_zero() {
        let psnr_log = write_log("psnr_b.log", "n:1 psnr_y:40.0\nn:2 psnr_y:41.0\n");
        let short_log = write_log("ssim_b.log", "n:1 Y:0.9 U:0.9 V:0.9 All:0.91 (10.0)\n");
        let output_path = std::env::temp_dir()
            .join("vqa-csv-writer-test")
            .join("out_b.csv");

        write_frame_csv(
            &output_path,
            &[
                MetricColumn {
                    metric: MetricId::PsnrY,
                    log_path: psnr_log,
                    log_format: LogFormat::PsnrStats,
                },
                MetricColumn {
                    metric: MetricId::SsimAll,
                    log_path: short_log,
                    log_format: LogFormat::SsimStats,
                },
            ],
        )
        .unwrap();

        let content = std::fs::read_to_string(&output_path).unwrap();
        assert_eq!(content, "frame,psnr_y,ssim_all\n0,40,0.91\n1,41,\n");
    }
}
