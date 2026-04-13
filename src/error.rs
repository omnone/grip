//! Error types for the binary manager.

use thiserror::Error;

/// All errors that can occur during binary management operations.
#[derive(Debug, Error)]
pub enum GripError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// SHA-256 of the downloaded file did not match the expected value.
    #[error("Checksum mismatch: expected {expected}, got {got}")]
    ChecksumMismatch { expected: String, got: String },
    /// The current platform is not supported by the requested adapter.
    #[error("Unsupported platform for adapter '{adapter}'")]
    UnsupportedPlatform { adapter: String },
    /// The expected binary name was not found inside the downloaded archive.
    #[error("Binary not found in archive: {0}")]
    BinaryNotFound(String),
    /// No `grip.toml` was found in the current directory or any parent.
    #[error("Manifest not found (no grip.toml in current or parent dirs)")]
    ManifestNotFound,
    /// A subprocess exited with a non-zero status.
    #[error("Command failed: {0}")]
    CommandFailed(String),
    /// The GitHub API returned an error or unexpected response.
    #[error("GitHub API error: {0}")]
    GitHubApi(String),
    /// No release asset matched the provided glob pattern.
    #[error("No matching asset found for pattern '{0}'")]
    NoMatchingAsset(String),
    /// The `source` field in `grip.toml` names an unknown adapter.
    #[error("Unknown source adapter: {0}")]
    UnknownAdapter(String),
    /// The current user lacks the privileges needed to run the package manager.
    #[error("insufficient privileges: {hint}")]
    InsufficientPrivileges { hint: String },
    /// Shell install is blocked because `allow_shell` is not explicitly set to `true`.
    #[error(
        "shell install for '{name}' is blocked: `allow_shell` is not set to true in grip.toml"
    )]
    ShellNotAllowed { name: String },
    /// `gpg` binary was not found on PATH when signature verification was requested.
    #[error("gpg not found on PATH: install gpg to verify release signatures")]
    GpgNotFound,
    /// GPG signature verification failed or the fingerprint did not match.
    #[error("GPG signature verification failed for '{name}': {detail}")]
    GpgVerificationFailed { name: String, detail: String },
    /// One or more entries have no version pin and `--require-pins` was set.
    #[error(
        "the following entries have no version pin: {names}\n\
         Run `grip sync` without `--require-pins` to install the latest versions, \
         then pin them with `grip add <name>@<version>`."
    )]
    UnpinnedEntries { names: String },
    /// A catch-all for errors that don't fit the variants above.
    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for GripError {
    fn from(e: anyhow::Error) -> Self {
        GripError::Other(e.to_string())
    }
}

impl GripError {
    /// Optional short hint printed after `error:` (stderr).
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            GripError::ManifestNotFound => Some(
                "Run `grip init` in your project directory, or change to a directory that contains grip.toml.",
            ),
            GripError::UnknownAdapter(_) => Some("Valid sources: github, url, apt, dnf, shell."),
            GripError::TomlParse(_) => Some("Fix the syntax in grip.toml."),
            GripError::ChecksumMismatch { .. } => Some(
                "Re-run `grip install` to re-download, or update the expected hash in your manifest or lock file.",
            ),
            GripError::NoMatchingAsset(_) => Some(
                "Adjust `asset_pattern` in grip.toml or pin a release with assets that match this platform.",
            ),
            GripError::BinaryNotFound(_) => Some(
                "Set `binary` in grip.toml to the executable name inside the archive, or fix `asset_pattern`.",
            ),
            GripError::UnsupportedPlatform { .. } => Some(
                "This adapter does not support the current OS; use a different source or add a platform-specific entry.",
            ),
            GripError::GitHubApi(_) => Some("Check the repository name, release tags, and your network access."),
            GripError::InsufficientPrivileges { .. } => Some(
                "Run grip as root, or configure passwordless sudo for apt-get/dnf.",
            ),
            GripError::ShellNotAllowed { .. } => Some(
                "Review install_cmd in grip.toml, then add `allow_shell = true` to the entry to permit execution. \
                 Use `grip add --source shell --allow-shell` to set this flag when adding the entry.",
            ),
            GripError::GpgNotFound => Some(
                "Install gpg (e.g. `apt install gnupg` or `brew install gnupg`) \
                 or remove gpg_fingerprint from grip.toml to skip signature verification.",
            ),
            GripError::GpgVerificationFailed { .. } => Some(
                "The release asset may have been tampered with, or the key is not in your keyring. \
                 Import the maintainer's key with `gpg --recv-keys <fingerprint>` and re-run.",
            ),
            GripError::UnpinnedEntries { .. } => Some(
                "Pin each entry by adding a version: `grip add <name>@<version>`, \
                 or remove `--require-pins` to allow floating versions.",
            ),
            GripError::CommandFailed(_) => Some("Inspect the command output above; fix install_cmd or package name."),
            GripError::Io(_) => Some("Check file permissions and paths (use -v for more detail)."),
            GripError::Http(_) => Some("Check your network and proxy settings (use -v for more detail)."),
            GripError::TomlSerialize(_) => Some("If this persists, report a bug with your grip.toml contents."),
            GripError::Other(s) if s.contains("--repo required") => {
                Some("For GitHub, pass `--repo owner/name` or use `grip add owner/repo`.")
            }
            GripError::Other(s) if s.contains("--url required") => Some("Pass `--url` for url source."),
            GripError::Other(s) if s.contains("not found in grip.toml") => {
                Some("List entries in grip.toml or run `grip add <name>` first.")
            }
            _ => None,
        }
    }

    /// Message for the `error:` line; respects `verbose` for IO/HTTP detail.
    pub fn format_user_message(&self, verbose: bool) -> String {
        match self {
            GripError::Io(e) if !verbose => format!("I/O error: {e}"),
            GripError::Http(e) if !verbose => format!("HTTP error: {e}"),
            GripError::ManifestNotFound => {
                "Could not find grip.toml in the current directory or any parent.".to_string()
            }
            _ => self.to_string(),
        }
    }
}

/// Print a consistent error block to stderr.
pub fn print_grip_error(err: &GripError, verbose: bool) {
    eprintln!("error: {}", err.format_user_message(verbose));
    if let Some(h) = err.hint() {
        eprintln!("hint: {h}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── hint ──────────────────────────────────────────────────────────────────

    #[test]
    fn manifest_not_found_has_hint() {
        assert!(GripError::ManifestNotFound.hint().is_some());
    }

    #[test]
    fn unknown_adapter_has_hint() {
        assert!(GripError::UnknownAdapter("nope".into()).hint().is_some());
    }

    #[test]
    fn checksum_mismatch_has_hint() {
        let err = GripError::ChecksumMismatch {
            expected: "abc".into(),
            got: "def".into(),
        };
        assert!(err.hint().is_some());
    }

    #[test]
    fn no_matching_asset_has_hint() {
        assert!(GripError::NoMatchingAsset("*.tar.gz".into())
            .hint()
            .is_some());
    }

    #[test]
    fn binary_not_found_has_hint() {
        assert!(GripError::BinaryNotFound("jq".into()).hint().is_some());
    }

    #[test]
    fn unsupported_platform_has_hint() {
        assert!(GripError::UnsupportedPlatform {
            adapter: "apt".into()
        }
        .hint()
        .is_some());
    }

    #[test]
    fn github_api_error_has_hint() {
        assert!(GripError::GitHubApi("404".into()).hint().is_some());
    }

    #[test]
    fn insufficient_privileges_has_hint() {
        assert!(GripError::InsufficientPrivileges {
            hint: "run as root".into()
        }
        .hint()
        .is_some());
    }

    #[test]
    fn command_failed_has_hint() {
        assert!(GripError::CommandFailed("exit 1".into()).hint().is_some());
    }

    #[test]
    fn shell_not_allowed_has_hint() {
        assert!(GripError::ShellNotAllowed {
            name: "mytool".into()
        }
        .hint()
        .is_some());
    }

    #[test]
    fn shell_not_allowed_message_contains_name() {
        let err = GripError::ShellNotAllowed {
            name: "mytool".into(),
        };
        assert!(err.to_string().contains("mytool"));
    }

    #[test]
    fn gpg_not_found_has_hint() {
        assert!(GripError::GpgNotFound.hint().is_some());
    }

    #[test]
    fn gpg_verification_failed_has_hint() {
        assert!(GripError::GpgVerificationFailed {
            name: "jq".into(),
            detail: "bad sig".into()
        }
        .hint()
        .is_some());
    }

    #[test]
    fn gpg_verification_failed_message_contains_name_and_detail() {
        let err = GripError::GpgVerificationFailed {
            name: "jq".into(),
            detail: "fingerprint mismatch".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("jq"));
        assert!(msg.contains("fingerprint mismatch"));
    }

    #[test]
    fn unpinned_entries_has_hint() {
        assert!(GripError::UnpinnedEntries {
            names: "jq, rg".into()
        }
        .hint()
        .is_some());
    }

    #[test]
    fn unpinned_entries_message_contains_names() {
        let err = GripError::UnpinnedEntries {
            names: "jq, rg".into(),
        };
        assert!(err.to_string().contains("jq"));
        assert!(err.to_string().contains("rg"));
    }

    #[test]
    fn other_repo_required_has_hint() {
        let err = GripError::Other("--repo required for GitHub".into());
        assert!(err.hint().is_some());
    }

    #[test]
    fn other_url_required_has_hint() {
        let err = GripError::Other("--url required for url source".into());
        assert!(err.hint().is_some());
    }

    #[test]
    fn other_not_found_in_grip_toml_has_hint() {
        let err = GripError::Other("jq not found in grip.toml".into());
        assert!(err.hint().is_some());
    }

    // ── format_user_message ───────────────────────────────────────────────────

    #[test]
    fn manifest_not_found_message() {
        let msg = GripError::ManifestNotFound.format_user_message(false);
        assert!(msg.contains("grip.toml"));
    }

    #[test]
    fn other_error_message() {
        let msg = GripError::Other("something went wrong".into()).format_user_message(false);
        assert_eq!(msg, "something went wrong");
    }

    #[test]
    fn checksum_mismatch_message_contains_values() {
        let err = GripError::ChecksumMismatch {
            expected: "aaa".into(),
            got: "bbb".into(),
        };
        let msg = err.format_user_message(false);
        assert!(msg.contains("aaa") && msg.contains("bbb"));
    }
}
