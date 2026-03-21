//! `grip.toml` manifest types and loading/saving logic.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use crate::error::GripError;

/// The top-level `grip.toml` document.
#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    /// Map of logical binary name → installation entry.
    #[serde(default)]
    pub binaries: IndexMap<String, BinaryEntry>,
    /// Map of logical library name → library installation entry (no executable required).
    #[serde(default)]
    pub libraries: IndexMap<String, LibraryEntry>,
}

/// Metadata shared across all entry types, flattened into the TOML table.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CommonMeta {
    /// Only install this entry on the listed platforms (e.g. ["linux", "darwin"]).
    /// Omit to install on all platforms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    /// Shell command to run after a successful install.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_install: Option<String>,
    /// If false, a failure is a warning rather than a hard error. Defaults to true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Arbitrary tags for selective installs (`grip install --tag <tag>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl CommonMeta {
    /// Returns `true` if the entry is required (default when `required` is unset).
    pub fn is_required(&self) -> bool {
        self.required.unwrap_or(true)
    }

    /// Returns `true` if this entry should be installed on the given OS string.
    /// When no `platforms` filter is set, the entry matches every platform.
    pub fn matches_platform(&self, os: &str) -> bool {
        match &self.platforms {
            None => true,
            Some(list) => list.iter().any(|p| p == os),
        }
    }

    /// Returns `true` if the entry carries the given tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.as_deref().unwrap_or(&[]).iter().any(|t| t == tag)
    }
}

/// A single binary dependency, discriminated by the `source` field in TOML.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum BinaryEntry {
    Apt(AptEntry),
    Dnf(DnfEntry),
    Github(GithubEntry),
    Url(UrlEntry),
    Shell(ShellEntry),
}

/// Entry installed via `dnf` (Fedora / RHEL).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DnfEntry {
    pub package: String,
    /// On-PATH command name when it differs from the manifest key (e.g. package `ripgrep` → `rg`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    pub version: Option<String>,
    #[serde(flatten)]
    pub meta: CommonMeta,
}

/// Entry installed via `apt-get` (Debian / Ubuntu).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AptEntry {
    pub package: String,
    /// On-PATH command name when it differs from the manifest key (e.g. `fd-find` → `fd`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    pub version: Option<String>,
    #[serde(flatten)]
    pub meta: CommonMeta,
}

/// Entry downloaded from a GitHub release.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubEntry {
    /// `owner/repo` slug, e.g. `"jqlang/jq"`.
    pub repo: String,
    /// Pinned version (without `v` prefix). `None` resolves to the latest release.
    pub version: Option<String>,
    /// Glob pattern to match the release asset filename. Falls back to platform heuristics.
    pub asset_pattern: Option<String>,
    /// Name of the binary inside the archive when it differs from the entry name.
    pub binary: Option<String>,
    #[serde(flatten)]
    pub meta: CommonMeta,
}

/// Entry downloaded from an arbitrary URL.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UrlEntry {
    pub url: String,
    /// Name of the binary inside the archive when it differs from the entry name.
    pub binary: Option<String>,
    /// Expected SHA-256 hex digest for download verification.
    pub sha256: Option<String>,
    #[serde(flatten)]
    pub meta: CommonMeta,
}

/// Entry installed by running an arbitrary shell command.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ShellEntry {
    /// Shell command passed to `sh -c`. The `GRIP_BIN_DIR` environment variable is set to the
    /// project's `.bin/` directory so the script can place the binary there.
    pub install_cmd: String,
    pub version: Option<String>,
    #[serde(flatten)]
    pub meta: CommonMeta,
}

impl BinaryEntry {
    /// Return a clone with the version field pinned to `version`.
    /// For system adapters (apt/dnf) this sets the package version field.
    /// For github/url entries it sets the version/sha256 fields so the adapter
    /// fetches exactly that release.
    pub fn pin_version(&self, version: &str) -> Self {
        match self {
            BinaryEntry::Apt(a) => BinaryEntry::Apt(AptEntry {
                version: Some(version.to_string()),
                ..a.clone()
            }),
            BinaryEntry::Dnf(d) => BinaryEntry::Dnf(DnfEntry {
                version: Some(version.to_string()),
                ..d.clone()
            }),
            BinaryEntry::Github(g) => BinaryEntry::Github(GithubEntry {
                version: Some(version.to_string()),
                ..g.clone()
            }),
            BinaryEntry::Url(u) => BinaryEntry::Url(u.clone()),
            BinaryEntry::Shell(s) => BinaryEntry::Shell(ShellEntry {
                version: Some(version.to_string()),
                ..s.clone()
            }),
        }
    }

    /// Return a reference to the [`CommonMeta`] carried by any entry variant.
    pub fn meta(&self) -> &CommonMeta {
        match self {
            BinaryEntry::Apt(e) => &e.meta,
            BinaryEntry::Dnf(e) => &e.meta,
            BinaryEntry::Github(e) => &e.meta,
            BinaryEntry::Url(e) => &e.meta,
            BinaryEntry::Shell(e) => &e.meta,
        }
    }
}

/// A library dependency (no executable), discriminated by the `source` field in TOML.
/// Only system package manager sources are supported since other sources produce executables.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum LibraryEntry {
    Apt(LibAptEntry),
    Dnf(LibDnfEntry),
}

/// Library entry installed via `apt-get` (Debian / Ubuntu).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LibAptEntry {
    pub package: String,
    pub version: Option<String>,
    #[serde(flatten)]
    pub meta: CommonMeta,
}

/// Library entry installed via `dnf` (Fedora / RHEL).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LibDnfEntry {
    pub package: String,
    pub version: Option<String>,
    #[serde(flatten)]
    pub meta: CommonMeta,
}

impl LibraryEntry {
    pub fn meta(&self) -> &CommonMeta {
        match self {
            LibraryEntry::Apt(e) => &e.meta,
            LibraryEntry::Dnf(e) => &e.meta,
        }
    }
}

impl Manifest {
    /// Load and parse a `grip.toml` from `path`.
    pub fn load(path: &Path) -> Result<Self, GripError> {
        let content = std::fs::read_to_string(path)?;
        let manifest: Manifest = toml::from_str(&content)?;
        Ok(manifest)
    }

    /// Serialize and write the manifest to `path`.
    pub fn save(&self, path: &Path) -> Result<(), GripError> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Create an empty manifest with no entries.
    pub fn empty() -> Self {
        Manifest {
            binaries: IndexMap::new(),
            libraries: IndexMap::new(),
        }
    }
}

/// Walk up from `start` to find the directory containing `grip.toml`.
pub fn find_manifest_dir(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("grip.toml").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}
