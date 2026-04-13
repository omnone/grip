//! Verify installed binaries against `grip.lock` (and optional manifest version pins) without installing.

use std::path::{Path, PathBuf};

use crate::checksum::sha256_file;
use crate::config::lockfile::LockFile;
use crate::config::manifest::{find_manifest_dir, BinaryEntry, LibraryEntry, Manifest};
use crate::error::GripError;
use crate::platform::Platform;

/// Outcome of `grip check` for one binary.
#[derive(Debug)]
pub enum CheckStatus {
    Ok,
    MissingBinary,
    MissingLockEntry,
    VersionMismatch { expected: String, locked: String },
    ChecksumMismatch { expected: String, got: String },
    NoChecksumInLock,
}

/// Summary of a `grip check` run.
#[derive(Debug, Default)]
pub struct CheckResult {
    /// Total entries in `grip.toml` (binaries + libraries, before platform / tag filters).
    pub declared: usize,
    /// Entries that were checked (after platform / tag filters).
    pub examined: usize,
    pub passed: Vec<String>,
    /// Required entries that failed (name, message).
    pub failed: Vec<(String, String)>,
    /// Optional (`required = false`) entries that failed.
    pub warned: Vec<(String, String)>,
    /// Names that passed but lock had no SHA256 to verify.
    pub no_checksum: Vec<String>,
}

/// Compare manifest-pinned version to lock file version (normalizes leading `v`, case-insensitive).
fn versions_match(manifest_ver: &str, lock_ver: &str) -> bool {
    fn norm(s: &str) -> String {
        s.trim().trim_start_matches('v').to_lowercase()
    }
    norm(manifest_ver) == norm(lock_ver)
}

fn manifest_pinned_version(entry: &BinaryEntry) -> Option<&str> {
    match entry {
        BinaryEntry::Apt(a) => a.version.as_deref(),
        BinaryEntry::Dnf(d) => d.version.as_deref(),
        BinaryEntry::Github(g) => g.version.as_deref(),
        BinaryEntry::Url(_) => None,
        BinaryEntry::Shell(s) => s.version.as_deref(),
    }
}

fn check_one(
    name: &str,
    entry: &BinaryEntry,
    bin_dir: &Path,
    lock: &LockFile,
) -> Result<CheckStatus, GripError> {
    let bin_path = bin_dir.join(name);
    if !bin_path.exists() {
        return Ok(CheckStatus::MissingBinary);
    }

    let Some(lock_entry) = lock.get(name) else {
        return Ok(CheckStatus::MissingLockEntry);
    };

    if let Some(pin) = manifest_pinned_version(entry) {
        if !versions_match(pin, &lock_entry.version) {
            return Ok(CheckStatus::VersionMismatch {
                expected: pin.to_string(),
                locked: lock_entry.version.clone(),
            });
        }
    }

    match &lock_entry.sha256 {
        Some(expected) => {
            let got = sha256_file(&bin_path)?;
            if &got != expected {
                return Ok(CheckStatus::ChecksumMismatch {
                    expected: expected.clone(),
                    got,
                });
            }
            Ok(CheckStatus::Ok)
        }
        None => Ok(CheckStatus::NoChecksumInLock),
    }
}

fn check_one_library(
    name: &str,
    entry: &LibraryEntry,
    lock: &LockFile,
) -> CheckStatus {
    let Some(lock_entry) = lock.get_library(name) else {
        return CheckStatus::MissingLockEntry;
    };

    // Verify the package is still actually installed on the system.
    let is_installed = match entry {
        LibraryEntry::Apt(a) => std::process::Command::new("dpkg-query")
            .args(["-W", "-f=${Status}", &a.package])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("install ok installed"))
            .unwrap_or(false),
        LibraryEntry::Dnf(d) => std::process::Command::new("rpm")
            .args(["-q", &d.package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false),
    };

    if !is_installed {
        return CheckStatus::MissingBinary;
    }

    // Check version pin if set in manifest.
    let manifest_ver = match entry {
        LibraryEntry::Apt(a) => a.version.as_deref(),
        LibraryEntry::Dnf(d) => d.version.as_deref(),
    };
    if let Some(pin) = manifest_ver {
        if !versions_match(pin, &lock_entry.version) {
            return CheckStatus::VersionMismatch {
                expected: pin.to_string(),
                locked: lock_entry.version.clone(),
            };
        }
    }

    CheckStatus::NoChecksumInLock
}

/// Verify on-disk binaries against `grip.lock` and optional manifest version pins. Does not install or modify files.
pub fn run_check(tag: Option<&str>, root: Option<PathBuf>) -> Result<CheckResult, GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };

    let manifest_path = project_root.join("grip.toml");
    let lock_path = project_root.join("grip.lock");
    let bin_dir = project_root.join(".bin");

    let manifest = Manifest::load(&manifest_path)?;
    let lock = LockFile::load(&lock_path)?;
    let platform = Platform::current();

    let mut out = CheckResult::default();
    out.declared = manifest.binaries.len() + manifest.libraries.len();

    for (name, entry) in &manifest.binaries {
        let meta = entry.meta();

        if !meta.matches_platform(platform.os_str()) {
            continue;
        }

        if let Some(t) = tag {
            if !meta.has_tag(t) {
                continue;
            }
        }

        out.examined += 1;
        let required = meta.is_required();
        let status = check_one(name, entry, &bin_dir, &lock)?;

        match status {
            CheckStatus::Ok => out.passed.push(name.clone()),
            CheckStatus::NoChecksumInLock => {
                out.passed.push(name.clone());
                out.no_checksum.push(name.clone());
            }
            CheckStatus::MissingBinary => {
                let msg = format!("binary not found at {}", bin_dir.join(name).display());
                if required {
                    out.failed.push((name.clone(), msg));
                } else {
                    out.warned.push((name.clone(), msg));
                }
            }
            CheckStatus::MissingLockEntry => {
                let msg = "no entry in grip.lock (run `grip install`)".to_string();
                if required {
                    out.failed.push((name.clone(), msg));
                } else {
                    out.warned.push((name.clone(), msg));
                }
            }
            CheckStatus::VersionMismatch { expected, locked } => {
                let msg = format!("version mismatch: grip.toml wants {expected}, grip.lock has {locked}");
                if required {
                    out.failed.push((name.clone(), msg));
                } else {
                    out.warned.push((name.clone(), msg));
                }
            }
            CheckStatus::ChecksumMismatch { expected, got } => {
                let msg = format!("checksum mismatch: expected {expected}, got {got}");
                if required {
                    out.failed.push((name.clone(), msg));
                } else {
                    out.warned.push((name.clone(), msg));
                }
            }
        }
    }

    // ── Library checks ──────────────────────────────────────────────────────
    for (name, entry) in &manifest.libraries {
        let meta = entry.meta();

        if !meta.matches_platform(platform.os_str()) {
            continue;
        }

        if let Some(t) = tag {
            if !meta.has_tag(t) {
                continue;
            }
        }

        out.examined += 1;
        let required = meta.is_required();
        let status = check_one_library(name, entry, &lock);

        match status {
            CheckStatus::Ok | CheckStatus::NoChecksumInLock => {
                out.passed.push(name.clone());
            }
            CheckStatus::MissingBinary => {
                let msg = format!("library `{}` is not installed on this system", name);
                if required {
                    out.failed.push((name.clone(), msg));
                } else {
                    out.warned.push((name.clone(), msg));
                }
            }
            CheckStatus::MissingLockEntry => {
                let msg = "no entry in grip.lock (run `grip install`)".to_string();
                if required {
                    out.failed.push((name.clone(), msg));
                } else {
                    out.warned.push((name.clone(), msg));
                }
            }
            CheckStatus::VersionMismatch { expected, locked } => {
                let msg = format!("version mismatch: grip.toml wants {expected}, grip.lock has {locked}");
                if required {
                    out.failed.push((name.clone(), msg));
                } else {
                    out.warned.push((name.clone(), msg));
                }
            }
            CheckStatus::ChecksumMismatch { .. } => {
                // Libraries have no checksum; this branch is unreachable.
                out.passed.push(name.clone());
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    use crate::config::lockfile::{LockEntry, LockFile};
    use crate::config::manifest::{
        BinaryEntry, CommonMeta, GithubEntry,
    };

    // ── versions_match ────────────────────────────────────────────────────────

    #[test]
    fn versions_match_identical() {
        assert!(versions_match("1.0.0", "1.0.0"));
    }

    #[test]
    fn versions_match_v_prefix_stripped() {
        assert!(versions_match("1.0.0", "v1.0.0"));
        assert!(versions_match("v1.0.0", "1.0.0"));
        assert!(versions_match("v1.0.0", "v1.0.0"));
    }

    #[test]
    fn versions_match_case_insensitive() {
        assert!(versions_match("1.0.0-BETA", "1.0.0-beta"));
    }

    #[test]
    fn versions_match_different_versions() {
        assert!(!versions_match("1.0.0", "2.0.0"));
        assert!(!versions_match("v1.2.3", "v1.2.4"));
    }

    // ── check_one helpers ─────────────────────────────────────────────────────

    fn make_lock_entry(name: &str, version: &str, sha256: Option<&str>) -> LockEntry {
        LockEntry {
            name: name.to_string(),
            version: version.to_string(),
            source: "github".to_string(),
            url: None,
            sha256: sha256.map(String::from),
            installed_at: Utc::now(),
            auto_binary: None,
        }
    }

    fn github_entry(version: Option<&str>) -> BinaryEntry {
        BinaryEntry::Github(GithubEntry {
            repo: "a/b".to_string(),
            version: version.map(String::from),
            asset_pattern: None,
            binary: None,
            gpg_fingerprint: None,
            sig_asset_pattern: None,
            checksums_asset_pattern: None,
            meta: CommonMeta::default(),
        })
    }

    // ── check_one: MissingBinary ──────────────────────────────────────────────

    #[test]
    fn check_one_missing_binary() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        let lf = LockFile::default();
        let status = check_one("jq", &github_entry(None), &bin_dir, &lf).unwrap();
        assert!(matches!(status, CheckStatus::MissingBinary));
    }

    // ── check_one: MissingLockEntry ───────────────────────────────────────────

    #[test]
    fn check_one_missing_lock_entry() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("jq"), b"stub").unwrap();

        let lf = LockFile::default(); // no entries
        let status = check_one("jq", &github_entry(None), &bin_dir, &lf).unwrap();
        assert!(matches!(status, CheckStatus::MissingLockEntry));
    }

    // ── check_one: VersionMismatch ────────────────────────────────────────────

    #[test]
    fn check_one_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("jq"), b"stub").unwrap();

        let mut lf = LockFile::default();
        lf.upsert(make_lock_entry("jq", "1.6.0", None));

        let entry = github_entry(Some("1.7.0")); // pinned to different version
        let status = check_one("jq", &entry, &bin_dir, &lf).unwrap();
        assert!(matches!(status, CheckStatus::VersionMismatch { .. }));
    }

    // ── check_one: ChecksumMismatch ───────────────────────────────────────────

    #[test]
    fn check_one_checksum_mismatch() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("jq"), b"hello").unwrap();

        let mut lf = LockFile::default();
        lf.upsert(make_lock_entry("jq", "1.7.0", Some("deadbeef")));

        let entry = github_entry(Some("1.7.0"));
        let status = check_one("jq", &entry, &bin_dir, &lf).unwrap();
        assert!(matches!(status, CheckStatus::ChecksumMismatch { .. }));
    }

    // ── check_one: Ok ─────────────────────────────────────────────────────────

    #[test]
    fn check_one_ok() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let content = b"hello";
        std::fs::write(bin_dir.join("jq"), content).unwrap();

        // Compute the real SHA256
        let sha = crate::checksum::sha256_file(&bin_dir.join("jq")).unwrap();

        let mut lf = LockFile::default();
        lf.upsert(make_lock_entry("jq", "1.7.0", Some(&sha)));

        let entry = github_entry(Some("1.7.0"));
        let status = check_one("jq", &entry, &bin_dir, &lf).unwrap();
        assert!(matches!(status, CheckStatus::Ok));
    }

    // ── check_one: NoChecksumInLock ───────────────────────────────────────────

    #[test]
    fn check_one_no_checksum_in_lock() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("jq"), b"stub").unwrap();

        let mut lf = LockFile::default();
        lf.upsert(make_lock_entry("jq", "1.7.0", None)); // no sha256

        let entry = github_entry(Some("1.7.0"));
        let status = check_one("jq", &entry, &bin_dir, &lf).unwrap();
        assert!(matches!(status, CheckStatus::NoChecksumInLock));
    }

    // ── run_check: end-to-end with temp project ───────────────────────────────

    #[test]
    fn run_check_empty_manifest_passes() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "[binaries]\n").unwrap();
        let result = run_check(None, Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(result.declared, 0);
        assert!(result.failed.is_empty());
    }

    #[test]
    fn run_check_detects_missing_binary() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1" }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();
        // No .bin/jq, no grip.lock
        let result = run_check(None, Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, "jq");
    }

    #[test]
    fn run_check_tag_filter_skips_untagged() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1" }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();
        // Filter by a tag that jq doesn't have — nothing examined, nothing failed
        let result = run_check(Some("ci"), Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(result.examined, 0);
        assert!(result.failed.is_empty());
    }

    #[test]
    fn run_check_optional_entry_is_warned_not_failed() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1", required = false }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();
        let result = run_check(None, Some(tmp.path().to_path_buf())).unwrap();
        assert!(result.failed.is_empty());
        assert_eq!(result.warned.len(), 1);
    }

    #[test]
    fn run_check_shell_entry_without_version_accepts_any_lock_version() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
mytool = { source = "shell", install_cmd = "echo hi" }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();

        // Create .bin/mytool
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let content = b"stub";
        std::fs::write(bin_dir.join("mytool"), content).unwrap();
        let sha = crate::checksum::sha256_file(&bin_dir.join("mytool")).unwrap();

        // Write grip.lock with matching sha
        let lock_toml = format!(
            r#"[[binary]]
name = "mytool"
version = "any"
source = "shell"
sha256 = "{sha}"
installed_at = "2024-01-01T00:00:00Z"
"#
        );
        std::fs::write(tmp.path().join("grip.lock"), lock_toml).unwrap();

        let result = run_check(None, Some(tmp.path().to_path_buf())).unwrap();
        assert!(result.failed.is_empty());
        assert_eq!(result.passed.len(), 1);
    }
}
