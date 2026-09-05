//! Reads the `--json` file that FFVship writes.
//!
//! Every FFVship metric shares the same outer shape: an array of arrays, one inner
//! array for each frame. SSIMULACRA 2 and ColorVideoVDP write one column; Butteraugli
//! writes three, NormQ, Norm3, NormINF, in that order. This is why Butteraugli3Norm and
//! ButteraugliMax already share one `fuse_group` in the registry: one FFVship process
//! with `-m Butteraugli` gives both columns in one file.
//!
//! ColorVideoVDP's column is cumulative through the clip, not a per-frame value. Only
//! its last row is a real number, the score of the whole clip, per FFVship's own docs.
//! This parser pushes exactly that one value, at frame 0, through the same `FrameSink`
//! every other parser uses, so it rides the same tested `pool()` path.

use std::io::BufRead;
use vqtt_core::backend::FrameSink;
use vqtt_core::metric::MetricId;

fn column_index(metric: MetricId) -> Option<usize> {
    match metric {
        MetricId::Ssimulacra2 | MetricId::Cvvdp => Some(0),
        MetricId::Butteraugli3Norm => Some(1),
        MetricId::ButteraugliMax => Some(2),
        _ => None,
    }
}

pub fn parse_vship_json(
    mut reader: impl BufRead,
    metric: MetricId,
    sink: &mut dyn FrameSink,
) -> vqtt_core::Result<()> {
    let column = column_index(metric).ok_or_else(|| {
        vqtt_core::CoreError::parse(
            "vship json",
            format!("{metric:?} has no FFVship JSON column"),
        )
    })?;

    let mut text = String::new();
    reader
        .read_to_string(&mut text)
        .map_err(|error| vqtt_core::CoreError::parse("vship json", error.to_string()))?;
    let rows: Vec<Vec<f32>> = serde_json::from_str(&text)
        .map_err(|error| vqtt_core::CoreError::parse("vship json", error.to_string()))?;

    if metric == MetricId::Cvvdp {
        let last = rows
            .last()
            .ok_or_else(|| vqtt_core::CoreError::parse("vship json", "empty json array"))?;
        let value = *last
            .get(column)
            .ok_or_else(|| vqtt_core::CoreError::parse("vship json", "row has no column 0"))?;
        sink.push(metric, 0, value);
        return Ok(());
    }

    for (frame, row) in rows.iter().enumerate() {
        let value = *row.get(column).ok_or_else(|| {
            vqtt_core::CoreError::parse("vship json", format!("row {frame} has no column {column}"))
        })?;
        sink.push(metric, frame as u64, value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vqtt_core::backend::BufferSink;

    const SSIMULACRA2_JSON: &str = "[[70.5], [68.2], [71.0]]";
    const BUTTERAUGLI_JSON: &str = "[[0.5, 1.0, 3.0], [0.6, 1.5, 4.5]]";
    const CVVDP_JSON: &str = "[[9.6], [9.4], [9.1]]";

    #[test]
    fn reads_ssimulacra2_one_value_for_each_row() {
        let mut sink = BufferSink::default();
        parse_vship_json(
            SSIMULACRA2_JSON.as_bytes(),
            MetricId::Ssimulacra2,
            &mut sink,
        )
        .unwrap();
        assert_eq!(sink.values, vec![70.5, 68.2, 71.0]);
    }

    #[test]
    fn reads_the_norm3_and_norminf_columns_from_the_same_file() {
        let mut norm3 = BufferSink::default();
        parse_vship_json(
            BUTTERAUGLI_JSON.as_bytes(),
            MetricId::Butteraugli3Norm,
            &mut norm3,
        )
        .unwrap();
        assert_eq!(norm3.values, vec![1.0, 1.5]);

        let mut norm_inf = BufferSink::default();
        parse_vship_json(
            BUTTERAUGLI_JSON.as_bytes(),
            MetricId::ButteraugliMax,
            &mut norm_inf,
        )
        .unwrap();
        assert_eq!(norm_inf.values, vec![3.0, 4.5]);
    }

    #[test]
    fn cvvdp_pushes_only_the_last_cumulative_row_as_frame_zero() {
        let mut sink = BufferSink::default();
        parse_vship_json(CVVDP_JSON.as_bytes(), MetricId::Cvvdp, &mut sink).unwrap();
        assert_eq!(sink.values, vec![9.1]);
    }

    #[test]
    fn a_metric_with_no_ffvship_json_column_is_a_parse_error() {
        let mut sink = BufferSink::default();
        assert!(parse_vship_json(SSIMULACRA2_JSON.as_bytes(), MetricId::PsnrY, &mut sink).is_err());
    }

    #[test]
    fn malformed_json_is_a_parse_error_not_a_panic() {
        let mut sink = BufferSink::default();
        assert!(parse_vship_json("not json".as_bytes(), MetricId::Ssimulacra2, &mut sink).is_err());
    }

    #[test]
    fn an_empty_array_gives_no_cvvdp_value() {
        let mut sink = BufferSink::default();
        assert!(parse_vship_json("[]".as_bytes(), MetricId::Cvvdp, &mut sink).is_err());
    }
}
