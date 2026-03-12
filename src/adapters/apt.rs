//! Adapter that installs binaries via APT (`apt-get install`).

use async_trait::async_trait;
use indicatif::ProgressBar;
use reqwest::Client;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::adapters::SourceAdapter;
use crate::bin_dir::{sha256_of_installed, symlink_binary};
use crate::config::lockfile::LockEntry;
use crate::config::manifest::BinaryEntry;
use crate::error::GripError;
use crate::platform::Platform;

/// Installs packages with `apt-get install` (falling back to `sudo apt-get install`) and
/// symlinks the binary into `.bin/`. Only supported on Linux.
pub struct AptAdapter {
    pub platform: Platform,
}

#[async_trait]
impl SourceAdapter for AptAdapter {
    fn name(&self) -> &str {
        "apt"
    }

    fn is_supported(&self) -> bool {
        self.platform.is_linux()
    }

    async fn resolve_latest(&self, entry: &BinaryEntry, _client: &Client) -> Result<String, GripError> {
        let BinaryEntry::Apt(a) = entry else {
            return Err(GripError::Other("expected apt entry".into()));
        };
        Ok(a.version.clone().unwrap_or_else(|| "latest".to_string()))
    }

    async fn install(
        &self,
        name: &str,
        entry: &BinaryEntry,
        bin_dir: &Path,
        _client: &Client,
        pb: ProgressBar,
    ) -> Result<LockEntry, GripError> {
        if !self.is_supported() {
            return Err(GripError::UnsupportedPlatform {
                adapter: "apt".to_string(),
            });
        }

        let BinaryEntry::Apt(a) = entry else {
            return Err(GripError::Other("expected apt entry".into()));
        };

        let pkg = if let Some(v) = &a.version {
            format!("{}={}", a.package, v)
        } else {
            a.package.clone()
        };
        let pkg = pkg.trim_end_matches('=').to_string();

        // If already on PATH, skip installation and just symlink
        let which_pre = Command::new("which").arg(name).output()?;
        if !which_pre.status.success() {
            pb.set_message(format!("{name}  updating package index..."));
            let updated = Command::new("apt-get")
                .args(["update"])
                .stdout(Stdio::null()).stderr(Stdio::null())
                .status().map(|s| s.success()).unwrap_or(false);
            if !updated {
                Command::new("sudo").args(["apt-get", "update"])
                    .stdout(Stdio::null()).stderr(Stdio::null())
                    .status().ok();
            }

            pb.set_message(format!("{name}  installing via apt..."));
            let status = Command::new("apt-get")
                .args(["install", "-y", &pkg])
                .stdout(Stdio::null()).stderr(Stdio::null())
                .status();

            let ok = match status {
                Ok(s) if s.success() => true,
                _ => Command::new("sudo")
                    .args(["apt-get", "install", "-y", &pkg])
                    .stdout(Stdio::null()).stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false),
            };

            if !ok {
                return Err(GripError::CommandFailed(format!(
                    "apt-get install {} (try running: sudo apt-get install -y {})",
                    pkg, pkg
                )));
            }
        }

        let which = Command::new("which").arg(name).output()?;
        if which.status.success() {
            let path_str = String::from_utf8_lossy(&which.stdout).trim().to_string();
            let target = std::path::PathBuf::from(&path_str);
            symlink_binary(&target, bin_dir, name)?;
        }

        let version = installed_version(&a.package).unwrap_or_else(|| "unknown".to_string());
        pb.finish_with_message(format!("\x1b[32m✓\x1b[0m {name}  {version}"));
        Ok(LockEntry {
            name: name.to_string(),
            version,
            source: "apt".to_string(),
            url: None,
            sha256: sha256_of_installed(bin_dir, name),
            installed_at: chrono::Utc::now(),
        })
    }
}

/// Query the actual installed version via dpkg-query.
fn installed_version(package: &str) -> Option<String> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f=${Version}", package])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}
