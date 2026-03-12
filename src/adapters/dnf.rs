//! Adapter that installs binaries via DNF (`dnf install`).

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

/// Installs packages with `dnf install` (falling back to `sudo dnf install`) and symlinks the
/// binary into `.bin/`. Only supported on Linux where `dnf` is on PATH.
pub struct DnfAdapter {
    pub platform: Platform,
}

#[async_trait]
impl SourceAdapter for DnfAdapter {
    fn name(&self) -> &str {
        "dnf"
    }

    fn is_supported(&self) -> bool {
        self.platform.is_linux() && which_exists("dnf")
    }

    async fn resolve_latest(&self, entry: &BinaryEntry, _client: &Client) -> Result<String, GripError> {
        let BinaryEntry::Dnf(d) = entry else {
            return Err(GripError::Other("expected dnf entry".into()));
        };
        Ok(d.version.clone().unwrap_or_else(|| "latest".to_string()))
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
                adapter: "dnf".to_string(),
            });
        }

        let BinaryEntry::Dnf(d) = entry else {
            return Err(GripError::Other("expected dnf entry".into()));
        };

        // Use version-pinned package spec when a version is set (e.g. from --locked).
        let pkg = if let Some(v) = &d.version {
            format!("{}-{}", d.package, v)
        } else {
            d.package.clone()
        };
        let pkg = pkg.trim_end_matches('-').to_string();

        // If already on PATH, skip installation and just symlink
        let which_pre = Command::new("which").arg(name).output()?;
        if !which_pre.status.success() {
            pb.set_message(format!("{name}  refreshing package metadata..."));
            let updated = Command::new("dnf")
                .args(["makecache"])
                .stdout(Stdio::null()).stderr(Stdio::null())
                .status().map(|s| s.success()).unwrap_or(false);
            if !updated {
                Command::new("sudo").args(["dnf", "makecache"])
                    .stdout(Stdio::null()).stderr(Stdio::null())
                    .status().ok();
            }

            pb.set_message(format!("{name}  installing via dnf..."));
            let status = Command::new("dnf")
                .args(["install", "-y", &pkg])
                .stdout(Stdio::null()).stderr(Stdio::null())
                .status();

            let ok = match status {
                Ok(s) if s.success() => true,
                _ => Command::new("sudo")
                    .args(["dnf", "install", "-y", &pkg])
                    .stdout(Stdio::null()).stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false),
            };

            if !ok {
                return Err(GripError::CommandFailed(format!(
                    "dnf install {} (try running: sudo dnf install -y {})",
                    pkg, pkg
                )));
            }
        }

        // Symlink the installed binary into .bin/
        let which = Command::new("which").arg(name).output()?;
        if which.status.success() {
            let path_str = String::from_utf8_lossy(&which.stdout).trim().to_string();
            let target = std::path::PathBuf::from(&path_str);
            symlink_binary(&target, bin_dir, name)?;
        }

        let version = installed_version(&d.package).unwrap_or_else(|| "unknown".to_string());
        pb.finish_with_message(format!("\x1b[32m✓\x1b[0m {name}  {version}"));
        Ok(LockEntry {
            name: name.to_string(),
            version,
            source: "dnf".to_string(),
            url: None,
            sha256: sha256_of_installed(bin_dir, name),
            installed_at: chrono::Utc::now(),
        })
    }
}

/// Returns `true` if `cmd` is found on PATH via `which`.
fn which_exists(cmd: &str) -> bool {
    Command::new("which").arg(cmd).output().map(|o| o.status.success()).unwrap_or(false)
}

/// Query the actual installed version via rpm.
fn installed_version(package: &str) -> Option<String> {
    let out = Command::new("rpm")
        .args(["-q", "--queryformat", "%{VERSION}-%{RELEASE}", package])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}
