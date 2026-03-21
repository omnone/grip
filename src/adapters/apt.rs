//! Adapter that installs binaries via APT (`apt-get install`).

use async_trait::async_trait;
use indicatif::ProgressBar;
use reqwest::Client;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::adapters::SourceAdapter;
use crate::bin_dir::{sha256_of_installed, symlink_binary};
use crate::config::lockfile::LockEntry;
use crate::config::manifest::{BinaryEntry, LibAptEntry};
use crate::error::GripError;
use crate::output;
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
        colored: bool,
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

        let cmd_name = a.binary.as_deref().unwrap_or(name);

        // If already on PATH, skip installation and just symlink
        let which_pre = Command::new("which").arg(cmd_name).output()?;
        if !which_pre.status.success() {
            pb.set_message(format!("{name}  updating package index..."));
            // `-y` + noninteractive frontend avoid blocking prompts; stderr must be visible if
            // something still asks (e.g. conffile) or errors.
            let updated = Command::new("apt-get")
                .env("DEBIAN_FRONTEND", "noninteractive")
                .args(["-y", "update"])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !updated {
                Command::new("sudo")
                    .args([
                        "env",
                        "DEBIAN_FRONTEND=noninteractive",
                        "apt-get",
                        "-y",
                        "update",
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
                    .status()
                    .ok();
            }

            pb.set_message(format!("{name}  installing via apt..."));
            let status = Command::new("apt-get")
                .env("DEBIAN_FRONTEND", "noninteractive")
                .args(["install", "-y", &pkg])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status();

            let ok = match status {
                Ok(s) if s.success() => true,
                _ => Command::new("sudo")
                    .args([
                        "env",
                        "DEBIAN_FRONTEND=noninteractive",
                        "apt-get",
                        "install",
                        "-y",
                        &pkg,
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
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

        let which = Command::new("which").arg(cmd_name).output()?;
        if !which.status.success() {
            return Err(GripError::CommandFailed(format!(
                "installed package `{}` but `{cmd_name}` is not on PATH; \
                 set `binary = \"...\"` in grip.toml if the executable uses another name",
                a.package
            )));
        }
        let path_str = String::from_utf8_lossy(&which.stdout).trim().to_string();
        let target = std::path::PathBuf::from(&path_str);
        symlink_binary(&target, bin_dir, name)?;

        let version = installed_version(&a.package).unwrap_or_else(|| "unknown".to_string());
        pb.finish_with_message(format!(
            "{} {name}  {version}",
            output::success_checkmark(colored)
        ));
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

/// Install a library package via apt-get without symlinking any binary.
pub async fn install_apt_library(
    name: &str,
    entry: &LibAptEntry,
    pb: ProgressBar,
    colored: bool,
) -> Result<LockEntry, GripError> {
    let pkg = if let Some(v) = &entry.version {
        format!("{}={}", entry.package, v)
    } else {
        entry.package.clone()
    };
    let pkg = pkg.trim_end_matches('=').to_string();

    // Check if already installed via dpkg
    let already_installed = Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", &entry.package])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("install ok installed"))
        .unwrap_or(false);

    if !already_installed {
        pb.set_message(format!("{name}  updating package index..."));
        let updated = Command::new("apt-get")
            .env("DEBIAN_FRONTEND", "noninteractive")
            .args(["-y", "update"])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !updated {
            Command::new("sudo")
                .args(["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "-y", "update"])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .ok();
        }

        pb.set_message(format!("{name}  installing via apt..."));
        let ok = Command::new("apt-get")
            .env("DEBIAN_FRONTEND", "noninteractive")
            .args(["install", "-y", &pkg])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let ok2 = Command::new("sudo")
                .args([
                    "env",
                    "DEBIAN_FRONTEND=noninteractive",
                    "apt-get",
                    "install",
                    "-y",
                    &pkg,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok2 {
                return Err(GripError::CommandFailed(format!(
                    "apt-get install {} (try running: sudo apt-get install -y {})",
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
        source: "apt".to_string(),
        url: None,
        sha256: None,
        installed_at: chrono::Utc::now(),
    })
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
