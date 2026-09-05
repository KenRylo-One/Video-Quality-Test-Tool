//! What each back-end binary is, and what the tool found on this machine.
//!
//! The tool ships no binary. The user supplies them, and that is what keeps the license
//! free to choose.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Which lane a measurement runs in.
///
/// The two lanes run at the same time, so the run estimate takes the larger of the two
/// and never the sum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaneKind {
    /// The processor lane.
    Cpu,
    /// The graphics card lane.
    Gpu,
}

/// One back-end program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BinaryId {
    /// The FFmpeg command line tool.
    #[serde(rename = "ffmpeg")]
    Ffmpeg,
    /// The FFmpeg probe tool.
    #[serde(rename = "ffprobe")]
    Ffprobe,
    /// The Netflix `vmaf` command line tool.
    #[serde(rename = "vmaf")]
    Vmaf,
    /// FFVship, from Codeberg.
    #[serde(rename = "ffvship")]
    Ffvship,
    /// The processor fallback for SSIMULACRA 2.
    #[serde(rename = "ssimulacra2_rs")]
    Ssimulacra2Rs,
}

impl BinaryId {
    /// Every binary that the tool looks for, in the order that Settings lists them.
    pub const ALL: [BinaryId; 5] = [
        BinaryId::Ffmpeg,
        BinaryId::Ffprobe,
        BinaryId::Vmaf,
        BinaryId::Ssimulacra2Rs,
        BinaryId::Ffvship,
    ];

    /// The name to show, with the letter case that the project uses.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
            Self::Vmaf => "vmaf",
            Self::Ffvship => "FFVship",
            Self::Ssimulacra2Rs => "ssimulacra2_rs",
        }
    }

    /// The file name to look for on the PATH. Windows adds `.exe` in the discovery layer.
    pub fn command_name(self) -> &'static str {
        self.display_name()
    }

    /// What the tool loses without this binary.
    pub fn provides(self) -> &'static str {
        match self {
            Self::Ffmpeg => "PSNR, SSIM, XPSNR, and every libvmaf metric",
            Self::Ffprobe => "the media information of every file",
            Self::Vmaf => "VMAF, when FFmpeg has no libvmaf filter",
            Self::Ffvship => "SSIMULACRA 2, Butteraugli and ColorVideoVDP",
            Self::Ssimulacra2Rs => "SSIMULACRA 2 on a machine with no supported graphics card",
        }
    }

    /// The site that has it. The tool never downloads anything.
    pub fn source(self) -> &'static str {
        match self {
            Self::Ffmpeg | Self::Ffprobe => "ffmpeg.org",
            Self::Vmaf => "github.com/Netflix/vmaf",
            Self::Ffvship => "codeberg.org/Line-fr/Vship",
            Self::Ssimulacra2Rs => "github.com/rust-av/ssimulacra2_bin",
        }
    }

    /// The address of that site.
    ///
    /// The window opens this in the browser of the operating system. The tool itself
    /// uses the network for nothing, which is NFR-7.
    pub fn source_url(self) -> &'static str {
        match self {
            Self::Ffmpeg | Self::Ffprobe => "https://ffmpeg.org/download.html",
            Self::Vmaf => "https://github.com/Netflix/vmaf",
            Self::Ffvship => "https://codeberg.org/Line-fr/Vship",
            Self::Ssimulacra2Rs => "https://github.com/rust-av/ssimulacra2_bin",
        }
    }

    /// What the build must have, when the program alone is not enough.
    pub fn source_requirement(self) -> Option<&'static str> {
        match self {
            Self::Ffmpeg | Self::Ffprobe => {
                Some("Version 7.1 or later, built with --enable-libvmaf.")
            }
            _ => None,
        }
    }

    /// True when the tool cannot do useful work without it.
    pub fn is_essential(self) -> bool {
        matches!(self, Self::Ffmpeg | Self::Ffprobe)
    }
}

/// One thing that a metric needs from a binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// An FFmpeg filter of that name must be present.
    FfmpegFilter(&'static str),
    /// The `libvmaf` filter must offer that feature.
    LibVmafFeature(&'static str),
    /// FFVship must offer that metric name.
    VshipMetric(&'static str),
}

impl Requirement {
    /// A short sentence for the disabled row in the metric list.
    pub fn reason(&self) -> String {
        match self {
            Self::FfmpegFilter(name) => format!("needs the FFmpeg {name} filter"),
            Self::LibVmafFeature(name) => format!("needs the libvmaf {name} feature"),
            Self::VshipMetric(name) => format!("needs the FFVship {name} metric"),
        }
    }
}

/// What one binary on this machine can do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryCapabilities {
    /// The version string, as the binary printed it.
    pub version: Option<String>,
    /// The major and minor version, when the tool could read them.
    pub version_parts: Option<(u32, u32)>,
    /// The filter names that `ffmpeg -filters` listed.
    pub ffmpeg_filters: BTreeSet<String>,
    /// The feature names that `ffmpeg -h filter=libvmaf` listed.
    pub libvmaf_features: BTreeSet<String>,
    /// The metric names that `FFVship --help` listed.
    pub vship_metrics: BTreeSet<String>,
}

impl BinaryCapabilities {
    /// The short version label for the Settings row.
    pub fn version_label(&self) -> &str {
        self.version.as_deref().unwrap_or("unknown version")
    }
}

/// One binary that the tool found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundBinary {
    /// Which binary this is.
    pub id: BinaryId,
    /// Where it is.
    pub path: PathBuf,
    /// The SHA-256 hash of the file. The capability cache is keyed on this.
    pub sha256: String,
    /// What it can do.
    pub capabilities: BinaryCapabilities,
}

/// Every binary that the tool found on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    found: BTreeMap<BinaryId, FoundBinary>,
}

impl Inventory {
    /// An empty inventory. This is what a new user has, and the window must still open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one binary.
    pub fn insert(&mut self, binary: FoundBinary) {
        self.found.insert(binary.id, binary);
    }

    /// Forgets one binary.
    pub fn remove(&mut self, id: BinaryId) {
        self.found.remove(&id);
    }

    /// Reads one binary.
    pub fn get(&self, id: BinaryId) -> Option<&FoundBinary> {
        self.found.get(&id)
    }

    /// True when the tool found that binary.
    pub fn has(&self, id: BinaryId) -> bool {
        self.found.contains_key(&id)
    }

    /// True when the inventory holds nothing at all. This opens the first-run panel.
    pub fn is_empty(&self) -> bool {
        self.found.is_empty()
    }

    /// Every binary that the tool found.
    pub fn iter(&self) -> impl Iterator<Item = &FoundBinary> {
        self.found.values()
    }

    /// True when that binary meets the requirement.
    pub fn satisfies(&self, id: BinaryId, requirement: &Requirement) -> bool {
        let Some(binary) = self.found.get(&id) else {
            return false;
        };
        match requirement {
            Requirement::FfmpegFilter(name) => binary.capabilities.ffmpeg_filters.contains(*name),
            Requirement::LibVmafFeature(name) => {
                binary.capabilities.ffmpeg_filters.contains("libvmaf")
                    && binary.capabilities.libvmaf_features.contains(*name)
            }
            Requirement::VshipMetric(name) => binary.capabilities.vship_metrics.contains(*name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ffmpeg_with(filters: &[&str], features: &[&str]) -> FoundBinary {
        FoundBinary {
            id: BinaryId::Ffmpeg,
            path: PathBuf::from("/usr/bin/ffmpeg"),
            sha256: "0000".into(),
            capabilities: BinaryCapabilities {
                version: Some("7.1".into()),
                version_parts: Some((7, 1)),
                ffmpeg_filters: filters.iter().map(|f| f.to_string()).collect(),
                libvmaf_features: features.iter().map(|f| f.to_string()).collect(),
                vship_metrics: BTreeSet::new(),
            },
        }
    }

    #[test]
    fn an_empty_inventory_satisfies_nothing() {
        let inventory = Inventory::new();
        assert!(inventory.is_empty());
        assert!(!inventory.satisfies(BinaryId::Ffmpeg, &Requirement::FfmpegFilter("psnr")));
    }

    #[test]
    fn a_filter_requirement_reads_the_filter_list() {
        let mut inventory = Inventory::new();
        inventory.insert(ffmpeg_with(&["psnr", "ssim"], &[]));
        assert!(inventory.satisfies(BinaryId::Ffmpeg, &Requirement::FfmpegFilter("psnr")));
        assert!(!inventory.satisfies(BinaryId::Ffmpeg, &Requirement::FfmpegFilter("xpsnr")));
    }

    #[test]
    fn a_libvmaf_feature_also_needs_the_libvmaf_filter() {
        let mut inventory = Inventory::new();
        inventory.insert(ffmpeg_with(&["psnr"], &["cambi"]));
        assert!(!inventory.satisfies(BinaryId::Ffmpeg, &Requirement::LibVmafFeature("cambi")));

        inventory.insert(ffmpeg_with(&["psnr", "libvmaf"], &["cambi"]));
        assert!(inventory.satisfies(BinaryId::Ffmpeg, &Requirement::LibVmafFeature("cambi")));
    }
}
