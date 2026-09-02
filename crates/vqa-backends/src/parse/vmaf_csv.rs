//! Reads the CSV log that the `libvmaf` filter writes.
//!
//! One file can hold several metrics, one column for each. The column name for
//! `vmaf`, `VmafV0` and `VmafNegV0` is the same, `vmaf`, and the same is true of
//! `Cambi` and `VmafV1Cambi`, both named `cambi`. Which file a metric's column came
//! from is what tells them apart, not the column name, so this parser only ever reads
//! one metric out of one file at a time, the same as the caller already asks for.

use std::io::BufRead;
use vqa_core::backend::FrameSink;
use vqa_core::metric::MetricId;

/// The CSV column name that holds one metric's value.
///
/// Real column names captured from this project's own `ffmpeg` build:
/// `Frame,integer_adm2,...,psnr_hvs_y,psnr_hvs_cb,psnr_hvs_cr,psnr_hvs,ciede2000,
/// float_ms_ssim,cambi,integer_motion2,integer_motion3,vmaf,`
fn column_name(metric: MetricId) -> Option<&'static str> {
    match metric {
        MetricId::Vmaf | MetricId::VmafV0 | MetricId::VmafNegV0 => Some("vmaf"),
        MetricId::Cambi | MetricId::VmafV1Cambi => Some("cambi"),
        MetricId::PsnrHvs => Some("psnr_hvs"),
        MetricId::Ciede2000 => Some("ciede2000"),
        MetricId::MsSsim => Some("float_ms_ssim"),
        _ => None,
    }
}

/// Reads one metric's column out of a `libvmaf` CSV log.
pub fn parse_vmaf_csv(
    reader: impl BufRead,
    metric: MetricId,
    sink: &mut dyn FrameSink,
) -> vqa_core::Result<()> {
    let column = column_name(metric).ok_or_else(|| {
        vqa_core::CoreError::parse("vmaf csv", format!("{metric:?} has no libvmaf CSV column"))
    })?;

    let mut lines = reader.lines();
    let header = lines
        .next()
        .ok_or_else(|| vqa_core::CoreError::parse("vmaf csv", "empty file"))?
        .map_err(|error| vqa_core::CoreError::parse("vmaf csv", error.to_string()))?;
    let headers: Vec<&str> = header.split(',').collect();
    let column_index = headers
        .iter()
        .position(|name| *name == column)
        .ok_or_else(|| {
            vqa_core::CoreError::parse("vmaf csv", format!("no {column} column in {header}"))
        })?;

    for line in lines {
        let line =
            line.map_err(|error| vqa_core::CoreError::parse("vmaf csv", error.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let frame: u64 = fields
            .first()
            .and_then(|field| field.parse().ok())
            .ok_or_else(|| vqa_core::CoreError::parse("vmaf csv", format!("bad row: {line}")))?;
        let value: f32 = fields
            .get(column_index)
            .and_then(|field| field.trim().parse().ok())
            .ok_or_else(|| vqa_core::CoreError::parse("vmaf csv", format!("bad row: {line}")))?;
        sink.push(metric, frame, value);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vqa_core::backend::BufferSink;

    /// Captured for real from this machine's `ffmpeg`, with `psnr_hvs`, `ciede`,
    /// `float_ms_ssim` and `cambi` all requested as extra features alongside the
    /// default model.
    const REAL_CSV: &str = "Frame,integer_adm2,integer_aim,integer_adm3,integer_adm_scale0,integer_adm_scale1,integer_adm_scale2,integer_adm_scale3,VMAF_integer_feature_motion_sad_score,integer_vif_scale0,integer_vif_scale1,integer_vif_scale2,integer_vif_scale3,psnr_hvs_y,psnr_hvs_cb,psnr_hvs_cr,psnr_hvs,ciede2000,float_ms_ssim,cambi,integer_motion2,integer_motion3,vmaf,\n\
0,0.966702,0.003968,0.981367,0.997116,0.976805,0.921449,0.908977,0.000000,0.928167,0.978296,0.989208,0.997001,47.155364,53.548673,52.987293,47.866011,48.205560,0.999222,7.965462,0.000000,0.000000,89.340759,\n\
1,0.966702,0.003968,0.981367,0.997116,0.976805,0.921449,0.908977,0.000000,0.928167,0.978296,0.989208,0.997001,47.155364,53.548673,52.987293,47.866011,48.205560,0.999222,7.965462,0.000000,0.000000,89.340759,\n";

    /// Captured on an identity comparison: `ciede2000` reports `inf` at a perfect
    /// match, the same infinite-at-perfect-match shape PSNR already has.
    const IDENTITY_CSV: &str = "Frame,integer_adm2,ciede2000,vmaf,\n\
0,0.999973,inf,97.421980,\n";

    #[test]
    fn reads_the_vmaf_column_by_name_and_not_by_position() {
        let mut sink = BufferSink::default();
        parse_vmaf_csv(REAL_CSV.as_bytes(), MetricId::VmafV0, &mut sink).unwrap();
        assert_eq!(sink.values, vec![89.340_76, 89.340_76]);
    }

    #[test]
    fn reads_the_cambi_column() {
        let mut sink = BufferSink::default();
        parse_vmaf_csv(REAL_CSV.as_bytes(), MetricId::Cambi, &mut sink).unwrap();
        assert_eq!(sink.values, vec![7.965462, 7.965462]);
    }

    #[test]
    fn reads_the_psnr_hvs_and_ciede_and_ms_ssim_columns() {
        let mut sink = BufferSink::default();
        parse_vmaf_csv(REAL_CSV.as_bytes(), MetricId::PsnrHvs, &mut sink).unwrap();
        assert_eq!(sink.values, vec![47.866011, 47.866011]);

        let mut sink = BufferSink::default();
        parse_vmaf_csv(REAL_CSV.as_bytes(), MetricId::Ciede2000, &mut sink).unwrap();
        assert_eq!(sink.values, vec![48.205_56, 48.205_56]);

        let mut sink = BufferSink::default();
        parse_vmaf_csv(REAL_CSV.as_bytes(), MetricId::MsSsim, &mut sink).unwrap();
        assert_eq!(sink.values, vec![0.999222, 0.999222]);
    }

    #[test]
    fn an_infinite_value_at_a_perfect_match_parses_as_infinity() {
        let mut sink = BufferSink::default();
        parse_vmaf_csv(IDENTITY_CSV.as_bytes(), MetricId::Ciede2000, &mut sink).unwrap();
        assert_eq!(sink.values, vec![f32::INFINITY]);
    }

    #[test]
    fn a_missing_column_is_a_parse_error() {
        let mut sink = BufferSink::default();
        let result = parse_vmaf_csv(IDENTITY_CSV.as_bytes(), MetricId::MsSsim, &mut sink);
        assert!(result.is_err());
    }

    #[test]
    fn a_metric_with_no_libvmaf_column_is_a_parse_error() {
        let mut sink = BufferSink::default();
        let result = parse_vmaf_csv(REAL_CSV.as_bytes(), MetricId::PsnrY, &mut sink);
        assert!(result.is_err());
    }
}
