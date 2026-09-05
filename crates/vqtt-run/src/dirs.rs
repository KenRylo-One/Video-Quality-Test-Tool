//! Where the tool keeps the settings and the capability cache.
//!
//! One place holds the folder rule of each operating system, which is what NFR-4 asks for.

use std::path::{Path, PathBuf};

/// The folder that this tool owns.
const FOLDER: &str = "vqtt";

/// Which folder the caller wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// What the user set. It is worth a backup.
    Config,
    /// What the tool can build again. Deleting it costs one rescan.
    Cache,
}

/// The environment values that decide the answer.
///
/// The resolver takes these instead of reading them, because a test that sets a variable is
/// unsafe in this edition and races every other test in the same process.
#[derive(Debug, Default, Clone)]
pub struct Environment {
    /// The value of `std::env::consts::OS`.
    pub os: &'static str,
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub xdg_config: Option<PathBuf>,
    pub xdg_cache: Option<PathBuf>,
}

impl Environment {
    /// Reads the real environment.
    pub fn read() -> Self {
        Self {
            os: std::env::consts::OS,
            appdata: std::env::var_os("APPDATA").map(PathBuf::from),
            local_appdata: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            home: std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from),
            xdg_config: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            xdg_cache: std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        }
    }
}

/// The folder of that kind on this machine.
pub fn folder(kind: Kind) -> Option<PathBuf> {
    resolve(kind, &Environment::read())
}

/// The folder of that kind for one environment.
pub fn resolve(kind: Kind, env: &Environment) -> Option<PathBuf> {
    match (env.os, kind) {
        ("windows", Kind::Config) => Some(env.appdata.as_ref()?.join(FOLDER)),
        ("windows", Kind::Cache) => Some(env.local_appdata.as_ref()?.join(FOLDER).join("cache")),
        ("macos", Kind::Config) => Some(
            env.home
                .as_ref()?
                .join("Library")
                .join("Application Support")
                .join(FOLDER),
        ),
        ("macos", Kind::Cache) => Some(
            env.home
                .as_ref()?
                .join("Library")
                .join("Caches")
                .join(FOLDER),
        ),
        (_, Kind::Config) => xdg(env.xdg_config.as_deref(), env.home.as_deref(), ".config"),
        (_, Kind::Cache) => xdg(env.xdg_cache.as_deref(), env.home.as_deref(), ".cache"),
    }
}

/// The XDG rule. A relative value is ignored, which the specification requires.
///
/// The test is a leading separator and not `is_absolute`, because `is_absolute` answers for
/// the machine that runs the code. A Windows build reads `/srv/settings` as relative, and
/// this function must give the same answer for a named system on any machine.
fn xdg(set: Option<&Path>, home: Option<&Path>, fallback: &str) -> Option<PathBuf> {
    match set {
        Some(path) if path.starts_with("/") => Some(path.join(FOLDER)),
        _ => Some(home?.join(fallback).join(FOLDER)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> Environment {
        Environment {
            os: "windows",
            appdata: Some(PathBuf::from(r"C:\Users\ada\AppData\Roaming")),
            local_appdata: Some(PathBuf::from(r"C:\Users\ada\AppData\Local")),
            ..Environment::default()
        }
    }

    fn unix(os: &'static str) -> Environment {
        Environment {
            os,
            home: Some(PathBuf::from("/home/ada")),
            ..Environment::default()
        }
    }

    #[test]
    fn windows_uses_the_roaming_folder_for_settings_and_the_local_one_for_the_cache() {
        let env = windows();
        assert_eq!(
            resolve(Kind::Config, &env).unwrap(),
            PathBuf::from(r"C:\Users\ada\AppData\Roaming").join("vqtt")
        );
        assert_eq!(
            resolve(Kind::Cache, &env).unwrap(),
            PathBuf::from(r"C:\Users\ada\AppData\Local")
                .join("vqtt")
                .join("cache")
        );
    }

    #[test]
    fn macos_uses_the_library_folders() {
        let env = unix("macos");
        assert!(
            resolve(Kind::Config, &env)
                .unwrap()
                .ends_with("Library/Application Support/vqtt")
        );
        assert!(
            resolve(Kind::Cache, &env)
                .unwrap()
                .ends_with("Library/Caches/vqtt")
        );
    }

    #[test]
    fn linux_falls_back_to_the_dot_folders_when_no_xdg_variable_is_set() {
        let env = unix("linux");
        assert!(
            resolve(Kind::Config, &env)
                .unwrap()
                .ends_with(".config/vqtt")
        );
        assert!(resolve(Kind::Cache, &env).unwrap().ends_with(".cache/vqtt"));
    }

    #[test]
    fn an_absolute_xdg_variable_wins() {
        let env = Environment {
            xdg_config: Some(PathBuf::from("/srv/settings")),
            ..unix("linux")
        };
        assert_eq!(
            resolve(Kind::Config, &env).unwrap(),
            PathBuf::from("/srv/settings/vqtt")
        );
    }

    #[test]
    fn a_relative_xdg_variable_is_ignored() {
        let env = Environment {
            xdg_config: Some(PathBuf::from("settings")),
            ..unix("linux")
        };
        assert!(
            resolve(Kind::Config, &env)
                .unwrap()
                .ends_with(".config/vqtt")
        );
    }

    #[test]
    fn an_environment_that_names_nothing_gives_nothing() {
        for os in ["windows", "macos", "linux"] {
            let env = Environment {
                os,
                ..Environment::default()
            };
            assert!(resolve(Kind::Config, &env).is_none());
            assert!(resolve(Kind::Cache, &env).is_none());
        }
    }
}
