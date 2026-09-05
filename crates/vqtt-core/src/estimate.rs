//! The estimated run time.
//!
//! Show a range, never a single number. The measured spread on one metric is a factor of
//! seven, so a single number is not honest.

use crate::capability::{Inventory, LaneKind};
use crate::metric::{MetricId, availability};
use std::collections::BTreeMap;

/// The pixel count that every cost value is anchored on.
const REFERENCE_PIXELS: f64 = 1920.0 * 1080.0;

/// The low end of the reported range.
const LOW_FACTOR: f64 = 0.75;

/// The high end of the reported range.
const HIGH_FACTOR: f64 = 1.4;

/// An estimated run time, as a range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunEstimate {
    /// The low end, in seconds.
    pub low_seconds: f64,
    /// The high end, in seconds.
    pub high_seconds: f64,
}

impl RunEstimate {
    /// The label beside the frame range, for example `about 12 to 18 min`.
    pub fn label(&self) -> String {
        let (low, high, unit) = if self.high_seconds < 90.0 {
            (self.low_seconds, self.high_seconds, "s")
        } else if self.high_seconds < 5400.0 {
            (self.low_seconds / 60.0, self.high_seconds / 60.0, "min")
        } else {
            (self.low_seconds / 3600.0, self.high_seconds / 3600.0, "h")
        };
        if unit == "h" {
            format!("about {low:.1} to {high:.1} {unit}")
        } else {
            format!(
                "about {} to {} {unit}",
                low.round().max(1.0),
                high.round().max(1.0)
            )
        }
    }
}

/// Estimates the run time of one comparison.
///
/// The two lanes run at the same time, so the estimate takes the larger of the two and
/// never the sum. Metrics that share one process are counted once, at the cost of the
/// slowest member.
///
/// Returns `None` when nothing can run.
pub fn estimate(
    metrics: &[MetricId],
    inventory: &Inventory,
    frames: u64,
    width: u32,
    height: u32,
    encodes: usize,
) -> Option<RunEstimate> {
    if metrics.is_empty() || frames == 0 || encodes == 0 {
        return None;
    }

    let pixel_scale = (f64::from(width) * f64::from(height)) / REFERENCE_PIXELS;
    let mut lane_totals: BTreeMap<LaneKind, f64> = BTreeMap::new();
    let mut fused: BTreeMap<(LaneKind, &'static str), f64> = BTreeMap::new();
    let mut ran_something = false;

    for id in metrics {
        let Some(provider) = availability(*id, inventory).provider else {
            continue;
        };
        ran_something = true;
        let cost = f64::from(provider.seconds_per_frame_1080p);
        match provider.fuse_group {
            Some(group) => {
                let slot = fused.entry((provider.lane, group)).or_insert(0.0);
                *slot = slot.max(cost);
            }
            None => *lane_totals.entry(provider.lane).or_insert(0.0) += cost,
        }
    }

    if !ran_something {
        return None;
    }

    for ((lane, _), cost) in fused {
        *lane_totals.entry(lane).or_insert(0.0) += cost;
    }

    let slowest_lane = lane_totals.values().copied().fold(0.0_f64, f64::max);
    let point = slowest_lane * frames as f64 * pixel_scale * encodes as f64;

    Some(RunEstimate {
        low_seconds: point * LOW_FACTOR,
        high_seconds: point * HIGH_FACTOR,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{BinaryCapabilities, BinaryId, FoundBinary};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    fn full_inventory() -> Inventory {
        let mut inventory = Inventory::new();
        inventory.insert(FoundBinary {
            id: BinaryId::Ffmpeg,
            path: PathBuf::from("ffmpeg"),
            sha256: "0".into(),
            capabilities: BinaryCapabilities {
                version: Some("7.1".into()),
                version_parts: Some((7, 1)),
                ffmpeg_filters: names(&["psnr", "ssim", "xpsnr", "libvmaf"]),
                libvmaf_features: names(&[
                    "cambi",
                    "psnr_hvs",
                    "ciede",
                    "float_ms_ssim",
                    "psnr",
                    "float_ssim",
                ]),
                vship_metrics: BTreeSet::new(),
            },
        });
        inventory.insert(FoundBinary {
            id: BinaryId::Ffvship,
            path: PathBuf::from("FFVship"),
            sha256: "1".into(),
            capabilities: BinaryCapabilities {
                version: Some("3.0.2".into()),
                version_parts: Some((3, 0)),
                ffmpeg_filters: BTreeSet::new(),
                libvmaf_features: BTreeSet::new(),
                vship_metrics: names(&["SSIMULACRA2", "BUTTERAUGLI", "CVVDP"]),
            },
        });
        inventory
    }

    #[test]
    fn nothing_selected_gives_no_estimate() {
        assert!(estimate(&[], &full_inventory(), 1000, 1920, 1080, 1).is_none());
    }

    #[test]
    fn a_metric_with_no_back_end_gives_no_estimate() {
        let empty = Inventory::new();
        assert!(estimate(&[MetricId::Vmaf], &empty, 1000, 1920, 1080, 1).is_none());
    }

    #[test]
    fn the_ffmpeg_family_counts_once_at_its_slowest_member() {
        let inventory = full_inventory();
        let one = estimate(&[MetricId::XpsnrMin], &inventory, 1000, 1920, 1080, 1).unwrap();
        let three = estimate(
            &[MetricId::PsnrY, MetricId::SsimAll, MetricId::XpsnrMin],
            &inventory,
            1000,
            1920,
            1080,
            1,
        )
        .unwrap();
        assert!((one.low_seconds - three.low_seconds).abs() < 1e-9);
    }

    #[test]
    fn the_two_lanes_take_the_larger_and_never_the_sum() {
        let inventory = full_inventory();
        let cpu = estimate(&[MetricId::Vmaf], &inventory, 1000, 1920, 1080, 1).unwrap();
        let gpu = estimate(&[MetricId::Ssimulacra2], &inventory, 1000, 1920, 1080, 1).unwrap();
        let both = estimate(
            &[MetricId::Vmaf, MetricId::Ssimulacra2],
            &inventory,
            1000,
            1920,
            1080,
            1,
        )
        .unwrap();
        let larger = cpu.low_seconds.max(gpu.low_seconds);
        assert!((both.low_seconds - larger).abs() < 1e-9);
        assert!(both.low_seconds < cpu.low_seconds + gpu.low_seconds);
    }

    #[test]
    fn four_times_the_pixels_cost_four_times_as_much() {
        let inventory = full_inventory();
        let hd = estimate(&[MetricId::Vmaf], &inventory, 1000, 1920, 1080, 1).unwrap();
        let uhd = estimate(&[MetricId::Vmaf], &inventory, 1000, 3840, 2160, 1).unwrap();
        assert!((uhd.low_seconds - hd.low_seconds * 4.0).abs() < 1e-6);
    }

    #[test]
    fn two_encodes_cost_twice_as_much() {
        let inventory = full_inventory();
        let one = estimate(&[MetricId::Vmaf], &inventory, 1000, 1920, 1080, 1).unwrap();
        let two = estimate(&[MetricId::Vmaf], &inventory, 1000, 1920, 1080, 2).unwrap();
        assert!((two.low_seconds - one.low_seconds * 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_two_butteraugli_values_come_from_one_run() {
        let inventory = full_inventory();
        let one = estimate(&[MetricId::ButteraugliMax], &inventory, 1000, 1920, 1080, 1).unwrap();
        let two = estimate(
            &[MetricId::ButteraugliMax, MetricId::Butteraugli3Norm],
            &inventory,
            1000,
            1920,
            1080,
            1,
        )
        .unwrap();
        assert!((one.low_seconds - two.low_seconds).abs() < 1e-9);
    }

    #[test]
    fn the_label_always_gives_a_range() {
        let estimate = RunEstimate {
            low_seconds: 720.0,
            high_seconds: 1080.0,
        };
        assert_eq!(estimate.label(), "about 12 to 18 min");
        let short = RunEstimate {
            low_seconds: 20.0,
            high_seconds: 40.0,
        };
        assert_eq!(short.label(), "about 20 to 40 s");
        let long = RunEstimate {
            low_seconds: 7200.0,
            high_seconds: 12600.0,
        };
        assert_eq!(long.label(), "about 2.0 to 3.5 h");
    }
}
