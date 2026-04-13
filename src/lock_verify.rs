//! Verify on-disk binaries against sha256 hashes recorded in `grip.lock`.
//!
//! This is the backing logic for `grip lock verify` — a tamper-detection
//! command that reads the lock file directly (not the manifest) and
//! re-hashes every `.bin/` binary that has a recorded sha256.

use std::path::{Path, PathBuf};

use crate::checksum::sha256_file;
use crate::config::lockfile::LockFile;
use crate::config::manifest::find_manifest_dir;
use crate::error::GripError;

/// Outcome of verifying one binary.
#[derive(Debug, PartialEq)]
pub enum VerifyStatus {
    /// sha256 matched.
    Ok,
    /// No sha256 in the lock entry — cannot verify.
    NoChecksum,
    /// Binary file is missing from `.bin/`.
    Missing,
    /// sha256 does not match what is on disk.
    Mismatch { expected: String, got: String },
}

/// Summary of a `grip lock verify` run.
#[derive(Debug, Default)]
pub struct VerifyResult {
    /// Entries that passed (sha256 matched).
    pub verified: Vec<String>,
    /// Entries skipped because the lock has no sha256 to compare.
    pub no_checksum: Vec<String>,
    /// Entries that failed (name, human-readable reason).
    pub failed: Vec<(String, String)>,
}

/// Verify a single binary by name.
///
/// * `name` — key in `grip.lock`
/// * `expected_sha` — sha256 from the lock entry (if any)
/// * `bin_dir` — path to the `.bin/` directory
pub fn verify_one(name: &str, expected_sha: Option<&str>, bin_dir: &Path) -> VerifyStatus {
    let Some(expected) = expected_sha else {
        return VerifyStatus::NoChecksum;
    };

    let bin_path = bin_dir.join(name);
    if !bin_path.exists() {
        return VerifyStatus::Missing;
    }

    match sha256_file(&bin_path) {
        Ok(got) if got == expected => VerifyStatus::Ok,
        Ok(got) => VerifyStatus::Mismatch {
            expected: expected.to_string(),
            got,
        },
        Err(_) => VerifyStatus::Missing,
    }
}

/// Walk every binary entry in `grip.lock` and verify its on-disk sha256.
///
/// Does **not** re-download anything or touch `grip.toml`; purely a
/// read-and-hash operation suitable for CI.
pub fn run_lock_verify(root: Option<PathBuf>) -> Result<VerifyResult, GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };

    let lock_path = project_root.join("grip.lock");
    let bin_dir = project_root.join(".bin");
    let lock = LockFile::load(&lock_path)?;

    let mut out = VerifyResult::default();

    for entry in &lock.entries {
        match verify_one(&entry.name, entry.sha256.as_deref(), &bin_dir) {
            VerifyStatus::Ok => out.verified.push(entry.name.clone()),
            VerifyStatus::NoChecksum => out.no_checksum.push(entry.name.clone()),
            VerifyStatus::Missing => out
                .failed
                .push((entry.name.clone(), "binary missing from .bin/".to_string())),
            VerifyStatus::Mismatch { expected, got } => out.failed.push((
                entry.name.clone(),
                format!("checksum mismatch — lock: {expected}  disk: {got}"),
            )),
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

    fn make_entry(name: &str, sha256: Option<&str>) -> LockEntry {
        LockEntry {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            source: "github".to_string(),
            url: None,
            sha256: sha256.map(String::from),
            installed_at: Utc::now(),
            extra_binaries: vec![],
            auto_binary: None,
            auto_extra_binaries: vec![],
        }
    }

    // ── verify_one ────────────────────────────────────────────────────────────

    #[test]
    fn verify_one_no_checksum() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("jq"), b"stub").unwrap();
        assert_eq!(verify_one("jq", None, tmp.path()), VerifyStatus::NoChecksum);
    }

    #[test]
    fn verify_one_missing_binary() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            verify_one("jq", Some("deadbeef"), tmp.path()),
            VerifyStatus::Missing
        );
    }

    #[test]
    fn verify_one_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("jq"), b"stub").unwrap();
        assert!(matches!(
            verify_one("jq", Some("deadbeef"), tmp.path()),
            VerifyStatus::Mismatch { .. }
        ));
    }

    #[test]
    fn verify_one_ok() {
        let tmp = TempDir::new().unwrap();
        let content = b"the real binary content";
        std::fs::write(tmp.path().join("jq"), content).unwrap();
        let sha = sha256_file(&tmp.path().join("jq")).unwrap();
        assert_eq!(verify_one("jq", Some(&sha), tmp.path()), VerifyStatus::Ok);
    }

    // ── run_lock_verify ───────────────────────────────────────────────────────

    #[test]
    fn run_lock_verify_empty_lock_passes() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "[binaries]\n").unwrap();
        let result = run_lock_verify(Some(tmp.path().to_path_buf())).unwrap();
        assert!(result.failed.is_empty());
        assert!(result.verified.is_empty());
        assert!(result.no_checksum.is_empty());
    }

    #[test]
    fn run_lock_verify_detects_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "[binaries]\n").unwrap();

        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("jq"), b"modified binary").unwrap();

        // A sha256 that will never match the file content above
        let wrong_sha = "0000000000000000000000000000000000000000000000000000000000000000";
        let mut lf = LockFile::default();
        lf.upsert(make_entry("jq", Some(wrong_sha)));
        lf.save(&tmp.path().join("grip.lock")).unwrap();

        let result = run_lock_verify(Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, "jq");
        assert!(result.failed[0].1.contains("checksum mismatch"));
    }

    #[test]
    fn run_lock_verify_passes_correct_sha() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "[binaries]\n").unwrap();

        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let content = b"the real binary";
        std::fs::write(bin_dir.join("jq"), content).unwrap();
        let sha = sha256_file(&bin_dir.join("jq")).unwrap();

        let mut lf = LockFile::default();
        lf.upsert(make_entry("jq", Some(&sha)));
        lf.save(&tmp.path().join("grip.lock")).unwrap();

        let result = run_lock_verify(Some(tmp.path().to_path_buf())).unwrap();
        assert!(result.failed.is_empty());
        assert_eq!(result.verified.len(), 1);
        assert_eq!(result.verified[0], "jq");
    }

    #[test]
    fn run_lock_verify_no_checksum_is_not_failed() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "[binaries]\n").unwrap();

        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("jq"), b"stub").unwrap();

        let mut lf = LockFile::default();
        lf.upsert(make_entry("jq", None)); // no sha256
        lf.save(&tmp.path().join("grip.lock")).unwrap();

        let result = run_lock_verify(Some(tmp.path().to_path_buf())).unwrap();
        assert!(result.failed.is_empty());
        assert_eq!(result.no_checksum.len(), 1);
        assert_eq!(result.no_checksum[0], "jq");
    }

    #[test]
    fn run_lock_verify_missing_binary_is_failed() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "[binaries]\n").unwrap();
        // .bin/ exists but binary is absent
        std::fs::create_dir_all(tmp.path().join(".bin")).unwrap();

        let mut lf = LockFile::default();
        lf.upsert(make_entry("jq", Some("aaaa")));
        lf.save(&tmp.path().join("grip.lock")).unwrap();

        let result = run_lock_verify(Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(result.failed[0].1.contains("missing"));
    }

    #[test]
    fn run_lock_verify_mixed_results() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("grip.toml"), "[binaries]\n").unwrap();

        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        // jq — correct sha
        let content = b"jq content";
        std::fs::write(bin_dir.join("jq"), content).unwrap();
        let sha_jq = sha256_file(&bin_dir.join("jq")).unwrap();

        // rg — tampered (wrong sha)
        std::fs::write(bin_dir.join("rg"), b"rg content").unwrap();

        // fd — no sha in lock
        std::fs::write(bin_dir.join("fd"), b"fd content").unwrap();

        let mut lf = LockFile::default();
        lf.upsert(make_entry("jq", Some(&sha_jq)));
        lf.upsert(make_entry(
            "rg",
            Some("badhash00000000000000000000000000000000000000000000000000000000"),
        ));
        lf.upsert(make_entry("fd", None));
        lf.save(&tmp.path().join("grip.lock")).unwrap();

        let result = run_lock_verify(Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(result.verified.len(), 1);
        assert_eq!(result.no_checksum.len(), 1);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, "rg");
    }
}
