//! The metric registry.
//!
//! This table is the center of the data model. Every other part reads it. A new metric is
//! one new row plus one new match arm.

use crate::capability::{BinaryId, Inventory, LaneKind, Requirement};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// One measured series.
///
/// Two identities are deliberate. `vmaf_neg_v0` carries `_v0` because VMAF v1 is already
/// NEG. `cambi` and `vmaf_v1_cambi` are two different measurements with two different
/// ranges, so they never share a column and never share a graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetricId {
    /// Peak signal to noise ratio, luma plane.
    PsnrY,
    /// Structural similarity, all planes.
    SsimAll,
    /// Extended perceptually weighted peak signal to noise ratio, the lowest plane average.
    XpsnrMin,
    /// Multi-scale structural similarity.
    MsSsim,
    /// Peak signal to noise ratio with a human visual system model.
    PsnrHvs,
    /// Color difference.
    Ciede2000,
    /// VMAF with a version 1 model.
    Vmaf,
    /// VMAF with a version 0 model.
    VmafV0,
    /// VMAF with a version 0 model in no-enhancement-gain mode.
    VmafNegV0,
    /// CAMBI on its own, range 0 to 24.
    Cambi,
    /// CAMBI inside a VMAF version 1 model, clipped at 17.
    VmafV1Cambi,
    /// SSIMULACRA 2.
    Ssimulacra2,
    /// Butteraugli, 3-norm.
    Butteraugli3Norm,
    /// Butteraugli, maximum norm.
    ButteraugliMax,
    /// ColorVideoVDP.
    Cvvdp,
}

impl MetricId {
    /// The stable key. This is the CSV column name and the JSON name.
    pub fn key(self) -> &'static str {
        match self {
            Self::PsnrY => "psnr_y",
            Self::SsimAll => "ssim_all",
            Self::XpsnrMin => "xpsnr_min",
            Self::MsSsim => "ms_ssim",
            Self::PsnrHvs => "psnr_hvs",
            Self::Ciede2000 => "ciede2000",
            Self::Vmaf => "vmaf",
            Self::VmafV0 => "vmaf_v0",
            Self::VmafNegV0 => "vmaf_neg_v0",
            Self::Cambi => "cambi",
            Self::VmafV1Cambi => "vmaf_v1_cambi",
            Self::Ssimulacra2 => "ssimulacra2",
            Self::Butteraugli3Norm => "butteraugli_3norm",
            Self::ButteraugliMax => "butteraugli_max",
            Self::Cvvdp => "cvvdp",
        }
    }

    /// Reads a key back.
    pub fn from_key(key: &str) -> Option<Self> {
        REGISTRY
            .iter()
            .find(|def| def.id.key() == key)
            .map(|def| def.id)
    }

    /// The registry row for this metric.
    pub fn def(self) -> &'static MetricDef {
        REGISTRY
            .iter()
            .find(|def| def.id == self)
            .expect("every metric id has a registry row")
    }
}

impl fmt::Display for MetricId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl Serialize for MetricId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for MetricId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Self::from_key(&key).ok_or_else(|| D::Error::custom(format!("unknown metric key: {key}")))
    }
}

/// Which way is better.
///
/// Four parts of the tool read this field. A wrong value gives a wrong answer in all four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// A high value is a good result.
    HigherIsBetter,
    /// A low value is a good result.
    LowerIsBetter,
    /// Not settled by measurement yet.
    Unknown,
}

impl Direction {
    /// The percentile at the bad end of the metric.
    pub fn bad_end(self) -> Percentile {
        match self {
            Self::LowerIsBetter => Percentile::P95,
            _ => Percentile::P5,
        }
    }

    /// The corner label of the graph. The tool never flips an axis.
    pub fn label(self) -> &'static str {
        match self {
            Self::HigherIsBetter => "higher is better",
            Self::LowerIsBetter => "lower is better",
            Self::Unknown => "direction not settled",
        }
    }
}

/// Which percentile the report shows at the bad end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Percentile {
    /// The 5th percentile.
    P5,
    /// The 95th percentile.
    P95,
}

/// Whether the harmonic mean is defensible for this metric.
///
/// The harmonic mean fails on zero and on negative numbers. The tool writes `null` and
/// gives the reason rather than a value that it cannot defend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarmonicMean {
    /// The tool computes it.
    Allowed,
    /// The tool computes it only when every value is above zero.
    AllowedAboveZero,
    /// The tool writes `null`, with this reason.
    Blocked(&'static str),
}

/// The unit of a metric value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    /// Decibels.
    Db,
    /// A ratio with no unit.
    Ratio,
    /// A score with no unit.
    Score,
    /// An index with no unit.
    Index,
    /// A perceptual distance.
    Distance,
    /// Just-objectionable-difference units.
    Jod,
}

impl Unit {
    /// The suffix to put after a number. An empty string for a unit with no suffix.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Db => " dB",
            Self::Jod => " JOD",
            _ => "",
        }
    }
}

/// The group label above a block of checkboxes in the metric list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MetricGroup {
    /// Filters that FFmpeg holds on its own.
    Ffmpeg,
    /// Features of the `libvmaf` filter.
    LibVmaf,
    /// Metrics that only FFVship gives.
    Ffvship,
}

impl MetricGroup {
    /// Every group, in the order that the metric list shows them.
    pub const ALL: [MetricGroup; 3] = [
        MetricGroup::Ffmpeg,
        MetricGroup::LibVmaf,
        MetricGroup::Ffvship,
    ];

    /// The label above the block.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ffmpeg => "FFmpeg",
            Self::LibVmaf => "libvmaf",
            Self::Ffvship => "FFVship",
        }
    }
}

/// One way to measure one metric.
///
/// The registry lists providers in order of preference. The first one that this machine
/// can run wins, and the run record names it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Provider {
    /// The binary that runs it.
    pub binary: BinaryId,
    /// The implementation name for the run record, for example `FFmpeg xpsnr filter`.
    pub implementation: &'static str,
    /// Everything that the binary must offer.
    pub requires: &'static [Requirement],
    /// Which lane it runs in.
    pub lane: LaneKind,
    /// Provisional cost in seconds for each frame at 1920x1080. Refer to `estimate`.
    pub seconds_per_frame_1080p: f32,
    /// Metrics that share this key run in one process, and the estimate counts them once.
    pub fuse_group: Option<&'static str>,
    /// An extra sentence for the disabled row, when the reason needs one.
    pub hint: Option<&'static str>,
}

/// One row of the registry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricDef {
    /// The metric.
    pub id: MetricId,
    /// The label on the checkbox and the graph tab.
    pub label: &'static str,
    /// Which block of the metric list holds it.
    pub group: MetricGroup,
    /// The unit of the value.
    pub unit: Unit,
    /// The range, as a sentence for the interface.
    pub range: &'static str,
    /// The low edge of the drawn vertical axis. This is a legibility window, not the
    /// full range of the metric. PSNR draws from 20 dB because nothing readable
    /// happens below it. A value outside the window is clipped, and a series with no
    /// value inside it makes the plot fit the data instead.
    pub plot_lo: f32,
    /// The high edge of the drawn vertical axis.
    pub plot_hi: f32,
    /// Which way is better.
    pub direction: Direction,
    /// Whether the harmonic mean is defensible.
    pub harmonic_mean: HarmonicMean,
    /// The ways to measure it, in order of preference.
    pub providers: &'static [Provider],
    /// Notes that the report attaches to every result for this metric.
    pub notes: &'static [&'static str],
}

// The cost values below are provisional, and the interface shows them as a range.
//
// One measured anchor holds the FFmpeg family: XPSNR runs at about real time at 1080p
// 24 fps, which is 0.042 seconds for each frame. The FFVship values divide a clip
// measurement by an assumed 500 frames, because the source measurement records clip
// time and not frame count. That assumption is the weak part of this table.
//
// A later milestone stores a measured frames-for-each-second value for each metric,
// lane and resolution on this machine, and the estimate prefers the measured value
// when there is one.

const FFMPEG_FAMILY: Option<&str> = Some("ffmpeg-family");
const VMAF_V1_RUN: Option<&str> = Some("vmaf-v1-run");
const BUTTERAUGLI_RUN: Option<&str> = Some("butteraugli-run");

/// The metric registry.
pub const REGISTRY: &[MetricDef] = &[
    MetricDef {
        id: MetricId::PsnrY,
        label: "PSNR",
        group: MetricGroup::Ffmpeg,
        unit: Unit::Db,
        range: "0 to about 60 dB",
        plot_lo: 20.0,
        plot_hi: 48.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[
            Provider {
                binary: BinaryId::Ffmpeg,
                implementation: "FFmpeg psnr filter",
                requires: &[Requirement::FfmpegFilter("psnr")],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.008,
                fuse_group: FFMPEG_FAMILY,
                hint: None,
            },
            Provider {
                binary: BinaryId::Ffmpeg,
                implementation: "libvmaf psnr feature",
                requires: &[Requirement::LibVmafFeature("psnr")],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.020,
                fuse_group: None,
                hint: None,
            },
        ],
        notes: &[],
    },
    MetricDef {
        id: MetricId::SsimAll,
        label: "SSIM",
        group: MetricGroup::Ffmpeg,
        unit: Unit::Ratio,
        range: "-1 to 1",
        plot_lo: 0.7,
        plot_hi: 1.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::AllowedAboveZero,
        providers: &[
            Provider {
                binary: BinaryId::Ffmpeg,
                implementation: "FFmpeg ssim filter",
                requires: &[Requirement::FfmpegFilter("ssim")],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.012,
                fuse_group: FFMPEG_FAMILY,
                hint: None,
            },
            Provider {
                binary: BinaryId::Ffmpeg,
                implementation: "libvmaf float_ssim feature",
                requires: &[Requirement::LibVmafFeature("float_ssim")],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.025,
                fuse_group: None,
                hint: None,
            },
        ],
        notes: &[
            "The FFmpeg ssim filter and the libvmaf SSIM feature give different numbers. The run record names the one that ran.",
        ],
    },
    MetricDef {
        id: MetricId::XpsnrMin,
        label: "XPSNR",
        group: MetricGroup::Ffmpeg,
        unit: Unit::Db,
        range: "dB, the lowest plane average",
        plot_lo: 22.0,
        plot_hi: 52.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[Provider {
            binary: BinaryId::Ffmpeg,
            implementation: "FFmpeg xpsnr filter",
            requires: &[Requirement::FfmpegFilter("xpsnr")],
            lane: LaneKind::Cpu,
            seconds_per_frame_1080p: 0.042,
            fuse_group: FFMPEG_FAMILY,
            hint: Some("FFmpeg 7.1 is the first version with the xpsnr filter."),
        }],
        notes: &[],
    },
    MetricDef {
        id: MetricId::MsSsim,
        label: "MS-SSIM",
        group: MetricGroup::LibVmaf,
        unit: Unit::Ratio,
        range: "0 to 1",
        plot_lo: 0.7,
        plot_hi: 1.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[Provider {
            binary: BinaryId::Ffmpeg,
            implementation: "libvmaf float_ms_ssim feature",
            requires: &[Requirement::LibVmafFeature("float_ms_ssim")],
            lane: LaneKind::Cpu,
            seconds_per_frame_1080p: 0.020,
            fuse_group: None,
            hint: None,
        }],
        notes: &[],
    },
    MetricDef {
        id: MetricId::PsnrHvs,
        label: "PSNR-HVS",
        group: MetricGroup::LibVmaf,
        unit: Unit::Db,
        range: "dB",
        plot_lo: 20.0,
        plot_hi: 48.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[Provider {
            binary: BinaryId::Ffmpeg,
            implementation: "libvmaf psnr_hvs feature",
            requires: &[Requirement::LibVmafFeature("psnr_hvs")],
            lane: LaneKind::Cpu,
            seconds_per_frame_1080p: 0.030,
            fuse_group: None,
            hint: None,
        }],
        notes: &[],
    },
    MetricDef {
        id: MetricId::Ciede2000,
        label: "CIEDE2000",
        group: MetricGroup::LibVmaf,
        unit: Unit::Index,
        range: "0 to about 50, unbounded upward, infinite at a perfect match",
        // Measured on this project's own test content: TEST_A against TEST_B
        // gives 56.7, and TEST_A against a CRF 45 encode gives 44.0.
        plot_lo: 35.0,
        plot_hi: 65.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[Provider {
            binary: BinaryId::Ffmpeg,
            implementation: "libvmaf ciede feature",
            requires: &[Requirement::LibVmafFeature("ciede")],
            lane: LaneKind::Cpu,
            seconds_per_frame_1080p: 0.020,
            fuse_group: None,
            hint: None,
        }],
        notes: &[
            "Measured for real: an identical pair gives an infinite value, and a differing pair gives a finite, lower value, the same shape PSNR has. Higher is better.",
        ],
    },
    MetricDef {
        id: MetricId::Vmaf,
        label: "VMAF v1",
        group: MetricGroup::LibVmaf,
        unit: Unit::Score,
        range: "0 to 100",
        plot_lo: 45.0,
        plot_hi: 100.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[
            Provider {
                binary: BinaryId::Ffmpeg,
                implementation: "FFmpeg libvmaf filter",
                requires: &[Requirement::FfmpegFilter("libvmaf")],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.080,
                fuse_group: VMAF_V1_RUN,
                hint: None,
            },
            Provider {
                binary: BinaryId::Vmaf,
                implementation: "Netflix vmaf binary",
                requires: &[],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.110,
                fuse_group: None,
                hint: None,
            },
        ],
        notes: &["VMAF v1 is already NEG. Every v1 model holds adm_enhn_gain_limit 1.0."],
    },
    MetricDef {
        id: MetricId::VmafV0,
        label: "VMAF v0",
        group: MetricGroup::LibVmaf,
        unit: Unit::Score,
        range: "0 to 100",
        plot_lo: 45.0,
        plot_hi: 100.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[
            Provider {
                binary: BinaryId::Ffmpeg,
                implementation: "FFmpeg libvmaf filter",
                requires: &[Requirement::FfmpegFilter("libvmaf")],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.080,
                fuse_group: None,
                hint: None,
            },
            Provider {
                binary: BinaryId::Vmaf,
                implementation: "Netflix vmaf binary",
                requires: &[],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.110,
                fuse_group: None,
                hint: None,
            },
        ],
        notes: &[
            "VMAF v0 under-predicts quality at 60 fps and over-predicts it on very high motion. v1 corrects both.",
        ],
    },
    MetricDef {
        id: MetricId::VmafNegV0,
        label: "VMAF NEG (v0)",
        group: MetricGroup::LibVmaf,
        unit: Unit::Score,
        range: "0 to 100",
        plot_lo: 45.0,
        plot_hi: 100.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[Provider {
            binary: BinaryId::Ffmpeg,
            implementation: "FFmpeg libvmaf filter, v0 model in NEG mode",
            requires: &[Requirement::FfmpegFilter("libvmaf")],
            lane: LaneKind::Cpu,
            seconds_per_frame_1080p: 0.080,
            fuse_group: None,
            hint: Some(
                "NEG applies to v0 models only. There is no v1 NEG model, and none is needed.",
            ),
        }],
        notes: &["NEG removes the enhancement gain. A sharpening filter cannot lift this score."],
    },
    MetricDef {
        id: MetricId::Cambi,
        label: "CAMBI",
        group: MetricGroup::LibVmaf,
        unit: Unit::Index,
        range: "0 to 24",
        plot_lo: 0.0,
        plot_hi: 24.0,
        direction: Direction::LowerIsBetter,
        harmonic_mean: HarmonicMean::Blocked(
            "CAMBI starts at 0, and the harmonic mean fails on zero.",
        ),
        providers: &[Provider {
            binary: BinaryId::Ffmpeg,
            implementation: "libvmaf cambi feature, speedup 0",
            requires: &[Requirement::LibVmafFeature("cambi")],
            lane: LaneKind::Cpu,
            seconds_per_frame_1080p: 0.040,
            fuse_group: None,
            hint: None,
        }],
        notes: &[
            "Standalone CAMBI runs with cambi_high_res_speedup 0 and a range of 0 to 24. It is not the CAMBI inside a VMAF v1 model.",
        ],
    },
    MetricDef {
        id: MetricId::VmafV1Cambi,
        label: "CAMBI in VMAF v1",
        group: MetricGroup::LibVmaf,
        unit: Unit::Index,
        range: "0 to 17",
        plot_lo: 0.0,
        plot_hi: 17.0,
        direction: Direction::LowerIsBetter,
        harmonic_mean: HarmonicMean::Blocked(
            "CAMBI starts at 0, and the harmonic mean fails on zero.",
        ),
        providers: &[Provider {
            binary: BinaryId::Ffmpeg,
            implementation: "libvmaf v1 model, cambi_high_res_speedup 1080",
            requires: &[Requirement::FfmpegFilter("libvmaf")],
            lane: LaneKind::Cpu,
            seconds_per_frame_1080p: 0.080,
            fuse_group: VMAF_V1_RUN,
            hint: None,
        }],
        notes: &[
            "A v1 model runs CAMBI with cambi_high_res_speedup 1080 and clips it at 17.0. This is a different number from standalone CAMBI.",
        ],
    },
    MetricDef {
        id: MetricId::Ssimulacra2,
        label: "SSIMULACRA 2",
        group: MetricGroup::Ffvship,
        unit: Unit::Score,
        range: "below 0 to 100",
        plot_lo: 35.0,
        plot_hi: 100.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Blocked(
            "SSIMULACRA 2 goes negative for strong distortion, and the harmonic mean fails on a negative value.",
        ),
        providers: &[
            Provider {
                binary: BinaryId::Ffvship,
                implementation: "FFVship SSIMULACRA2",
                requires: &[Requirement::VshipMetric("SSIMULACRA2")],
                lane: LaneKind::Gpu,
                seconds_per_frame_1080p: 0.017,
                fuse_group: None,
                hint: None,
            },
            Provider {
                binary: BinaryId::Ssimulacra2Rs,
                implementation: "ssimulacra2_rs, processor",
                requires: &[],
                lane: LaneKind::Cpu,
                seconds_per_frame_1080p: 0.144,
                fuse_group: None,
                hint: None,
            },
        ],
        notes: &["SSIMULACRA 2 has no temporal model."],
    },
    MetricDef {
        id: MetricId::Butteraugli3Norm,
        label: "Butteraugli 3-norm",
        group: MetricGroup::Ffvship,
        unit: Unit::Distance,
        range: "0 upward",
        plot_lo: 0.0,
        plot_hi: 12.0,
        direction: Direction::LowerIsBetter,
        harmonic_mean: HarmonicMean::Blocked(
            "Butteraugli starts at 0, and the harmonic mean fails on zero.",
        ),
        providers: &[Provider {
            binary: BinaryId::Ffvship,
            implementation: "FFVship BUTTERAUGLI",
            requires: &[Requirement::VshipMetric("BUTTERAUGLI")],
            lane: LaneKind::Gpu,
            seconds_per_frame_1080p: 0.077,
            fuse_group: BUTTERAUGLI_RUN,
            hint: None,
        }],
        notes: &[],
    },
    MetricDef {
        id: MetricId::ButteraugliMax,
        label: "Butteraugli max",
        group: MetricGroup::Ffvship,
        unit: Unit::Distance,
        range: "0 upward",
        plot_lo: 0.0,
        plot_hi: 12.0,
        direction: Direction::LowerIsBetter,
        harmonic_mean: HarmonicMean::Blocked(
            "Butteraugli starts at 0, and the harmonic mean fails on zero.",
        ),
        providers: &[Provider {
            binary: BinaryId::Ffvship,
            implementation: "FFVship BUTTERAUGLI",
            requires: &[Requirement::VshipMetric("BUTTERAUGLI")],
            lane: LaneKind::Gpu,
            seconds_per_frame_1080p: 0.077,
            fuse_group: BUTTERAUGLI_RUN,
            hint: None,
        }],
        notes: &[
            "The maximum norm finds the one broken frame. Use it for a final transparency check.",
        ],
    },
    MetricDef {
        id: MetricId::Cvvdp,
        label: "ColorVideoVDP",
        group: MetricGroup::Ffvship,
        unit: Unit::Jod,
        range: "0 to 10 JOD",
        plot_lo: 0.0,
        plot_hi: 10.0,
        direction: Direction::HigherIsBetter,
        harmonic_mean: HarmonicMean::Allowed,
        providers: &[Provider {
            binary: BinaryId::Ffvship,
            implementation: "FFVship CVVDP",
            requires: &[Requirement::VshipMetric("CVVDP")],
            lane: LaneKind::Gpu,
            seconds_per_frame_1080p: 0.044,
            fuse_group: None,
            hint: None,
        }],
        notes: &[],
    },
];

/// Whether a metric can run on this machine, and why not when it cannot.
#[derive(Debug, Clone, PartialEq)]
pub struct Availability {
    /// The provider that will run, when there is one.
    pub provider: Option<&'static Provider>,
    /// The sentence for the disabled row. Present only when no provider can run.
    pub reason: Option<String>,
}

impl Availability {
    /// True when the metric can run.
    pub fn is_available(&self) -> bool {
        self.provider.is_some()
    }
}

/// Chooses the provider for one metric, or explains why none can run.
///
/// A metric with no back end is shown, disabled, with the binary that gives it. It is
/// never hidden. A hidden metric teaches nothing.
pub fn availability(id: MetricId, inventory: &Inventory) -> Availability {
    let def = id.def();

    for provider in def.providers {
        let found = inventory.has(provider.binary);
        let met = provider
            .requires
            .iter()
            .all(|requirement| inventory.satisfies(provider.binary, requirement));
        if found && met {
            return Availability {
                provider: Some(provider),
                reason: None,
            };
        }
    }

    let reason = def
        .providers
        .first()
        .map(|provider| explain(provider, inventory))
        .unwrap_or_else(|| "no back end gives this metric".to_string());

    Availability {
        provider: None,
        reason: Some(reason),
    }
}

/// Builds the sentence under a disabled metric row.
fn explain(provider: &Provider, inventory: &Inventory) -> String {
    let name = provider.binary.display_name();
    let mut reason = if !inventory.has(provider.binary) {
        format!("needs {name}. Set the path in Settings.")
    } else {
        match provider
            .requires
            .iter()
            .find(|requirement| !inventory.satisfies(provider.binary, requirement))
        {
            Some(requirement) => format!("{}.", requirement.reason()),
            None => format!("needs {name}. Set the path in Settings."),
        }
    };
    if let Some(hint) = provider.hint {
        reason.push(' ');
        reason.push_str(hint);
    }
    reason
}

/// Every metric of one group, in registry order.
pub fn metrics_in_group(group: MetricGroup) -> impl Iterator<Item = &'static MetricDef> {
    REGISTRY.iter().filter(move |def| def.group == group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{BinaryCapabilities, FoundBinary};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    fn inventory_with_ffmpeg(filters: &[&str], features: &[&str]) -> Inventory {
        let mut inventory = Inventory::new();
        inventory.insert(FoundBinary {
            id: BinaryId::Ffmpeg,
            path: PathBuf::from("ffmpeg"),
            sha256: "0".into(),
            capabilities: BinaryCapabilities {
                version: Some("7.1".into()),
                version_parts: Some((7, 1)),
                ffmpeg_filters: names(filters),
                libvmaf_features: names(features),
                vship_metrics: BTreeSet::new(),
            },
        });
        inventory
    }

    #[test]
    fn every_registry_key_is_unique_and_reads_back() {
        let mut seen = BTreeSet::new();
        for def in REGISTRY {
            assert!(seen.insert(def.id.key()), "duplicate key {}", def.id.key());
            assert_eq!(MetricId::from_key(def.id.key()), Some(def.id));
        }
        assert_eq!(seen.len(), REGISTRY.len());
    }

    #[test]
    fn every_metric_has_at_least_one_provider() {
        for def in REGISTRY {
            assert!(
                !def.providers.is_empty(),
                "{} has no provider",
                def.id.key()
            );
        }
    }

    #[test]
    fn the_bad_end_follows_the_direction() {
        assert_eq!(MetricId::Vmaf.def().direction.bad_end(), Percentile::P5);
        assert_eq!(MetricId::Cambi.def().direction.bad_end(), Percentile::P95);
        assert_eq!(
            MetricId::ButteraugliMax.def().direction.bad_end(),
            Percentile::P95
        );
    }

    #[test]
    fn every_plot_window_runs_low_to_high() {
        for def in REGISTRY {
            assert!(
                def.plot_lo < def.plot_hi,
                "{} draws from {} to {}",
                def.id.key(),
                def.plot_lo,
                def.plot_hi
            );
        }
    }

    #[test]
    fn a_low_is_better_window_is_not_stored_reversed() {
        let low: Vec<&MetricDef> = REGISTRY
            .iter()
            .filter(|def| def.direction == Direction::LowerIsBetter)
            .collect();
        assert!(!low.is_empty(), "the registry holds no low-is-better metric");
        assert_eq!(MetricId::Cambi.def().plot_lo, 0.0);
        assert_eq!(MetricId::Cambi.def().plot_hi, 24.0);
    }

    #[test]
    fn the_two_cambi_identities_stay_apart() {
        let standalone = MetricId::Cambi.def();
        let in_model = MetricId::VmafV1Cambi.def();
        assert_ne!(standalone.id, in_model.id);
        assert_ne!(standalone.range, in_model.range);
        assert_ne!(
            standalone.providers[0].implementation,
            in_model.providers[0].implementation
        );
    }

    #[test]
    fn the_harmonic_mean_is_blocked_where_a_value_can_reach_zero() {
        for id in [
            MetricId::Cambi,
            MetricId::VmafV1Cambi,
            MetricId::Ssimulacra2,
            MetricId::Butteraugli3Norm,
            MetricId::ButteraugliMax,
        ] {
            assert!(
                matches!(id.def().harmonic_mean, HarmonicMean::Blocked(_)),
                "{} must block the harmonic mean",
                id.key()
            );
        }
    }

    #[test]
    fn with_no_binaries_every_metric_is_disabled_with_a_reason() {
        let inventory = Inventory::new();
        for def in REGISTRY {
            let state = availability(def.id, &inventory);
            assert!(!state.is_available(), "{} must be disabled", def.id.key());
            let reason = state
                .reason
                .expect("a disabled metric always gives a reason");
            assert!(!reason.is_empty());
        }
    }

    #[test]
    fn colorvideovdp_names_ffvship_and_the_settings_panel() {
        let inventory = inventory_with_ffmpeg(&["psnr", "ssim", "libvmaf"], &["cambi"]);
        let state = availability(MetricId::Cvvdp, &inventory);
        assert_eq!(
            state.reason.as_deref(),
            Some("needs FFVship. Set the path in Settings.")
        );
    }

    #[test]
    fn xpsnr_names_the_ffmpeg_version_that_adds_it() {
        let inventory = inventory_with_ffmpeg(&["psnr", "ssim"], &[]);
        let state = availability(MetricId::XpsnrMin, &inventory);
        let reason = state.reason.unwrap();
        assert!(reason.contains("xpsnr"), "{reason}");
        assert!(reason.contains("7.1"), "{reason}");
    }

    #[test]
    fn psnr_falls_back_to_the_libvmaf_feature() {
        let inventory = inventory_with_ffmpeg(&["libvmaf"], &["psnr"]);
        let state = availability(MetricId::PsnrY, &inventory);
        assert_eq!(
            state.provider.unwrap().implementation,
            "libvmaf psnr feature"
        );
    }

    #[test]
    fn the_ffmpeg_filter_wins_over_the_libvmaf_feature() {
        let inventory = inventory_with_ffmpeg(&["psnr", "libvmaf"], &["psnr"]);
        let state = availability(MetricId::PsnrY, &inventory);
        assert_eq!(state.provider.unwrap().implementation, "FFmpeg psnr filter");
    }
}
