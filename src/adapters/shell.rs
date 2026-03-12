//! Adapter that installs binaries by running a user-supplied shell command.

use async_trait::async_trait;
use indicatif::ProgressBar;
use reqwest::Client;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::adapters::SourceAdapter;
use crate::config::lockfile::LockEntry;
use crate::config::manifest::BinaryEntry;
use crate::error::GripError;

/// Runs the `install_cmd` field from the manifest entry via `sh -c`.
/// The `GRIP_BIN_DIR` environment variable is set to the project's `.bin/` directory.
/// Supported on all platforms.
pub struct ShellAdapter;

#[async_trait]
impl SourceAdapter for ShellAdapter {
    fn name(&self) -> &str {
        "shell"
    }

    fn is_supported(&self) -> bool {
        true
    }

    async fn resolve_latest(&self, entry: &BinaryEntry, _client: &Client) -> Result<String, GripError> {
        let BinaryEntry::Shell(s) = entry else {
            return Err(GripError::Other("expected shell entry".into()));
        };
        Ok(s.version.clone().unwrap_or_else(|| "custom".to_string()))
    }

    async fn install(
        &self,
        name: &str,
        entry: &BinaryEntry,
        bin_dir: &Path,
        _client: &Client,
        pb: ProgressBar,
    ) -> Result<LockEntry, GripError> {
        let BinaryEntry::Shell(s) = entry else {
            return Err(GripError::Other("expected shell entry".into()));
        };

        pb.set_message(format!("{name}  running install script..."));
        let status = Command::new("sh")
            .args(["-c", &s.install_cmd])
            .env("GRIP_BIN_DIR", bin_dir)
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status()?;

        if !status.success() {
            return Err(GripError::CommandFailed(s.install_cmd.clone()));
        }

        let version = s.version.clone().unwrap_or_else(|| "custom".to_string());
        pb.finish_with_message(format!("\x1b[32m✓\x1b[0m {name}  {version}"));
        Ok(LockEntry {
            name: name.to_string(),
            version,
            source: "shell".to_string(),
            url: None,
            sha256: None,
            installed_at: chrono::Utc::now(),
        })
    }
}
