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
use crate::privilege::{check_privileges, PrivilegeMode};

/// Installs packages with `dnf install` and symlinks the binary into `.bin/`.
/// Only supported on Linux where `dnf` is on PATH. Privilege check is performed
/// upfront — no silent sudo retry.
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

        let pkg = if let Some(v) = &d.version {
            format!("{}-{}", d.package, v)
        } else {
            d.package.clone()
        };
        let pkg = pkg.trim_end_matches('-').to_string();

        let cmd_name = d.binary.as_deref().unwrap_or(name);

        // If already on PATH, skip installation and just symlink.
        if find_in_path(cmd_name).is_none() {
            let priv_mode = check_privileges()?;

            pb.set_message(format!("{name}  refreshing package metadata..."));
            dnf(priv_mode, &["makecache", "-y"])?;

            pb.set_message(format!("{name}  installing via dnf..."));
            let ok = dnf(priv_mode, &["install", "-y", &pkg])
                .map(|s| s.success())
                .unwrap_or(false);

            if !ok {
                return Err(GripError::CommandFailed(format!("dnf install {pkg}")));
            }
        }

        let (target, auto_detected) = if let Some(p) = find_in_path(cmd_name) {
            (p, None)
        } else if d.binary.is_none() {
            // No explicit binary override set — try to discover the executable from
            // the package file list so the user doesn't have to set it manually.
            let candidates = detect_package_executables(&d.package);
            match candidates.as_slice() {
                [single] => {
                    let p = find_in_path(single).ok_or_else(|| {
                        GripError::CommandFailed(format!(
                            "installed package `{}` but auto-detected binary `{single}` \
                             is not on PATH",
                            d.package
                        ))
                    })?;
                    (p, Some(single.clone()))
                }
                [] => {
                    return Err(GripError::CommandFailed(format!(
                        "installed package `{}` but `{cmd_name}` is not on PATH; \
                         set `binary = \"...\"` in grip.toml if the executable uses another name",
                        d.package
                    )));
                }
                many => {
                    let list = many.join(", ");
                    return Err(GripError::CommandFailed(format!(
                        "installed package `{}` but `{cmd_name}` is not on PATH; \
                         multiple executables found: {list}; \
                         set `binary = \"...\"` in grip.toml to pick one",
                        d.package
                    )));
                }
            }
        } else {
            return Err(GripError::CommandFailed(format!(
                "installed package `{}` but `{cmd_name}` is not on PATH; \
                 set `binary = \"...\"` in grip.toml if the executable uses another name",
                d.package
            )));
        };
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
            auto_binary: auto_detected,
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

    // Check if already installed via rpm.
    let already_installed = Command::new("rpm")
        .args(["-q", &entry.package])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !already_installed {
        let priv_mode = check_privileges()?;

        pb.set_message(format!("{name}  refreshing package metadata..."));
        dnf(priv_mode, &["makecache", "-y"])?;

        pb.set_message(format!("{name}  installing via dnf..."));
        let ok = dnf(priv_mode, &["install", "-y", &pkg])
            .map(|s| s.success())
            .unwrap_or(false);

        if !ok {
            return Err(GripError::CommandFailed(format!("dnf install {pkg}")));
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
        auto_binary: None,
    })
}

/// Run `dnf` with the given args, using sudo if required by `priv_mode`.
fn dnf(
    priv_mode: PrivilegeMode,
    args: &[&str],
) -> Result<std::process::ExitStatus, GripError> {
    let status = match priv_mode {
        PrivilegeMode::Root => Command::new("dnf")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()?,
        PrivilegeMode::Sudo => Command::new("sudo")
            .arg("dnf")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()?,
    };
    Ok(status)
}

/// Returns `true` if `cmd` is found somewhere on PATH.
fn which_exists(cmd: &str) -> bool {
    find_in_path(cmd).is_some()
}

/// Searches PATH directories for `cmd` and returns the full path if found.
fn find_in_path(cmd: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(cmd))
            .find(|p| p.is_file())
    })
}

/// List executables installed by a package that live in a standard binary directory.
/// Uses `rpm -ql` to query the package file list.
fn detect_package_executables(package: &str) -> Vec<String> {
    let Ok(out) = Command::new("rpm").args(["-ql", package]).output() else {
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

/// Query the actual installed version via rpm.
pub fn installed_version(package: &str) -> Option<String> {
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
