//! Reading what a back-end binary can do.
//!
//! Every function here is pure. A test gives captured output and needs no binary.

use std::collections::BTreeSet;

/// Reads the version out of the first line that a binary printed.
///
/// It covers three shapes: `ffmpeg version n9.0.1 Copyright ...`, a bare `3.2.0` from the
/// `vmaf` binary, and `name 0.5.0`.
pub fn parse_version(text: &str) -> Option<String> {
    let line = text.lines().find(|line| !line.trim().is_empty())?.trim();

    let raw = match line.split_whitespace().collect::<Vec<_>>().as_slice() {
        [] => return None,
        [only] => (*only).to_string(),
        tokens => {
            let after_word = tokens
                .iter()
                .position(|token| token.eq_ignore_ascii_case("version"))
                .and_then(|index| tokens.get(index + 1))
                .map(|token| (*token).to_string());
            match after_word {
                Some(value) => value,
                None => (*tokens.last().unwrap()).to_string(),
            }
        }
    };

    let trimmed = raw.trim_start_matches('v');
    let trimmed = match trimmed.strip_prefix('n') {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => trimmed,
    };

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Reads the major and minor number out of a version string.
///
/// A build from a git snapshot carries no number, and this returns `None` for it.
pub fn parse_version_parts(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split(['.', '-', '_']);
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

/// Reads the filter names out of `ffmpeg -hide_banner -filters`.
///
/// A line holds the flags, the name, the stream shape and the description:
/// ` TS xpsnr             VV->V      Calculate ...`.
pub fn parse_ffmpeg_filters(text: &str) -> BTreeSet<String> {
    let mut filters = BTreeSet::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 3 {
            continue;
        }
        let flags = tokens[0];
        if flags.len() > 3 || !flags.chars().all(|c| ".TSC".contains(c)) {
            continue;
        }
        if !tokens[2].contains("->") {
            continue;
        }
        filters.insert(tokens[1].to_string());
    }
    filters
}

/// The `libvmaf` features that the tool asks for.
///
/// `ffmpeg -h filter=libvmaf` prints one generic `feature <string>` option and no list of
/// names, so the capability probe cannot read the real set. The tool therefore assumes
/// this set whenever the `libvmaf` filter is present.
///
/// A later milestone builds the `libvmaf` back end. It must replace this assumption
/// with a real answer, because a missing feature gives an error at run time and not
/// before the run.
pub const ASSUMED_LIBVMAF_FEATURES: &[&str] = &[
    "psnr",
    "float_ssim",
    "float_ms_ssim",
    "psnr_hvs",
    "ciede",
    "cambi",
];

/// The assumed feature set, or nothing when FFmpeg holds no `libvmaf` filter.
pub fn libvmaf_features(filters: &BTreeSet<String>) -> BTreeSet<String> {
    if filters.contains("libvmaf") {
        ASSUMED_LIBVMAF_FEATURES
            .iter()
            .map(|name| name.to_string())
            .collect()
    } else {
        BTreeSet::new()
    }
}

/// The FFVship metric names that the tool asks for.
pub const KNOWN_VSHIP_METRICS: &[&str] = &["SSIMULACRA2", "BUTTERAUGLI", "CVVDP"];

/// Reads the metric names out of the FFVship help text.
///
/// When the help text names none of them, the tool assumes all three. FFVship exists to
/// give these three metrics, so a binary that names none is a parser problem and not a
/// reason to hide a metric. A later milestone replaces this with a measured answer.
pub fn parse_vship_metrics(help: &str) -> BTreeSet<String> {
    let upper = help.to_ascii_uppercase();
    let found: BTreeSet<String> = KNOWN_VSHIP_METRICS
        .iter()
        .filter(|name| upper.contains(*name))
        .map(|name| name.to_string())
        .collect();

    if found.is_empty() {
        KNOWN_VSHIP_METRICS
            .iter()
            .map(|name| name.to_string())
            .collect()
    } else {
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILTER_OUTPUT: &str = "Filters:
  T.. = Timeline support
  .S. = Slice threading
  ------
 TS aap               AA->A      Apply Affine Projection algorithm.
 .. libvmaf           VV->V      Calculate the VMAF between two video streams.
 TS psnr              VV->V      Calculate the PSNR between two video streams.
 TS ssim              VV->V      Calculate the SSIM between two video streams.
 .. ssim360           VV->V      Calculate the SSIM between two 360 video streams.
 T. xpsnr             VV->V      Calculate the extended PSNR.
 ..C scale            V->V       Scale the input video size.
";

    #[test]
    fn reads_every_filter_name() {
        let filters = parse_ffmpeg_filters(FILTER_OUTPUT);
        assert!(filters.contains("psnr"));
        assert!(filters.contains("ssim"));
        assert!(filters.contains("xpsnr"));
        assert!(filters.contains("libvmaf"));
        assert!(filters.contains("scale"));
        assert!(!filters.contains("Filters:"));
        assert!(!filters.contains("------"));
    }

    #[test]
    fn keeps_ssim360_apart_from_ssim() {
        let filters = parse_ffmpeg_filters(FILTER_OUTPUT);
        assert!(filters.contains("ssim360"));
        assert!(filters.contains("ssim"));
    }

    #[test]
    fn reads_the_ffmpeg_version() {
        let text = "ffmpeg version n9.0.1 Copyright (c) 2000-2026 the FFmpeg developers";
        assert_eq!(parse_version(text).as_deref(), Some("9.0.1"));
        assert_eq!(parse_version_parts("9.0.1"), Some((9, 0)));
    }

    #[test]
    fn reads_a_static_build_version() {
        let text = "ffmpeg version 7.1.1-static https://johnvansickle.com/ffmpeg/";
        assert_eq!(parse_version(text).as_deref(), Some("7.1.1-static"));
        assert_eq!(parse_version_parts("7.1.1-static"), Some((7, 1)));
    }

    #[test]
    fn reads_the_bare_version_of_the_vmaf_binary() {
        assert_eq!(parse_version("3.2.0\n").as_deref(), Some("3.2.0"));
        assert_eq!(parse_version_parts("3.2.0"), Some((3, 2)));
    }

    #[test]
    fn a_git_snapshot_gives_no_version_number() {
        let text = "ffmpeg version N-109421-g8f0e2f4 Copyright (c) 2000-2026";
        assert_eq!(parse_version(text).as_deref(), Some("N-109421-g8f0e2f4"));
        assert_eq!(parse_version_parts("N-109421-g8f0e2f4"), None);
    }

    #[test]
    fn the_libvmaf_feature_set_follows_the_filter() {
        let with = parse_ffmpeg_filters(FILTER_OUTPUT);
        assert!(libvmaf_features(&with).contains("cambi"));

        let without: BTreeSet<String> = ["psnr".to_string()].into_iter().collect();
        assert!(libvmaf_features(&without).is_empty());
    }

    #[test]
    fn reads_the_vship_metric_names() {
        let help = "Usage: FFVship -m SSIMULACRA2 ... metrics: SSIMULACRA2, BUTTERAUGLI";
        let metrics = parse_vship_metrics(help);
        assert!(metrics.contains("SSIMULACRA2"));
        assert!(metrics.contains("BUTTERAUGLI"));
        assert!(!metrics.contains("CVVDP"));
    }

    #[test]
    fn help_text_that_names_nothing_falls_back_to_all_three() {
        let metrics = parse_vship_metrics("Usage: FFVship [options]");
        assert_eq!(metrics.len(), 3);
    }
}
