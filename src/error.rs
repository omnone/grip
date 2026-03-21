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
