//! `grip.lock` lock file types and I/O.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use crate::error::GripError;

/// The top-level `grip.lock` document — a list of installed binary records.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct LockFile {
    #[serde(default, rename = "binary")]
    pub entries: Vec<LockEntry>,
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
}
