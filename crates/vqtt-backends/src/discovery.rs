//! Finding the back-end binaries, and reading what they can do.
//!
//! The tool looks in three places, in this order: the path in the settings, a `bin` folder
//! beside the executable of the tool, and the PATH of the operating system.

use crate::capabilities::{
    libvmaf_features, parse_ffmpeg_filters, parse_version, parse_version_parts, parse_vship_metrics,
};
use crate::error::{BackendError, Result};
use crate::hash::sha256_file;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use vqtt_core::capability::{BinaryCapabilities, BinaryId, FoundBinary, Inventory};

/// The file name suffix of an executable on this operating system.
#[cfg(windows)]
const EXE_SUFFIX: &str = ".exe";
/// The file name suffix of an executable on this operating system.
#[cfg(not(windows))]
const EXE_SUFFIX: &str = "";

/// Where to look for the binaries.
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    /// A path that the user set in Settings, for each binary.
    pub overrides: BTreeMap<BinaryId, PathBuf>,
    /// The folder that holds the executable of the tool.
    pub tool_folder: Option<PathBuf>,
}

impl Discovery {
    /// Builds a discovery that reads the folder of the running executable.
    pub fn from_current_exe() -> Self {
        let tool_folder = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        Self {
            overrides: BTreeMap::new(),
            tool_folder,
        }
    }

    /// Sets the path of one binary.
    pub fn set_override(&mut self, id: BinaryId, path: Option<PathBuf>) {
        match path {
            Some(path) => {
                self.overrides.insert(id, path);
            }
            None => {
                self.overrides.remove(&id);
            }
        }
    }
}

/// Finds one binary, or reports that it is not there.
pub fn find_path(id: BinaryId, discovery: &Discovery) -> Option<PathBuf> {
    let file_name = format!("{}{}", id.command_name(), EXE_SUFFIX);

    if let Some(path) = discovery.overrides.get(&id) {
        if is_program(path) {
            return Some(path.clone());
        }
        // A downloaded back end arrives as a folder, and naming the folder is what a
        // reader does first. Look inside it for the program before giving up.
        let inside = path.join(&file_name);
        if is_program(&inside) {
            return Some(inside);
        }
        // A path that the user set and that is wrong must not fall through in silence.
        // The Settings panel reads this as "not found" and shows the path that failed.
        return None;
    }

    if let Some(folder) = &discovery.tool_folder {
        let beside = folder.join("bin").join(&file_name);
        if is_program(&beside) {
            return Some(beside);
        }
    }

    let path_variable = std::env::var_os("PATH")?;
    std::env::split_paths(&path_variable)
        .map(|folder| folder.join(&file_name))
        .find(|candidate| is_program(candidate))
}

/// True when the path names a file that the tool can run.
fn is_program(path: &Path) -> bool {
    path.is_file()
}

/// Reads the version and the capabilities of one binary.
pub fn probe_binary(id: BinaryId, path: &Path) -> Result<FoundBinary> {
    let sha256 = sha256_file(path)?;
    let capabilities = read_capabilities(id, path)?;
    Ok(FoundBinary {
        id,
        path: path.to_path_buf(),
        sha256,
        capabilities,
    })
}

/// Runs the version command and the capability command of one binary.
fn read_capabilities(id: BinaryId, path: &Path) -> Result<BinaryCapabilities> {
    let mut capabilities = BinaryCapabilities::default();

    let version_args: &[&str] = match id {
        BinaryId::Ffmpeg | BinaryId::Ffprobe => &["-version"],
        BinaryId::Vmaf | BinaryId::Ffvship | BinaryId::Ssimulacra2Rs => &["--version"],
    };
    if let Some(text) = capture(path, version_args) {
        capabilities.version = parse_version(&text);
        capabilities.version_parts = capabilities
            .version
            .as_deref()
            .and_then(parse_version_parts);
    }

    match id {
        BinaryId::Ffmpeg => {
            let filters = capture(path, &["-hide_banner", "-filters"]).unwrap_or_default();
            capabilities.ffmpeg_filters = parse_ffmpeg_filters(&filters);
            capabilities.libvmaf_features = libvmaf_features(&capabilities.ffmpeg_filters);
        }
        BinaryId::Ffvship => {
            let help = capture(path, &["--help"]).unwrap_or_default();
            capabilities.vship_metrics = parse_vship_metrics(&help);
        }
        BinaryId::Ffprobe | BinaryId::Vmaf | BinaryId::Ssimulacra2Rs => {}
    }

    Ok(capabilities)
}

/// Runs a program and joins its standard output and standard error.
///
/// A capability command can report a failure and still print what the tool needs. The
/// `vmaf` binary is one example, because it has no `--help` option. The exit code is
/// therefore not read here.
fn capture(path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(path).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Some(text)
}

/// Finds and probes every binary.
///
/// A binary that is missing is left out of the inventory. Nothing here reports an error,
/// because a machine with no binaries is a normal first run.
pub fn discover(discovery: &Discovery) -> Inventory {
    let mut inventory = Inventory::new();
    for id in BinaryId::ALL {
        let Some(path) = find_path(id, discovery) else {
            continue;
        };
        match probe_binary(id, &path) {
            Ok(found) => inventory.insert(found),
            Err(error) => {
                tracing::warn!(binary = id.display_name(), %error, "the binary did not answer the capability probe");
            }
        }
    }
    inventory
}

/// The reason string for a binary that the tool did not find.
pub fn not_found_reason(id: BinaryId) -> String {
    format!("not found. Get it from {}.", id.source())
}

/// Turns a spawn failure into a message that names the program.
pub fn spawn_error(program: &Path, source: std::io::Error) -> BackendError {
    BackendError::Spawn {
        program: program.display().to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_that_points_at_nothing_reports_not_found() {
        let mut discovery = Discovery::default();
        discovery.set_override(BinaryId::Ffmpeg, Some(PathBuf::from("/no/such/ffmpeg")));
        assert_eq!(find_path(BinaryId::Ffmpeg, &discovery), None);
    }

    #[test]
    fn an_override_that_names_the_folder_finds_the_program_inside_it() {
        let folder = std::env::temp_dir().join("vqtt-discovery-folder-test");
        std::fs::create_dir_all(&folder).unwrap();
        let program = folder.join(format!("{}{EXE_SUFFIX}", BinaryId::Ffvship.command_name()));
        std::fs::write(&program, b"not a real program").unwrap();

        let mut discovery = Discovery::default();
        discovery.set_override(BinaryId::Ffvship, Some(folder.clone()));
        assert_eq!(find_path(BinaryId::Ffvship, &discovery), Some(program));

        // A folder that holds no such program is still not found.
        discovery.set_override(BinaryId::Vmaf, Some(folder));
        assert_eq!(find_path(BinaryId::Vmaf, &discovery), None);
    }

    #[test]
    fn an_override_that_names_the_program_itself_still_wins() {
        let folder = std::env::temp_dir().join("vqtt-discovery-file-test");
        std::fs::create_dir_all(&folder).unwrap();
        let program = folder.join(format!("{}{EXE_SUFFIX}", BinaryId::Ffvship.command_name()));
        std::fs::write(&program, b"not a real program").unwrap();

        let mut discovery = Discovery::default();
        discovery.set_override(BinaryId::Ffvship, Some(program.clone()));
        assert_eq!(find_path(BinaryId::Ffvship, &discovery), Some(program));
    }

    #[test]
    fn the_not_found_reason_names_where_to_get_it() {
        let reason = not_found_reason(BinaryId::Ffvship);
        assert!(reason.contains("codeberg.org"), "{reason}");
    }

    #[test]
    fn discovery_of_an_empty_machine_reports_no_error() {
        let discovery = Discovery {
            overrides: BTreeMap::new(),
            tool_folder: Some(PathBuf::from("/no/such/folder")),
        };
        // The PATH of the test machine can hold ffmpeg. The rule under test is that
        // discovery returns an inventory and never an error.
        let inventory = discover(&discovery);
        for found in inventory.iter() {
            assert!(!found.sha256.is_empty());
            assert!(found.path.is_file());
        }
    }
}
