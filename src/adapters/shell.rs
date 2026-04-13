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
use crate::output;

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
        colored: bool,
    ) -> Result<LockEntry, GripError> {
        let BinaryEntry::Shell(s) = entry else {
            return Err(GripError::Other("expected shell entry".into()));
        };

        pb.set_message(format!("{name}  running install script..."));
        let status = Command::new("sh")
            .args(["-c", &s.install_cmd])
            .env("GRIP_BIN_DIR", bin_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()?;

        if !status.success() {
            return Err(GripError::CommandFailed(s.install_cmd.clone()));
        }

        let version = s.version.clone().unwrap_or_else(|| "custom".to_string());
        pb.finish_with_message(format!(
            "{} {name}  {version}",
            output::success_checkmark(colored)
        ));
        Ok(LockEntry {
            name: name.to_string(),
            version,
            source: "shell".to_string(),
            url: None,
            sha256: None,
            installed_at: chrono::Utc::now(),
            auto_binary: None,
        })
    }
}
