use std::io::BufRead;
use vqa_core::backend::{FrameSink, LogFormat};
use vqa_core::metric::MetricId;

pub fn parse_psnr_line(line: &str) -> Option<(u64, f32)> {
    let frame_number: u64 = value_after(line, "n:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let value: f32 = value_after(line, "psnr_y:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((frame_number - 1, value))
}

pub fn parse_ssim_line(line: &str) -> Option<(u64, f32)> {
    let frame_number: u64 = value_after(line, "n:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let value: f32 = value_after(line, "All:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((frame_number - 1, value))
}

// The xpsnr line has no key:value shape. Frame 25 of a real file also ends with a
// summary line, "XPSNR average, ...", that carries no frame number and must not
// become a 26th frame.
pub fn parse_xpsnr_line(line: &str) -> Option<(u64, f32)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.first() != Some(&"n:") {
        return None;
    }
    let frame_number: u64 = tokens.get(1)?.parse().ok()?;

    let mut channel_values = Vec::with_capacity(3);
    let mut index = 0;
    while index < tokens.len() {
        if matches!(tokens[index], "y:" | "u:" | "v:") {
            let value: f32 = tokens.get(index + 1)?.parse().ok()?;
            channel_values.push(value);
        }
        index += 1;
    }
    // A perfect match reports "inf" for every channel, so the lowest channel value can
    // itself be infinite. An empty channel list, and not an infinite value, is what
    // means "this line carried no reading".
    if channel_values.is_empty() {
        return None;
    }
    let lowest_channel = channel_values.into_iter().fold(f32::INFINITY, f32::min);
    Some((frame_number - 1, lowest_channel))
}

fn value_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let start = line.find(key)? + key.len();
    Some(&line[start..])
}

pub fn parse_stats_file(
    reader: impl BufRead,
    format: LogFormat,
    metric: MetricId,
    sink: &mut dyn FrameSink,
) -> vqa_core::Result<()> {
    let parse_line: fn(&str) -> Option<(u64, f32)> = match format {
        LogFormat::PsnrStats => parse_psnr_line,
        LogFormat::SsimStats => parse_ssim_line,
        LogFormat::XpsnrStats => parse_xpsnr_line,
        LogFormat::VmafCsv => {
            return Err(vqa_core::CoreError::parse(
                "stats file",
                "VMAF CSV needs the vmaf_csv parser",
            ));
        }
    };

    for line in reader.lines() {
        let line =
            line.map_err(|error| vqa_core::CoreError::parse("stats file", error.to_string()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((frame, value)) = parse_line(trimmed) else {
            if format == LogFormat::XpsnrStats {
                continue;
            }
            return Err(vqa_core::CoreError::parse(
                "stats file",
                format!("cannot read line: {trimmed}"),
            ));
        };
        sink.push(metric, frame, value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const PSNR_FIXTURE: &str = include_str!("../../tests/fixtures/psnr_fixture.log");
    const PSNR_IDENTITY_FIXTURE: &str =
        include_str!("../../tests/fixtures/psnr_identity_fixture.log");
    const SSIM_IDENTITY_FIXTURE: &str =
        include_str!("../../tests/fixtures/ssim_identity_fixture.log");
    const XPSNR_FIXTURE: &str = include_str!("../../tests/fixtures/xpsnr_fixture.log");

    struct RecordingSink {
        values: Vec<(u64, f32)>,
    }

    impl FrameSink for RecordingSink {
        fn push(&mut self, _metric: MetricId, frame: u64, value: f32) {
            self.values.push((frame, value));
        }
    }

    #[test]
    fn a_real_psnr_line_reads_the_frame_and_the_luma_value() {
        let first_line = PSNR_FIXTURE.lines().next().unwrap();
        assert_eq!(parse_psnr_line(first_line), Some((0, 8.08)));
    }

    #[test]
    fn a_file_measured_against_itself_gives_an_infinite_psnr() {
        let first_line = PSNR_IDENTITY_FIXTURE.lines().next().unwrap();
        let (_, value) = parse_psnr_line(first_line).unwrap();
        assert!(value.is_infinite());
    }

    #[test]
    fn a_file_measured_against_itself_gives_an_ssim_of_one() {
        let first_line = SSIM_IDENTITY_FIXTURE.lines().next().unwrap();
        assert_eq!(parse_ssim_line(first_line), Some((0, 1.0)));
    }

    #[test]
    fn a_real_xpsnr_line_reads_the_frame_and_the_lowest_channel() {
        let first_line = XPSNR_FIXTURE.lines().next().unwrap();
        let (frame, value) = parse_xpsnr_line(first_line).unwrap();
        assert_eq!(frame, 0);
        assert!((value - 5.8571).abs() < 0.001);
    }

    #[test]
    fn the_xpsnr_summary_line_is_not_a_frame() {
        let summary_line = XPSNR_FIXTURE
            .lines()
            .find(|line| line.starts_with("XPSNR average"))
            .unwrap();
        assert_eq!(parse_xpsnr_line(summary_line), None);
    }

    #[test]
    fn a_perfect_match_reports_an_infinite_value_and_is_still_a_frame() {
        let line = "n:    1  XPSNR y: inf  XPSNR u: inf  XPSNR v: inf";
        let (frame, value) = parse_xpsnr_line(line).unwrap();
        assert_eq!(frame, 0);
        assert!(value.is_infinite());
    }

    #[test]
    fn parsing_a_whole_xpsnr_file_skips_the_summary_line_and_the_blank_line() {
        let mut sink = RecordingSink { values: Vec::new() };
        parse_stats_file(
            Cursor::new(XPSNR_FIXTURE),
            LogFormat::XpsnrStats,
            MetricId::XpsnrMin,
            &mut sink,
        )
        .unwrap();
        assert_eq!(sink.values.len(), 25);
        assert_eq!(sink.values[0].0, 0);
        assert_eq!(sink.values[24].0, 24);
    }

    #[test]
    fn parsing_a_whole_psnr_file_gives_one_value_for_each_frame() {
        let mut sink = RecordingSink { values: Vec::new() };
        parse_stats_file(
            Cursor::new(PSNR_FIXTURE),
            LogFormat::PsnrStats,
            MetricId::PsnrY,
            &mut sink,
        )
        .unwrap();
        assert_eq!(sink.values.len(), 25);
    }

    #[test]
    fn a_line_that_a_psnr_reader_cannot_understand_is_a_hard_error() {
        let mut sink = RecordingSink { values: Vec::new() };
        let broken = "this is not a stats line";
        let result = parse_stats_file(
            Cursor::new(broken),
            LogFormat::PsnrStats,
            MetricId::PsnrY,
            &mut sink,
        );
        assert!(result.is_err());
    }
}
