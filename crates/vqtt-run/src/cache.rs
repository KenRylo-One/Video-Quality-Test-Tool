//! The capability cache.
//!
//! A capability probe starts several processes. The answer is cached against the SHA-256
//! hash of the binary, so it runs once for each build of each back end. A new hash forces
//! a new probe.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vqtt_core::capability::{BinaryCapabilities, BinaryId, FoundBinary};

/// The version of this file format.
const SCHEMA: u32 = 1;

/// What the tool remembers about each binary that it probed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityCache {
    /// The version of this file.
    #[serde(default)]
    schema: u32,
    /// The capabilities, keyed by the SHA-256 hash of the binary.
    #[serde(default)]
    entries: BTreeMap<String, BinaryCapabilities>,
}

impl CapabilityCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self {
            schema: SCHEMA,
            entries: BTreeMap::new(),
        }
    }

    /// The file that holds the cache.
    pub fn file_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "vqtt")
            .map(|dirs| dirs.cache_dir().join("backends.json"))
    }

    /// Reads the cache. A missing or unreadable file gives an empty cache.
    pub fn load() -> Self {
        let Some(path) = Self::file_path() else {
            return Self::new();
        };
        Self::load_from(&path)
    }

    /// Reads the cache from one file.
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::new();
        };
        match serde_json::from_str::<CapabilityCache>(&text) {
            Ok(cache) if cache.schema == SCHEMA => cache,
            _ => Self::new(),
        }
    }

    /// Writes the cache.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::file_path() else {
            return Ok(());
        };
        self.save_to(&path)
    }

    /// Writes the cache to one file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(path, text)
    }

    /// Reads one entry.
    pub fn get(&self, sha256: &str) -> Option<&BinaryCapabilities> {
        self.entries.get(sha256)
    }

    /// Records one entry.
    pub fn insert(&mut self, sha256: impl Into<String>, capabilities: BinaryCapabilities) {
        self.entries.insert(sha256.into(), capabilities);
    }

    /// Records the answer for one binary that the tool probed.
    pub fn remember(&mut self, found: &FoundBinary) {
        self.insert(found.sha256.clone(), found.capabilities.clone());
    }

    /// Builds a found binary out of a cached answer, when there is one.
    pub fn recall(&self, id: BinaryId, path: &Path, sha256: &str) -> Option<FoundBinary> {
        self.get(sha256).map(|capabilities| FoundBinary {
            id,
            path: path.to_path_buf(),
            sha256: sha256.to_string(),
            capabilities: capabilities.clone(),
        })
    }

    /// How many answers the cache holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the cache holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_hash_misses_the_cache() {
        let mut cache = CapabilityCache::new();
        cache.insert(
            "aaaa",
            BinaryCapabilities {
                version: Some("7.1".into()),
                ..Default::default()
            },
        );
        assert!(cache.get("aaaa").is_some());
        assert!(cache.get("bbbb").is_none());
    }

    #[test]
    fn the_cache_reads_back_from_a_file() {
        let dir = std::env::temp_dir().join("vqtt-cache-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("backends.json");

        let mut cache = CapabilityCache::new();
        cache.insert(
            "aaaa",
            BinaryCapabilities {
                version: Some("7.1".into()),
                ..Default::default()
            },
        );
        cache.save_to(&path).unwrap();

        let read = CapabilityCache::load_from(&path);
        assert_eq!(read.len(), 1);
        assert_eq!(read.get("aaaa").unwrap().version.as_deref(), Some("7.1"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_broken_cache_file_gives_an_empty_cache() {
        let dir = std::env::temp_dir().join("vqtt-cache-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(CapabilityCache::load_from(&path).is_empty());
        std::fs::remove_file(&path).ok();
    }
}
