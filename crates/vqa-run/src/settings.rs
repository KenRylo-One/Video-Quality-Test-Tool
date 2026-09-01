//! What the user set, and where it is stored.
//!
//! Every setting in the first group changes a number, so every one of them also goes in
//! the run record.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vqa_core::capability::BinaryId;

/// The theme of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// The default.
    #[default]
    Dark,
    /// Fully supported.
    Light,
    /// Reads the setting of the operating system. This arrives after version 1.0.
    System,
}

/// The viewing distances that a VMAF v1 model offers, in picture heights.
pub const VIEWING_DISTANCES: [f32; 3] = [1.5, 3.0, 5.0];

/// Common intensity targets for Butteraugli, in nits.
///
/// 203 nits is the reference white of BT.2408.
pub const BUTTERAUGLI_PRESET_NITS: [u32; 5] = [80, 100, 203, 1000, 4000];

/// The version of this file format.
const SCHEMA: u32 = 1;

/// Everything that the user set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The version of this file. A reader that does not know it stops.
    #[serde(default = "default_schema")]
    pub schema: u32,

    /// The theme.
    #[serde(default)]
    pub theme: ThemeChoice,

    /// A path for each binary, when the user set one.
    #[serde(default)]
    pub binary_paths: BTreeMap<BinaryId, PathBuf>,

    /// The VMAF viewing distance, in picture heights. This feeds correction C5.
    #[serde(default = "default_viewing_distance")]
    pub vmaf_viewing_distance: f32,

    /// The Butteraugli intensity target, in nits.
    #[serde(default = "default_intensity")]
    pub butteraugli_intensity_nits: u32,

    /// How many measurements can run at once on the processor.
    #[serde(default = "default_cpu_permits")]
    pub cpu_lane_permits: u32,

    /// How many measurements can run at once on one graphics card.
    #[serde(default = "default_gpu_permits")]
    pub gpu_lane_permits: u32,

    /// Whether the FFmpeg metrics share one decode pass.
    #[serde(default = "default_true")]
    pub fused_passes: bool,

    /// Where FFVship writes a scaled intermediate file.
    #[serde(default)]
    pub temp_folder: Option<PathBuf>,

    /// Where CSV, JSON and PNG go.
    #[serde(default)]
    pub export_folder: Option<PathBuf>,
}

fn default_schema() -> u32 {
    SCHEMA
}
fn default_viewing_distance() -> f32 {
    3.0
}
fn default_intensity() -> u32 {
    203
}
fn default_true() -> bool {
    true
}

/// Half of the logical cores. Two full FFmpeg passes fight each other.
fn default_cpu_permits() -> u32 {
    let cores = std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(2);
    (cores / 2).max(1)
}

/// One job for each device. A second job on one device does not go faster.
fn default_gpu_permits() -> u32 {
    1
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            theme: ThemeChoice::default(),
            binary_paths: BTreeMap::new(),
            vmaf_viewing_distance: default_viewing_distance(),
            butteraugli_intensity_nits: default_intensity(),
            cpu_lane_permits: default_cpu_permits(),
            gpu_lane_permits: default_gpu_permits(),
            fused_passes: true,
            temp_folder: None,
            export_folder: None,
        }
    }
}

impl Settings {
    /// The file that holds the settings.
    pub fn file_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "vqa")
            .map(|dirs| dirs.config_dir().join("settings.toml"))
    }

    /// Reads the settings.
    ///
    /// A missing file, an unreadable file and a file from a newer version all give the
    /// defaults. The tool never stops at the door.
    pub fn load() -> Self {
        let Some(path) = Self::file_path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    /// Reads the settings from one file.
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match toml::from_str::<Settings>(&text) {
            Ok(settings) if settings.schema <= SCHEMA => settings,
            Ok(settings) => {
                tracing::warn!(
                    schema = settings.schema,
                    "the settings file is newer than this tool. Using the defaults."
                );
                Self::default()
            }
            Err(error) => {
                tracing::warn!(%error, "cannot read the settings file. Using the defaults.");
                Self::default()
            }
        }
    }

    /// Writes the settings.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::file_path() else {
            return Ok(());
        };
        self.save_to(&path)
    }

    /// Writes the settings to one file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(path, text)
    }

    /// The path that the user set for one binary.
    pub fn binary_path(&self, id: BinaryId) -> Option<&PathBuf> {
        self.binary_paths.get(&id)
    }

    /// Sets or clears the path of one binary.
    pub fn set_binary_path(&mut self, id: BinaryId, path: Option<PathBuf>) {
        match path {
            Some(path) if !path.as_os_str().is_empty() => {
                self.binary_paths.insert(id, path);
            }
            _ => {
                self.binary_paths.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_follow_the_design() {
        let settings = Settings::default();
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.vmaf_viewing_distance, 3.0);
        assert_eq!(settings.butteraugli_intensity_nits, 203);
        assert_eq!(settings.gpu_lane_permits, 1);
        assert!(settings.cpu_lane_permits >= 1);
        assert!(settings.fused_passes);
    }

    #[test]
    fn settings_read_back_from_a_file() {
        let dir = std::env::temp_dir().join("vqa-settings-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.toml");

        let mut settings = Settings {
            theme: ThemeChoice::Light,
            ..Default::default()
        };
        settings.set_binary_path(BinaryId::Ffvship, Some(PathBuf::from("/opt/FFVship")));
        settings.save_to(&path).unwrap();

        let read = Settings::load_from(&path);
        assert_eq!(read, settings);
        assert_eq!(
            read.binary_path(BinaryId::Ffvship).unwrap().to_str(),
            Some("/opt/FFVship")
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_broken_file_gives_the_defaults_and_never_stops_the_tool() {
        let dir = std::env::temp_dir().join("vqa-settings-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.toml");
        std::fs::write(&path, "this is not toml = = =").unwrap();

        assert_eq!(Settings::load_from(&path), Settings::default());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        assert_eq!(
            Settings::load_from(Path::new("/no/such/settings.toml")),
            Settings::default()
        );
    }

    #[test]
    fn an_empty_path_clears_the_override() {
        let mut settings = Settings::default();
        settings.set_binary_path(BinaryId::Ffmpeg, Some(PathBuf::from("/bin/ffmpeg")));
        assert!(settings.binary_path(BinaryId::Ffmpeg).is_some());
        settings.set_binary_path(BinaryId::Ffmpeg, Some(PathBuf::new()));
        assert!(settings.binary_path(BinaryId::Ffmpeg).is_none());
    }
}
