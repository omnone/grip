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

    async fn resolve_latest(
        &self,
        entry: &BinaryEntry,
        _client: &Client,
    ) -> Result<String, GripError> {
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

        if !s.allow_shell {
            pb.finish_and_clear();
            return Err(GripError::ShellNotAllowed {
                name: name.to_string(),
            });
        }

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
            sha256: crate::bin_dir::sha256_of_installed(bin_dir, name),
            installed_at: chrono::Utc::now(),
            extra_binaries: s.extra_binaries.clone().unwrap_or_default(),
            auto_binary: None,
            auto_extra_binaries: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{BinaryEntry, CommonMeta, ShellEntry};
    use indicatif::ProgressBar;
    use tempfile::TempDir;

    fn shell_entry(cmd: &str, version: Option<&str>) -> BinaryEntry {
        shell_entry_with_allow(cmd, version, true)
    }

    fn shell_entry_with_allow(cmd: &str, version: Option<&str>, allow_shell: bool) -> BinaryEntry {
        BinaryEntry::Shell(ShellEntry {
            install_cmd: cmd.to_string(),
            version: version.map(String::from),
            allow_shell,
            extra_binaries: None,
            meta: CommonMeta::default(),
        })
    }

    // ── resolve_latest ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_latest_returns_pinned_version() {
        let entry = shell_entry("true", Some("2.5.0"));
        let client = reqwest::Client::new();
        let v = ShellAdapter.resolve_latest(&entry, &client).await.unwrap();
        assert_eq!(v, "2.5.0");
    }

    #[tokio::test]
    async fn resolve_latest_returns_custom_when_version_unset() {
        let entry = shell_entry("true", None);
        let client = reqwest::Client::new();
        let v = ShellAdapter.resolve_latest(&entry, &client).await.unwrap();
        assert_eq!(v, "custom");
    }

    // ── install: sha256 capture ───────────────────────────────────────────────

    #[tokio::test]
    async fn install_records_sha256_when_binary_placed_in_bin_dir() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        // Write a known byte sequence so we can assert an exact hash.
        // SHA-256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let entry = shell_entry(
            "printf 'hello' > \"$GRIP_BIN_DIR/mytool\" && chmod +x \"$GRIP_BIN_DIR/mytool\"",
            Some("1.0.0"),
        );

        let client = reqwest::Client::new();
        let result = ShellAdapter
            .install(
                "mytool",
                &entry,
                &bin_dir,
                &client,
                ProgressBar::hidden(),
                false,
            )
            .await
            .unwrap();

        assert_eq!(result.source, "shell");
        assert_eq!(result.version, "1.0.0");
        assert!(
            result.sha256.is_some(),
            "sha256 must be set when binary exists in .bin/"
        );
        assert_eq!(
            result.sha256.as_deref().unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        );
    }

    #[tokio::test]
    async fn install_sha256_is_none_when_script_places_no_binary() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        // Script succeeds but writes nothing to $GRIP_BIN_DIR.
        let entry = shell_entry("true", None);

        let client = reqwest::Client::new();
        let result = ShellAdapter
            .install(
                "mytool",
                &entry,
                &bin_dir,
                &client,
                ProgressBar::hidden(),
                false,
            )
            .await
            .unwrap();

        assert!(
            result.sha256.is_none(),
            "sha256 must be None when no file is placed in .bin/"
        );
        assert_eq!(result.version, "custom");
    }

    // ── install: allow_shell guard ────────────────────────────────────────────

    #[tokio::test]
    async fn install_blocked_when_allow_shell_false() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        // allow_shell = false must block execution even for a harmless command.
        let entry = shell_entry_with_allow("true", None, false);
        let client = reqwest::Client::new();
        let result = ShellAdapter
            .install(
                "mytool",
                &entry,
                &bin_dir,
                &client,
                ProgressBar::hidden(),
                false,
            )
            .await;

        assert!(
            matches!(result, Err(crate::error::GripError::ShellNotAllowed { .. })),
            "install must return ShellNotAllowed when allow_shell = false"
        );
    }

    // ── install: failure handling ─────────────────────────────────────────────

    #[tokio::test]
    async fn install_returns_error_on_nonzero_exit() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = tmp.path().join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        let entry = shell_entry("exit 1", None);
        let client = reqwest::Client::new();
        let result = ShellAdapter
            .install(
                "mytool",
                &entry,
                &bin_dir,
                &client,
                ProgressBar::hidden(),
                false,
            )
            .await;

        assert!(
            result.is_err(),
            "install must fail when the script exits non-zero"
        );
    }
}
