//! `grip.toml` manifest types and loading/saving logic.

use crate::error::GripError;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Additional binaries installed by the same package (e.g. `ffprobe`, `ffplay` for `ffmpeg`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_binaries: Option<Vec<String>>,
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
    /// Additional binaries installed by the same package (e.g. `ffprobe`, `ffplay` for `ffmpeg`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_binaries: Option<Vec<String>>,
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
    /// Additional binary names to extract from the same archive (e.g. `ffprobe`, `ffplay`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_binaries: Option<Vec<String>>,
    /// GPG key fingerprint (or long key ID) used to verify the release signature.
    /// When set, grip downloads the detached signature asset and verifies it against
    /// this fingerprint using the system `gpg` binary. Hard error on mismatch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpg_fingerprint: Option<String>,
    /// Glob pattern to locate the detached signature asset in the release
    /// (e.g. `"*.sig"`, `"checksums.txt.asc"`).
    /// When omitted, grip auto-detects using common patterns (`<asset>.sig`, `<asset>.asc`, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig_asset_pattern: Option<String>,
    /// Glob pattern to locate a signed checksums file in the release
    /// (e.g. `"*SHA256SUMS*"`, `"checksums.txt"`). When set, activates signed-checksums
    /// verification: grip verifies the checksums file's GPG signature, then validates the
    /// downloaded asset against the hash inside it. Takes precedence over direct binary
    /// signature verification (`sig_asset_pattern`). Requires `gpg_fingerprint`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksums_asset_pattern: Option<String>,
    #[serde(flatten)]
    pub meta: CommonMeta,
}

/// Entry downloaded from an arbitrary URL.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UrlEntry {
    pub url: String,
    /// Name of the binary inside the archive when it differs from the entry name.
    pub binary: Option<String>,
    /// Additional binary names to extract from the same archive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_binaries: Option<Vec<String>>,
    /// Expected SHA-256 hex digest for download verification.
    pub sha256: Option<String>,
    /// GPG key fingerprint (or long key ID) used to verify the downloaded file.
    /// Requires `sig_url` to also be set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpg_fingerprint: Option<String>,
    /// URL of the detached GPG signature file (e.g. `https://example.com/tool.tar.gz.sig`).
    /// Used for direct binary signature verification (Mode 1). Required when `gpg_fingerprint`
    /// is set without `signed_checksums_url`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig_url: Option<String>,
    /// URL of a signed checksums file (e.g. `https://example.com/SHA256SUMS`). When set,
    /// activates signed-checksums verification (Mode 2): grip verifies the checksums file's GPG
    /// signature, then validates the downloaded binary against the hash inside it. Requires
    /// `gpg_fingerprint` and `checksums_sig_url`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_checksums_url: Option<String>,
    /// URL of the GPG signature for the checksums file
    /// (e.g. `https://example.com/SHA256SUMS.sig`). Required when `signed_checksums_url` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksums_sig_url: Option<String>,
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
    /// Must be explicitly set to `true` before grip will execute the shell command.
    /// Protects against arbitrary code execution if `grip.toml` is compromised (e.g. via a
    /// malicious PR). Omitting this field or setting it to `false` blocks execution and
    /// surfaces a clear error pointing to this field.
    #[serde(default)]
    pub allow_shell: bool,
    /// Additional binary names placed in `.bin/` by the install script.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_binaries: Option<Vec<String>>,
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

    /// Return the declared extra binary names, or an empty slice when none are set.
    pub fn extra_binaries(&self) -> &[String] {
        match self {
            BinaryEntry::Apt(e) => e.extra_binaries.as_deref().unwrap_or(&[]),
            BinaryEntry::Dnf(e) => e.extra_binaries.as_deref().unwrap_or(&[]),
            BinaryEntry::Github(e) => e.extra_binaries.as_deref().unwrap_or(&[]),
            BinaryEntry::Url(e) => e.extra_binaries.as_deref().unwrap_or(&[]),
            BinaryEntry::Shell(e) => e.extra_binaries.as_deref().unwrap_or(&[]),
        }
    }

    /// Returns `true` if this entry has an explicit version pin.
    ///
    /// A pinned entry always installs the same artifact on every `grip sync`.
    /// An unpinned entry silently upgrades to whatever is current — a supply-chain
    /// risk if the upstream release is ever compromised.
    ///
    /// `Url` entries are always considered pinned: the URL itself identifies a
    /// specific artifact. All other sources require an explicit `version` field.
    pub fn is_version_pinned(&self) -> bool {
        match self {
            BinaryEntry::Apt(a) => a.version.is_some(),
            BinaryEntry::Dnf(d) => d.version.is_some(),
            BinaryEntry::Github(g) => g.version.is_some(),
            BinaryEntry::Url(_) => true,
            BinaryEntry::Shell(s) => s.version.is_some(),
        }
    }

    /// Human-readable source label used in diagnostics.
    pub fn source_label(&self) -> &'static str {
        match self {
            BinaryEntry::Apt(_) => "apt",
            BinaryEntry::Dnf(_) => "dnf",
            BinaryEntry::Github(_) => "github",
            BinaryEntry::Url(_) => "url",
            BinaryEntry::Shell(_) => "shell",
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_meta(
        required: Option<bool>,
        platforms: Option<Vec<&str>>,
        tags: Option<Vec<&str>>,
    ) -> CommonMeta {
        CommonMeta {
            required,
            platforms: platforms.map(|v| v.into_iter().map(String::from).collect()),
            tags: tags.map(|v| v.into_iter().map(String::from).collect()),
            post_install: None,
        }
    }

    // ── CommonMeta ────────────────────────────────────────────────────────────

    #[test]
    fn is_required_defaults_to_true() {
        assert!(make_meta(None, None, None).is_required());
    }

    #[test]
    fn is_required_explicit_true() {
        assert!(make_meta(Some(true), None, None).is_required());
    }

    #[test]
    fn is_required_explicit_false() {
        assert!(!make_meta(Some(false), None, None).is_required());
    }

    #[test]
    fn matches_platform_no_filter() {
        let meta = make_meta(None, None, None);
        assert!(meta.matches_platform("linux"));
        assert!(meta.matches_platform("darwin"));
    }

    #[test]
    fn matches_platform_with_filter_hit() {
        let meta = make_meta(None, Some(vec!["linux", "darwin"]), None);
        assert!(meta.matches_platform("linux"));
        assert!(meta.matches_platform("darwin"));
    }

    #[test]
    fn matches_platform_with_filter_miss() {
        let meta = make_meta(None, Some(vec!["linux"]), None);
        assert!(!meta.matches_platform("darwin"));
        assert!(!meta.matches_platform("windows"));
    }

    #[test]
    fn has_tag_no_tags() {
        assert!(!make_meta(None, None, None).has_tag("ci"));
    }

    #[test]
    fn has_tag_hit() {
        let meta = make_meta(None, None, Some(vec!["ci", "dev"]));
        assert!(meta.has_tag("ci"));
        assert!(meta.has_tag("dev"));
    }

    #[test]
    fn has_tag_miss() {
        let meta = make_meta(None, None, Some(vec!["ci"]));
        assert!(!meta.has_tag("dev"));
    }

    // ── Manifest::empty ───────────────────────────────────────────────────────

    #[test]
    fn manifest_empty_has_no_entries() {
        let m = Manifest::empty();
        assert!(m.binaries.is_empty());
        assert!(m.libraries.is_empty());
    }

    // ── Manifest round-trip ───────────────────────────────────────────────────

    #[test]
    fn manifest_save_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("grip.toml");

        let mut m = Manifest::empty();
        m.binaries.insert(
            "rg".to_string(),
            BinaryEntry::Github(GithubEntry {
                repo: "BurntSushi/ripgrep".to_string(),
                version: Some("14.0.0".to_string()),
                asset_pattern: None,
                binary: None,
                extra_binaries: None,
                gpg_fingerprint: None,
                sig_asset_pattern: None,
                checksums_asset_pattern: None,
                meta: CommonMeta::default(),
            }),
        );
        m.save(&path).unwrap();

        let loaded = Manifest::load(&path).unwrap();
        assert_eq!(loaded.binaries.len(), 1);
        let entry = loaded.binaries.get("rg").unwrap();
        if let BinaryEntry::Github(g) = entry {
            assert_eq!(g.repo, "BurntSushi/ripgrep");
            assert_eq!(g.version.as_deref(), Some("14.0.0"));
        } else {
            panic!("expected Github entry");
        }
    }

    #[test]
    fn manifest_load_invalid_toml_returns_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("grip.toml");
        std::fs::write(&path, "not valid toml [[[[").unwrap();
        assert!(Manifest::load(&path).is_err());
    }

    // ── BinaryEntry::pin_version ──────────────────────────────────────────────

    #[test]
    fn pin_version_github() {
        let entry = BinaryEntry::Github(GithubEntry {
            repo: "cli/cli".to_string(),
            version: None,
            asset_pattern: None,
            binary: None,
            extra_binaries: None,
            gpg_fingerprint: None,
            sig_asset_pattern: None,
            checksums_asset_pattern: None,
            meta: CommonMeta::default(),
        });
        let pinned = entry.pin_version("2.40.0");
        if let BinaryEntry::Github(g) = pinned {
            assert_eq!(g.version.as_deref(), Some("2.40.0"));
        } else {
            panic!("expected Github entry");
        }
    }

    #[test]
    fn pin_version_apt() {
        let entry = BinaryEntry::Apt(AptEntry {
            package: "jq".to_string(),
            binary: None,
            extra_binaries: None,
            version: None,
            meta: CommonMeta::default(),
        });
        let pinned = entry.pin_version("1.6");
        if let BinaryEntry::Apt(a) = pinned {
            assert_eq!(a.version.as_deref(), Some("1.6"));
        } else {
            panic!("expected Apt entry");
        }
    }

    #[test]
    fn pin_version_dnf() {
        let entry = BinaryEntry::Dnf(DnfEntry {
            package: "jq".to_string(),
            binary: None,
            extra_binaries: None,
            version: None,
            meta: CommonMeta::default(),
        });
        let pinned = entry.pin_version("1.6");
        if let BinaryEntry::Dnf(d) = pinned {
            assert_eq!(d.version.as_deref(), Some("1.6"));
        } else {
            panic!("expected Dnf entry");
        }
    }

    #[test]
    fn pin_version_shell() {
        let entry = BinaryEntry::Shell(ShellEntry {
            install_cmd: "echo hi".to_string(),
            version: None,
            allow_shell: false,
            extra_binaries: None,
            meta: CommonMeta::default(),
        });
        let pinned = entry.pin_version("0.1.0");
        if let BinaryEntry::Shell(s) = pinned {
            assert_eq!(s.version.as_deref(), Some("0.1.0"));
        } else {
            panic!("expected Shell entry");
        }
    }

    // ── BinaryEntry::meta ─────────────────────────────────────────────────────

    #[test]
    fn binary_entry_meta_returns_inner_meta() {
        let meta = CommonMeta {
            required: Some(false),
            platforms: None,
            tags: None,
            post_install: None,
        };
        let entry = BinaryEntry::Github(GithubEntry {
            repo: "a/b".to_string(),
            version: None,
            asset_pattern: None,
            binary: None,
            extra_binaries: None,
            gpg_fingerprint: None,
            sig_asset_pattern: None,
            checksums_asset_pattern: None,
            meta: meta.clone(),
        });
        assert_eq!(entry.meta().required, Some(false));
    }

    // ── find_manifest_dir ─────────────────────────────────────────────────────

    #[test]
    fn find_manifest_dir_finds_in_start() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "").unwrap();
        let result = find_manifest_dir(tmp.path());
        assert_eq!(result, Some(tmp.path().to_path_buf()));
    }

    // ── BinaryEntry::is_version_pinned ───────────────────────────────────────

    #[test]
    fn github_entry_pinned_when_version_set() {
        let entry = BinaryEntry::Github(GithubEntry {
            repo: "jqlang/jq".to_string(),
            version: Some("1.7.1".to_string()),
            asset_pattern: None,
            binary: None,
            extra_binaries: None,
            gpg_fingerprint: None,
            sig_asset_pattern: None,
            checksums_asset_pattern: None,
            meta: CommonMeta::default(),
        });
        assert!(entry.is_version_pinned());
    }

    #[test]
    fn github_entry_unpinned_when_no_version() {
        let entry = BinaryEntry::Github(GithubEntry {
            repo: "jqlang/jq".to_string(),
            version: None,
            asset_pattern: None,
            binary: None,
            extra_binaries: None,
            gpg_fingerprint: None,
            sig_asset_pattern: None,
            checksums_asset_pattern: None,
            meta: CommonMeta::default(),
        });
        assert!(!entry.is_version_pinned());
    }

    #[test]
    fn apt_entry_pinned_when_version_set() {
        let entry = BinaryEntry::Apt(AptEntry {
            package: "jq".to_string(),
            binary: None,
            extra_binaries: None,
            version: Some("1.6".to_string()),
            meta: CommonMeta::default(),
        });
        assert!(entry.is_version_pinned());
    }

    #[test]
    fn apt_entry_unpinned_when_no_version() {
        let entry = BinaryEntry::Apt(AptEntry {
            package: "jq".to_string(),
            binary: None,
            extra_binaries: None,
            version: None,
            meta: CommonMeta::default(),
        });
        assert!(!entry.is_version_pinned());
    }

    #[test]
    fn url_entry_always_pinned() {
        let entry = BinaryEntry::Url(UrlEntry {
            url: "https://example.com/tool-1.0.tar.gz".to_string(),
            binary: None,
            extra_binaries: None,
            sha256: None,
            gpg_fingerprint: None,
            sig_url: None,
            signed_checksums_url: None,
            checksums_sig_url: None,
            meta: CommonMeta::default(),
        });
        assert!(entry.is_version_pinned());
    }

    #[test]
    fn shell_entry_pinned_when_version_set() {
        let entry = BinaryEntry::Shell(ShellEntry {
            install_cmd: "echo hi".to_string(),
            version: Some("2.0".to_string()),
            allow_shell: true,
            extra_binaries: None,
            meta: CommonMeta::default(),
        });
        assert!(entry.is_version_pinned());
    }

    #[test]
    fn shell_entry_unpinned_when_no_version() {
        let entry = BinaryEntry::Shell(ShellEntry {
            install_cmd: "curl ... | sh".to_string(),
            version: None,
            allow_shell: true,
            extra_binaries: None,
            meta: CommonMeta::default(),
        });
        assert!(!entry.is_version_pinned());
    }

    // ── BinaryEntry::source_label ─────────────────────────────────────────────

    #[test]
    fn source_label_matches_source_type() {
        let github = BinaryEntry::Github(GithubEntry {
            repo: "a/b".to_string(),
            version: None,
            asset_pattern: None,
            binary: None,
            extra_binaries: None,
            gpg_fingerprint: None,
            sig_asset_pattern: None,
            checksums_asset_pattern: None,
            meta: CommonMeta::default(),
        });
        assert_eq!(github.source_label(), "github");

        let apt = BinaryEntry::Apt(AptEntry {
            package: "jq".to_string(),
            binary: None,
            extra_binaries: None,
            version: None,
            meta: CommonMeta::default(),
        });
        assert_eq!(apt.source_label(), "apt");
    }

    #[test]
    fn find_manifest_dir_finds_in_parent() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "").unwrap();
        let child = tmp.path().join("subdir");
        std::fs::create_dir_all(&child).unwrap();
        let result = find_manifest_dir(&child);
        assert_eq!(result, Some(tmp.path().to_path_buf()));
    }

    #[test]
    fn find_manifest_dir_returns_none_when_absent() {
        let tmp = TempDir::new().unwrap();
        // No grip.toml written – should not find one (assuming tmp is isolated).
        // We start from a leaf directory well below the root of the temp tree.
        let leaf = tmp.path().join("a/b/c");
        std::fs::create_dir_all(&leaf).unwrap();
        // Only returns None when there's truly no grip.toml up the chain;
        // since TempDir is under /tmp, the walk will stop at filesystem root.
        let result = find_manifest_dir(&leaf);
        assert!(result.is_none());
    }
}
