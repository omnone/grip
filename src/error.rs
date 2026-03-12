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
    /// No `binaries.toml` was found in the current directory or any parent.
    #[error("Manifest not found (no binaries.toml in current or parent dirs)")]
    ManifestNotFound,
    /// `--locked` mode detected that the lock file would change.
    #[error("Lock file changed in --locked mode")]
    LockChanged,
    /// A subprocess exited with a non-zero status.
    #[error("Command failed: {0}")]
    CommandFailed(String),
    /// The GitHub API returned an error or unexpected response.
    #[error("GitHub API error: {0}")]
    GitHubApi(String),
    /// No release asset matched the provided glob pattern.
    #[error("No matching asset found for pattern '{0}'")]
    NoMatchingAsset(String),
    /// The `source` field in `binaries.toml` names an unknown adapter.
    #[error("Unknown source adapter: {0}")]
    UnknownAdapter(String),
    /// A catch-all for errors that don't fit the variants above.
    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for GripError {
    fn from(e: anyhow::Error) -> Self {
        GripError::Other(e.to_string())
    }
}
