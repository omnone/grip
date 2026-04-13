//! Shared GPG signature verification used by the GitHub and URL adapters.

use std::path::Path;

use crate::error::GripError;

/// Verify a detached GPG signature using the system `gpg` binary.
///
/// Uses `--status-fd 1` to parse the structured `VALIDSIG` output, which contains the full
/// 40-character fingerprint. The declared `fingerprint` is matched as a suffix of the actual
/// fingerprint so that both long key IDs (16 hex chars) and full fingerprints (40 hex chars) work.
///
/// Spaces and colons are stripped before comparison so both `AABBCCDD` and `AA BB CC DD` match.
pub(crate) fn verify_gpg_signature(
    archive_path: &Path,
    sig_path: &Path,
    fingerprint: &str,
    name: &str,
) -> Result<(), GripError> {
    verify_gpg_signature_with_cmd(archive_path, sig_path, fingerprint, name, "gpg")
}

/// Inner implementation — accepts a `gpg_cmd` so tests can pass a non-existent binary without
/// touching global state (no PATH mutation needed).
pub(crate) fn verify_gpg_signature_with_cmd(
    archive_path: &Path,
    sig_path: &Path,
    fingerprint: &str,
    name: &str,
    gpg_cmd: &str,
) -> Result<(), GripError> {
    // Check availability first to give a clear error rather than an opaque Io error.
    let available = std::process::Command::new(gpg_cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !available {
        return Err(GripError::GpgNotFound);
    }

    let output = std::process::Command::new(gpg_cmd)
        .args(["--status-fd", "1", "--verify"])
        .arg(sig_path)
        .arg(archive_path)
        .output()
        .map_err(GripError::Io)?;

    // Parse VALIDSIG from --status-fd output regardless of exit code;
    // gpg may exit non-zero even for "good" sigs when the key is untrusted.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let norm_expected = fingerprint.replace([' ', ':'], "").to_uppercase();

    let valid_sig_fp: Option<String> = stdout.lines().find_map(|line| {
        // Format: [GNUPG:] VALIDSIG <40-char-fp> <date> <timestamp> ...
        let rest = line.strip_prefix("[GNUPG:] VALIDSIG ")?;
        Some(rest.split_whitespace().next()?.to_uppercase())
    });

    match valid_sig_fp {
        Some(actual_fp) if actual_fp.ends_with(&norm_expected) => Ok(()),
        Some(actual_fp) => Err(GripError::GpgVerificationFailed {
            name: name.to_string(),
            detail: format!(
                "fingerprint mismatch: signature is from '{}' but grip.toml declares '{}'",
                actual_fp, fingerprint
            ),
        }),
        None => {
            // No VALIDSIG line — the signature is invalid or gpg exited with an error.
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(GripError::GpgVerificationFailed {
                name: name.to_string(),
                detail: stderr.trim().to_string(),
            })
        }
    }
}

/// Parse a checksums file (sha256sum / shasum format) and return the hash for `filename`.
///
/// Handles the two most common layouts:
/// - `<hash>  <filename>` — sha256sum standard (two spaces)
/// - `<hash> *<filename>` — binary mode (space + asterisk)
///
/// The filename is matched by basename so entries like `<hash>  dist/tool.tar.gz` match
/// when `filename` is `"tool.tar.gz"`.
pub(crate) fn parse_checksums_for_file(content: &str, filename: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split on the first whitespace character to separate hash from the path field.
        let ws = line.find(|c: char| c.is_ascii_whitespace())?;
        let hash = line[..ws].trim();
        // Strip leading spaces and the optional asterisk (binary mode marker).
        let filepath = line[ws..].trim_start_matches(|c: char| c == ' ' || c == '*');
        let basename = filepath.rsplit('/').next().unwrap_or(filepath);
        if basename == filename {
            return Some(hash.to_lowercase());
        }
    }
    None
}

/// Verify a signed checksums file and check the downloaded archive against it.
///
/// Steps:
/// 1. Verify `checksums_sig_path` is a valid GPG signature of `checksums_path` by `fingerprint`.
/// 2. Parse `checksums_path` to extract the authoritative SHA-256 for `asset_filename`.
/// 3. Compute the SHA-256 of `archive_path` and compare against the extracted hash.
pub(crate) fn verify_signed_checksums(
    archive_path: &Path,
    checksums_path: &Path,
    checksums_sig_path: &Path,
    fingerprint: &str,
    asset_filename: &str,
    name: &str,
) -> Result<(), GripError> {
    verify_signed_checksums_with_cmd(
        archive_path,
        checksums_path,
        checksums_sig_path,
        fingerprint,
        asset_filename,
        name,
        "gpg",
    )
}

/// Testable variant — accepts an explicit `gpg_cmd` to avoid touching global PATH state.
pub(crate) fn verify_signed_checksums_with_cmd(
    archive_path: &Path,
    checksums_path: &Path,
    checksums_sig_path: &Path,
    fingerprint: &str,
    asset_filename: &str,
    name: &str,
    gpg_cmd: &str,
) -> Result<(), GripError> {
    // Step 1: verify GPG signature of the checksums file.
    verify_gpg_signature_with_cmd(checksums_path, checksums_sig_path, fingerprint, name, gpg_cmd)?;

    // Step 2: parse the checksums file for the expected hash.
    let content = std::fs::read_to_string(checksums_path).map_err(GripError::Io)?;
    let expected = parse_checksums_for_file(&content, asset_filename).ok_or_else(|| {
        GripError::GpgVerificationFailed {
            name: name.to_string(),
            detail: format!(
                "'{}' not found in checksums file; \
                 verify the asset filename matches the entry in the checksums file exactly",
                asset_filename
            ),
        }
    })?;

    // Step 3: compare against the actual hash of the downloaded archive.
    let actual = crate::checksum::sha256_file(archive_path).map_err(GripError::Io)?;
    if actual != expected {
        return Err(GripError::ChecksumMismatch {
            expected,
            got: actual,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpg_not_available_returns_gpg_not_found() {
        // Use a clearly non-existent command — avoids any PATH mutation
        // (which would be racy with concurrently running tests).
        let tmp = tempfile::TempDir::new().unwrap();
        let dummy = tmp.path().join("archive");
        let sig = tmp.path().join("archive.sig");
        std::fs::write(&dummy, b"data").unwrap();
        std::fs::write(&sig, b"sig").unwrap();

        let r = verify_gpg_signature_with_cmd(
            &dummy,
            &sig,
            "AABBCCDD",
            "tool",
            "definitely-not-gpg-xyzzy-grip-test",
        );
        assert!(
            matches!(r, Err(GripError::GpgNotFound)),
            "expected GpgNotFound, got {r:?}"
        );
    }

    // ── parse_checksums_for_file ──────────────────────────────────────────────

    #[test]
    fn parses_standard_two_space_format() {
        let content = "abc123  tool.tar.gz\ndef456  other.tar.gz\n";
        assert_eq!(
            parse_checksums_for_file(content, "tool.tar.gz"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn parses_binary_mode_asterisk_format() {
        let content = "abc123 *tool.tar.gz\n";
        assert_eq!(
            parse_checksums_for_file(content, "tool.tar.gz"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn matches_by_basename_ignoring_path_prefix() {
        let content = "abc123  dist/linux/tool.tar.gz\n";
        assert_eq!(
            parse_checksums_for_file(content, "tool.tar.gz"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn returns_none_when_filename_absent() {
        let content = "abc123  other.tar.gz\n";
        assert!(parse_checksums_for_file(content, "tool.tar.gz").is_none());
    }

    #[test]
    fn skips_comment_and_blank_lines() {
        let content = "\n# generated by release script\nabc123  tool.tar.gz\n";
        assert_eq!(
            parse_checksums_for_file(content, "tool.tar.gz"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn normalises_hash_to_lowercase() {
        let content = "ABC123  tool.tar.gz\n";
        assert_eq!(
            parse_checksums_for_file(content, "tool.tar.gz"),
            Some("abc123".to_string())
        );
    }

    // ── verify_signed_checksums_with_cmd ─────────────────────────────────────

    #[test]
    fn signed_checksums_gpg_not_found_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive = tmp.path().join("tool.tar.gz");
        let checksums = tmp.path().join("SHA256SUMS");
        let checksums_sig = tmp.path().join("SHA256SUMS.sig");
        std::fs::write(&archive, b"binary data").unwrap();
        std::fs::write(&checksums, b"abc123  tool.tar.gz\n").unwrap();
        std::fs::write(&checksums_sig, b"sig data").unwrap();

        let r = verify_signed_checksums_with_cmd(
            &archive,
            &checksums,
            &checksums_sig,
            "AABBCCDD",
            "tool.tar.gz",
            "tool",
            "definitely-not-gpg-xyzzy-grip-test",
        );
        assert!(matches!(r, Err(GripError::GpgNotFound)));
    }
}
