//! One session: the settings, what the tool found, and one comparison.

use crate::cache::CapabilityCache;
use crate::settings::Settings;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use vqa_backends::ffprobe::FfprobeProbe;
use vqa_backends::{discovery, hash};
use vqa_core::capability::{BinaryId, Inventory};
use vqa_core::estimate::{RunEstimate, estimate};
use vqa_core::media::{ColorRange, FrameSample, LumaExtremes, MediaInfo, Rational};
use vqa_core::metric::{Availability, MetricId, availability};
use vqa_core::preset::PRESETS;
use vqa_core::probe::MediaProbe;
use vqa_core::set::{ComparisonSet, FileId};

/// What the tool ticks when it opens, when the back ends allow it.
const DEFAULT_METRICS: [MetricId; 6] = [
    MetricId::PsnrY,
    MetricId::SsimAll,
    MetricId::XpsnrMin,
    MetricId::Vmaf,
    MetricId::Cambi,
    MetricId::Ssimulacra2,
];

/// What to measure, and how much of the file to measure.
///
/// This is the input of a plan. It holds no interface state, so a command line can build
/// the same value.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// The ticked metrics.
    pub metrics: BTreeSet<MetricId>,
    /// The preset name, until any checkbox changes.
    pub preset: Option<&'static str>,
    /// True for the whole file.
    pub whole_file: bool,
    /// The first frame of the range.
    pub first_frame: u64,
    /// The last frame of the range.
    pub last_frame: u64,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            metrics: BTreeSet::new(),
            preset: None,
            whole_file: true,
            first_frame: 0,
            last_frame: 0,
        }
    }
}

/// One comparison, with everything that the tool knows about this machine.
#[derive(Debug)]
pub struct Session {
    /// What the user set.
    pub settings: Settings,
    /// What the tool found on this machine.
    pub inventory: Inventory,
    /// The reference and the encodes.
    pub files: ComparisonSet,
    /// What to measure.
    pub selection: Selection,
    /// The files that arrived without media information, and why.
    pub probe_problems: Vec<(PathBuf, String)>,
    cache: CapabilityCache,
}

impl Session {
    /// Builds a session and reads the settings and the cache.
    pub fn load() -> Self {
        Self::with_settings(Settings::load(), CapabilityCache::load())
    }

    /// Builds a session from a known settings value.
    pub fn with_settings(settings: Settings, cache: CapabilityCache) -> Self {
        let mut session = Self {
            settings,
            inventory: Inventory::new(),
            files: ComparisonSet::new(),
            selection: Selection::default(),
            probe_problems: Vec::new(),
            cache,
        };
        session.refresh_inventory();
        session.tick_defaults();
        session
    }

    /// Looks for every binary again, and reads the capabilities of each one.
    ///
    /// A machine with no binaries is a normal first run. This reports no error.
    pub fn refresh_inventory(&mut self) {
        let mut discovery = discovery::Discovery::from_current_exe();
        for (id, path) in &self.settings.binary_paths {
            discovery.overrides.insert(*id, path.clone());
        }

        let mut inventory = Inventory::new();
        let mut cache_changed = false;

        for id in BinaryId::ALL {
            let Some(path) = discovery::find_path(id, &discovery) else {
                continue;
            };
            let Ok(sha256) = hash::sha256_file(&path) else {
                continue;
            };

            if let Some(found) = self.cache.recall(id, &path, &sha256) {
                inventory.insert(found);
                continue;
            }

            match discovery::probe_binary(id, &path) {
                Ok(found) => {
                    self.cache.remember(&found);
                    cache_changed = true;
                    inventory.insert(found);
                }
                Err(error) => {
                    tracing::warn!(binary = id.display_name(), %error, "the capability probe failed");
                }
            }
        }

        self.inventory = inventory;
        if cache_changed {
            if let Err(error) = self.cache.save() {
                tracing::warn!(%error, "cannot write the capability cache");
            }
        }
    }

    /// The probe, when this machine has `ffprobe`.
    fn probe(&self) -> Option<FfprobeProbe> {
        self.inventory
            .get(BinaryId::Ffprobe)
            .map(|found| FfprobeProbe::new(found.path.clone()))
    }

    /// The real luma minimum and maximum of a sample of frames, for note N1.
    ///
    /// Returns nothing with no `ffprobe`, or when the read fails. Nothing blocks on
    /// this, so a failed sample just means the note never fires for this file.
    pub fn luma_extremes(&self, path: &Path) -> Option<LumaExtremes> {
        self.probe()?
            .luma_extremes(path, FrameSample::default())
            .ok()
    }

    /// Adds one file to the comparison set.
    ///
    /// With no `ffprobe` the row still appears, with the fields that the file system
    /// gives. Nothing blocks.
    pub fn add_file(&mut self, path: &Path) -> FileId {
        let info = match self.probe() {
            Some(probe) => match probe.probe(path) {
                Ok(info) => info,
                Err(error) => {
                    self.probe_problems
                        .push((path.to_path_buf(), error.to_string()));
                    placeholder_info(path)
                }
            },
            None => {
                self.probe_problems.push((
                    path.to_path_buf(),
                    "no ffprobe. Set the path in Settings to read the media information."
                        .to_string(),
                ));
                placeholder_info(path)
            }
        };

        let id = self.files.add(info);
        self.reset_range();
        id
    }

    /// Removes one file.
    pub fn remove_file(&mut self, id: FileId) {
        self.files.remove(id);
        self.reset_range();
    }

    /// Makes one file the reference.
    pub fn promote_to_reference(&mut self, id: FileId) {
        self.files.promote_to_reference(id);
        self.reset_range();
    }

    /// The frame count of the reference, which is the length of the measurement.
    pub fn total_frames(&self) -> u64 {
        self.files
            .reference()
            .and_then(|file| file.info.frame_count())
            .unwrap_or(0)
    }

    /// Puts the frame range back to the whole file.
    fn reset_range(&mut self) {
        let total = self.total_frames();
        self.selection.first_frame = 0;
        self.selection.last_frame = total.saturating_sub(1);
    }

    /// Ticks the default metrics that this machine can run.
    fn tick_defaults(&mut self) {
        for id in DEFAULT_METRICS {
            if availability(id, &self.inventory).is_available() {
                self.selection.metrics.insert(id);
            }
        }
    }

    /// Whether one metric can run, and why not when it cannot.
    pub fn availability(&self, id: MetricId) -> Availability {
        availability(id, &self.inventory)
    }

    /// Ticks or unticks one metric. Any change clears the preset name.
    pub fn toggle_metric(&mut self, id: MetricId, ticked: bool) {
        if ticked {
            self.selection.metrics.insert(id);
        } else {
            self.selection.metrics.remove(&id);
        }
        self.selection.preset = None;
    }

    /// Applies one preset by name. A preset is a button, not a mode.
    pub fn apply_preset(&mut self, name: &str) {
        let Some(preset) = PRESETS.iter().find(|preset| preset.name == name) else {
            return;
        };
        self.selection.metrics.clear();
        for id in preset.metrics {
            self.selection.metrics.insert(*id);
        }
        self.selection.preset = Some(preset.name);
    }

    /// The metrics that will run: ticked, and with a back end.
    pub fn runnable_metrics(&self) -> Vec<MetricId> {
        self.selection
            .metrics
            .iter()
            .copied()
            .filter(|id| self.availability(*id).is_available())
            .collect()
    }

    /// How many frames the run covers.
    pub fn frames_in_range(&self) -> u64 {
        let total = self.total_frames();
        if self.selection.whole_file || total == 0 {
            return total;
        }
        self.selection
            .last_frame
            .saturating_sub(self.selection.first_frame)
            .saturating_add(1)
            .min(total)
    }

    /// The estimated run time, as a range.
    pub fn estimate(&self) -> Option<RunEstimate> {
        let reference = self.files.reference()?;
        estimate(
            &self.runnable_metrics(),
            &self.inventory,
            self.frames_in_range(),
            reference.info.width,
            reference.info.height,
            self.files.encode_count().max(1),
        )
    }

    /// Sets the path of one binary and looks again.
    pub fn set_binary_path(&mut self, id: BinaryId, path: Option<PathBuf>) {
        self.settings.set_binary_path(id, path);
        self.refresh_inventory();
        if let Err(error) = self.settings.save() {
            tracing::warn!(%error, "cannot write the settings file");
        }
    }

    /// Writes the settings.
    pub fn save_settings(&self) {
        if let Err(error) = self.settings.save() {
            tracing::warn!(%error, "cannot write the settings file");
        }
    }
}

/// What a file row shows when `ffprobe` could not read the file.
fn placeholder_info(path: &Path) -> MediaInfo {
    MediaInfo {
        path: path.to_path_buf(),
        bytes: std::fs::metadata(path).map(|data| data.len()).unwrap_or(0),
        codec: "unknown".to_string(),
        profile: None,
        width: 0,
        height: 0,
        pix_fmt: "unknown".to_string(),
        bit_depth: 8,
        color_range: ColorRange::Unknown,
        color_space: None,
        frame_rate: Rational::ZERO,
        nb_frames: None,
        duration_s: None,
        bit_rate: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet as Set;
    use vqa_core::capability::{BinaryCapabilities, FoundBinary};

    fn session_with(filters: &[&str]) -> Session {
        let mut session = Session {
            settings: Settings::default(),
            inventory: Inventory::new(),
            files: ComparisonSet::new(),
            selection: Selection::default(),
            probe_problems: Vec::new(),
            cache: CapabilityCache::new(),
        };
        if !filters.is_empty() {
            let names: Set<String> = filters.iter().map(|name| name.to_string()).collect();
            let libvmaf = vqa_backends::capabilities::libvmaf_features(&names);
            session.inventory.insert(FoundBinary {
                id: BinaryId::Ffmpeg,
                path: PathBuf::from("ffmpeg"),
                sha256: "0".into(),
                capabilities: BinaryCapabilities {
                    version: Some("7.1".into()),
                    version_parts: Some((7, 1)),
                    ffmpeg_filters: names,
                    libvmaf_features: libvmaf,
                    vship_metrics: Set::new(),
                },
            });
        }
        session
    }

    #[test]
    fn a_machine_with_no_binaries_ticks_nothing_and_reports_no_error() {
        let mut session = session_with(&[]);
        session.tick_defaults();
        assert!(session.selection.metrics.is_empty());
        assert!(session.runnable_metrics().is_empty());
        assert!(session.estimate().is_none());
    }

    #[test]
    fn the_default_ticks_follow_what_the_machine_can_run() {
        let mut session = session_with(&["psnr", "ssim", "libvmaf"]);
        session.tick_defaults();
        assert!(session.selection.metrics.contains(&MetricId::PsnrY));
        assert!(session.selection.metrics.contains(&MetricId::SsimAll));
        assert!(session.selection.metrics.contains(&MetricId::Cambi));
        assert!(!session.selection.metrics.contains(&MetricId::XpsnrMin));
        assert!(!session.selection.metrics.contains(&MetricId::Ssimulacra2));
    }

    #[test]
    fn a_preset_ticks_boxes_and_a_manual_change_clears_the_name() {
        let mut session = session_with(&["psnr", "ssim", "xpsnr", "libvmaf"]);
        session.apply_preset("Cel-shaded or anime");
        assert_eq!(session.selection.preset, Some("Cel-shaded or anime"));
        assert!(session.selection.metrics.contains(&MetricId::Cambi));

        session.toggle_metric(MetricId::PsnrY, true);
        assert_eq!(session.selection.preset, None);
        assert!(session.selection.metrics.contains(&MetricId::Cambi));
    }

    #[test]
    fn an_unknown_preset_name_changes_nothing() {
        let mut session = session_with(&["psnr"]);
        session.toggle_metric(MetricId::PsnrY, true);
        let before = session.selection.clone();
        session.apply_preset("no such preset");
        assert_eq!(session.selection, before);
    }

    #[test]
    fn a_ticked_metric_with_no_back_end_never_reaches_the_run() {
        let mut session = session_with(&["psnr"]);
        session.toggle_metric(MetricId::PsnrY, true);
        session.toggle_metric(MetricId::Ssimulacra2, true);
        assert_eq!(session.selection.metrics.len(), 2);
        assert_eq!(session.runnable_metrics(), vec![MetricId::PsnrY]);
    }
}
