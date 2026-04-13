//! `grip.lock` lock file types and I/O.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use crate::error::GripError;

/// The top-level `grip.lock` document — a list of installed binary and library records.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct LockFile {
    #[serde(default, rename = "binary")]
    pub entries: Vec<LockEntry>,
    #[serde(default, rename = "library")]
    pub library_entries: Vec<LockEntry>,
}

/// A single record in the lock file describing an installed binary.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LockEntry {
    /// Logical name of the binary (matches the key in `grip.toml`).
    pub name: String,
    /// Installed version string.
    pub version: String,
    /// Adapter that installed the binary (e.g. `"github"`, `"apt"`).
    pub source: String,
    /// Download URL, if applicable.
    pub url: Option<String>,
    /// SHA-256 hex digest of the installed binary, if computed.
    pub sha256: Option<String>,
    /// UTC timestamp of when this entry was last written.
    pub installed_at: DateTime<Utc>,
}

impl LockFile {
    /// Load `grip.lock` from `path`. Returns an empty lock file if the path does not exist.
    pub fn load(path: &Path) -> Result<Self, GripError> {
        if !path.exists() {
            return Ok(LockFile::default());
        }
        let content = std::fs::read_to_string(path)?;
        let lf: LockFile = toml::from_str(&content)?;
        Ok(lf)
    }

    /// Atomically write the lock file to `path` via a temporary file + rename.
    pub fn save(&self, path: &Path) -> Result<(), GripError> {
        let content = toml::to_string_pretty(self)?;
        // Write to .tmp then rename for atomicity
        let tmp = path.with_extension("lock.tmp");
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Look up an entry by binary name.
    pub fn get(&self, name: &str) -> Option<&LockEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Insert or replace the entry with the same name.
    pub fn upsert(&mut self, entry: LockEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.name == entry.name) {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// Remove a binary lock entry by name. No-op if not present.
    pub fn remove(&mut self, name: &str) {
        self.entries.retain(|e| e.name != name);
    }

    /// Remove a library lock entry by name. No-op if not present.
    pub fn remove_library(&mut self, name: &str) {
        self.library_entries.retain(|e| e.name != name);
    }

    /// Look up a library entry by name.
    pub fn get_library(&self, name: &str) -> Option<&LockEntry> {
        self.library_entries.iter().find(|e| e.name == name)
    }

    /// Insert or replace a library lock entry with the same name.
    pub fn upsert_library(&mut self, entry: LockEntry) {
        if let Some(existing) = self.library_entries.iter_mut().find(|e| e.name == entry.name) {
            *existing = entry;
        } else {
            self.library_entries.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_entry(name: &str, version: &str, source: &str) -> LockEntry {
        LockEntry {
            name: name.to_string(),
            version: version.to_string(),
            source: source.to_string(),
            url: None,
            sha256: None,
            installed_at: Utc::now(),
        }
    }

    // ── get / upsert / remove ─────────────────────────────────────────────────

    #[test]
    fn get_returns_none_for_empty_lockfile() {
        let lf = LockFile::default();
        assert!(lf.get("rg").is_none());
    }

    #[test]
    fn upsert_inserts_new_entry() {
        let mut lf = LockFile::default();
        lf.upsert(make_entry("rg", "14.0.0", "github"));
        assert_eq!(lf.get("rg").unwrap().version, "14.0.0");
    }

    #[test]
    fn upsert_replaces_existing_entry() {
        let mut lf = LockFile::default();
        lf.upsert(make_entry("rg", "13.0.0", "github"));
        lf.upsert(make_entry("rg", "14.0.0", "github"));
        assert_eq!(lf.entries.len(), 1);
        assert_eq!(lf.get("rg").unwrap().version, "14.0.0");
    }

    #[test]
    fn remove_deletes_entry() {
        let mut lf = LockFile::default();
        lf.upsert(make_entry("rg", "14.0.0", "github"));
        lf.remove("rg");
        assert!(lf.get("rg").is_none());
        assert!(lf.entries.is_empty());
    }

    #[test]
    fn remove_is_noop_when_absent() {
        let mut lf = LockFile::default();
        lf.remove("nonexistent"); // must not panic
        assert!(lf.entries.is_empty());
    }

    // ── library variants ──────────────────────────────────────────────────────

    #[test]
    fn upsert_library_inserts_and_replaces() {
        let mut lf = LockFile::default();
        lf.upsert_library(make_entry("libssl-dev", "3.0.0", "apt"));
        assert_eq!(lf.get_library("libssl-dev").unwrap().version, "3.0.0");

        lf.upsert_library(make_entry("libssl-dev", "3.1.0", "apt"));
        assert_eq!(lf.library_entries.len(), 1);
        assert_eq!(lf.get_library("libssl-dev").unwrap().version, "3.1.0");
    }

    #[test]
    fn remove_library_deletes_entry() {
        let mut lf = LockFile::default();
        lf.upsert_library(make_entry("libssl-dev", "3.0.0", "apt"));
        lf.remove_library("libssl-dev");
        assert!(lf.get_library("libssl-dev").is_none());
    }

    // ── load / save round-trip ────────────────────────────────────────────────

    #[test]
    fn load_returns_default_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("grip.lock");
        let lf = LockFile::load(&path).unwrap();
        assert!(lf.entries.is_empty());
        assert!(lf.library_entries.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("grip.lock");

        let mut lf = LockFile::default();
        lf.upsert(LockEntry {
            name: "jq".to_string(),
            version: "1.7.1".to_string(),
            source: "github".to_string(),
            url: Some("https://example.com/jq".to_string()),
            sha256: Some("abc123".to_string()),
            installed_at: Utc::now(),
        });
        lf.save(&path).unwrap();

        let loaded = LockFile::load(&path).unwrap();
        let entry = loaded.get("jq").unwrap();
        assert_eq!(entry.version, "1.7.1");
        assert_eq!(entry.source, "github");
        assert_eq!(entry.sha256.as_deref(), Some("abc123"));
    }

    #[test]
    fn save_is_atomic_via_rename() {
        // save() writes .lock.tmp then renames; the final file must exist and be valid TOML
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("grip.lock");
        let lf = LockFile::default();
        lf.save(&path).unwrap();
        assert!(path.exists());
        assert!(!tmp.path().join("grip.lock.tmp").exists());
    }
}
