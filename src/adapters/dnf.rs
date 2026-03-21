//! Adapter that installs binaries via DNF (`dnf install`).

use async_trait::async_trait;
use indicatif::ProgressBar;
use reqwest::Client;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::adapters::SourceAdapter;
use crate::bin_dir::{sha256_of_installed, symlink_binary};
use crate::config::lockfile::LockEntry;
use crate::config::manifest::{BinaryEntry, LibDnfEntry};
use crate::error::GripError;
use crate::output;
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
        colored: bool,
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

        let cmd_name = d.binary.as_deref().unwrap_or(name);

        // If already on PATH, skip installation and just symlink
        let which_pre = Command::new("which").arg(cmd_name).output()?;
        if !which_pre.status.success() {
            pb.set_message(format!("{name}  refreshing package metadata..."));
            // `-y` avoids interactive GPG/repo prompts (e.g. new signing keys); stderr must not be
            // discarded or those prompts block on stdin with no visible text.
            let updated = Command::new("dnf")
                .args(["makecache", "-y"])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !updated {
                Command::new("sudo")
                    .args(["dnf", "makecache", "-y"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
                    .status()
                    .ok();
            }

            pb.set_message(format!("{name}  installing via dnf..."));
            let status = Command::new("dnf")
                .args(["install", "-y", &pkg])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status();

            let ok = match status {
                Ok(s) if s.success() => true,
                _ => Command::new("sudo")
                    .args(["dnf", "install", "-y", &pkg])
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
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

        // Symlink the installed binary into .bin/ (manifest `name` → actual command `cmd_name`)
        let which = Command::new("which").arg(cmd_name).output()?;
        if !which.status.success() {
            return Err(GripError::CommandFailed(format!(
                "installed package `{}` but `{cmd_name}` is not on PATH; \
                 set `binary = \"...\"` in grip.toml if the executable uses another name",
                d.package
            )));
        }
        let path_str = String::from_utf8_lossy(&which.stdout).trim().to_string();
        let target = std::path::PathBuf::from(&path_str);
        symlink_binary(&target, bin_dir, name)?;

        let version = installed_version(&d.package).unwrap_or_else(|| "unknown".to_string());
        pb.finish_with_message(format!(
            "{} {name}  {version}",
            output::success_checkmark(colored)
        ));
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

/// Install a library package via dnf without symlinking any binary.
pub async fn install_dnf_library(
    name: &str,
    entry: &LibDnfEntry,
    pb: ProgressBar,
    colored: bool,
) -> Result<LockEntry, GripError> {
    let pkg = if let Some(v) = &entry.version {
        format!("{}-{}", entry.package, v)
    } else {
        entry.package.clone()
    };
    let pkg = pkg.trim_end_matches('-').to_string();

    // Check if already installed via rpm
    let already_installed = Command::new("rpm")
        .args(["-q", &entry.package])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !already_installed {
        pb.set_message(format!("{name}  refreshing package metadata..."));
        let updated = Command::new("dnf")
            .args(["makecache", "-y"])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !updated {
            Command::new("sudo")
                .args(["dnf", "makecache", "-y"])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .ok();
        }

        pb.set_message(format!("{name}  installing via dnf..."));
        let ok = Command::new("dnf")
            .args(["install", "-y", &pkg])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let ok2 = Command::new("sudo")
                .args(["dnf", "install", "-y", &pkg])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok2 {
                return Err(GripError::CommandFailed(format!(
                    "dnf install {} (try running: sudo dnf install -y {})",
                    pkg, pkg
                )));
            }
        }
    }

    let version = installed_version(&entry.package).unwrap_or_else(|| "unknown".to_string());
    pb.finish_with_message(format!(
        "{} {name}  {version}",
        output::success_checkmark(colored)
    ));
    Ok(LockEntry {
        name: name.to_string(),
        version,
        source: "dnf".to_string(),
        url: None,
        sha256: None,
        installed_at: chrono::Utc::now(),
    })
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
