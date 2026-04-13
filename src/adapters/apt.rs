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
use crate::privilege::{check_privileges, PrivilegeMode};

/// Installs packages with `apt-get install` and symlinks the binary into `.bin/`.
/// Only supported on Linux. Privilege check is performed upfront — no silent sudo retry.
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

        // If already on PATH, skip installation and just symlink.
        let which_pre = Command::new("which").arg(cmd_name).output()?;
        if !which_pre.status.success() {
            let priv_mode = check_privileges()?;

            pb.set_message(format!("{name}  updating package index..."));
            apt_get(priv_mode, &["-y", "update"])?;

            pb.set_message(format!("{name}  installing via apt..."));
            let ok = apt_get(priv_mode, &["install", "-y", &pkg])
                .map(|s| s.success())
                .unwrap_or(false);

            if !ok {
                return Err(GripError::CommandFailed(format!(
                    "apt-get install {pkg}"
                )));
            }
        }

        let which_post = Command::new("which").arg(cmd_name).output()?;
        let (target, auto_detected) = if which_post.status.success() {
            let path_str = String::from_utf8_lossy(&which_post.stdout).trim().to_string();
            (std::path::PathBuf::from(path_str), None)
        } else if a.binary.is_none() {
            // No explicit binary override — try to discover the executable from
            // the package file list so the user doesn't have to set it manually.
            let candidates = detect_package_executables(&a.package);
            match candidates.as_slice() {
                [single] => {
                    let which_cand = Command::new("which").arg(single).output()?;
                    if which_cand.status.success() {
                        let path_str =
                            String::from_utf8_lossy(&which_cand.stdout).trim().to_string();
                        (std::path::PathBuf::from(path_str), Some(single.clone()))
                    } else {
                        return Err(GripError::CommandFailed(format!(
                            "installed package `{}` but auto-detected binary `{single}` \
                             is not on PATH",
                            a.package
                        )));
                    }
                }
                [] => {
                    return Err(GripError::CommandFailed(format!(
                        "installed package `{}` but `{cmd_name}` is not on PATH; \
                         set `binary = \"...\"` in grip.toml if the executable uses another name",
                        a.package
                    )));
                }
                many => {
                    let list = many.join(", ");
                    return Err(GripError::CommandFailed(format!(
                        "installed package `{}` but `{cmd_name}` is not on PATH; \
                         multiple executables found: {list}; \
                         set `binary = \"...\"` in grip.toml to pick one",
                        a.package
                    )));
                }
            }
        } else {
            return Err(GripError::CommandFailed(format!(
                "installed package `{}` but `{cmd_name}` is not on PATH; \
                 set `binary = \"...\"` in grip.toml if the executable uses another name",
                a.package
            )));
        };
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
            auto_binary: auto_detected,
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

    // Check if already installed via dpkg.
    let already_installed = Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", &entry.package])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("install ok installed"))
        .unwrap_or(false);

    if !already_installed {
        let priv_mode = check_privileges()?;

        pb.set_message(format!("{name}  updating package index..."));
        apt_get(priv_mode, &["-y", "update"])?;

        pb.set_message(format!("{name}  installing via apt..."));
        let ok = apt_get(priv_mode, &["install", "-y", &pkg])
            .map(|s| s.success())
            .unwrap_or(false);

        if !ok {
            return Err(GripError::CommandFailed(format!(
                "apt-get install {pkg}"
            )));
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
        auto_binary: None,
    })
}

/// List executables installed by a package that live in a standard binary directory.
/// Uses `dpkg -L` to query the package file list.
fn detect_package_executables(package: &str) -> Vec<String> {
    let Ok(out) = Command::new("dpkg").args(["-L", package]).output() else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    const BIN_DIRS: &[&str] = &["/usr/bin/", "/usr/sbin/", "/bin/", "/sbin/"];
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| BIN_DIRS.iter().any(|d| line.starts_with(d)))
        .filter_map(|line| {
            std::path::Path::new(line)
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
        .collect()
}

/// Run `apt-get` with the given args, using sudo if required by `priv_mode`.
/// Returns the exit status of the command.
fn apt_get(
    priv_mode: PrivilegeMode,
    args: &[&str],
) -> Result<std::process::ExitStatus, GripError> {
    let status = match priv_mode {
        PrivilegeMode::Root => Command::new("apt-get")
            .env("DEBIAN_FRONTEND", "noninteractive")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()?,
        PrivilegeMode::Sudo => Command::new("sudo")
            .args(["env", "DEBIAN_FRONTEND=noninteractive", "apt-get"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()?,
    };
    Ok(status)
}

/// Query the actual installed version via dpkg-query.
pub fn installed_version(package: &str) -> Option<String> {
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
