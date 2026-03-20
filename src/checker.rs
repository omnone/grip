//! Verify installed binaries against `grip.lock` (and optional manifest version pins) without installing.

use std::path::{Path, PathBuf};

use crate::checksum::sha256_file;
use crate::config::lockfile::LockFile;
use crate::config::manifest::{find_manifest_dir, BinaryEntry, Manifest};
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
    /// Total entries in `grip.toml` (before platform / tag filters).
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
    out.declared = manifest.binaries.len();

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

    Ok(out)
}
