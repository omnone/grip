//! Adapter that installs binaries via DNF (`dnf install`).

use async_trait::async_trait;
use indicatif::ProgressBar;
use reqwest::Client;
use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use crate::adapters::SourceAdapter;
use crate::bin_dir::{sha256_of_installed, symlink_binary};
use crate::checksum::sha256_file;
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

    async fn resolve_latest(
        &self,
        entry: &BinaryEntry,
        _client: &Client,
    ) -> Result<String, GripError> {
        let BinaryEntry::Dnf(d) = entry else {
            return Err(GripError::Other("expected dnf entry".into()));
        };
        if let Some(v) = &d.version {
            return Ok(v.clone());
        }
        if let Some(v) = dnf_latest_version(&d.package) {
            return Ok(v);
        }
        Ok("latest".to_string())
    }

    async fn install(
        &self,
        name: &str,
        entry: &BinaryEntry,
        bin_dir: &Path,
        client: &Client,
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

            // Issue 1: install repo RPMs before the main package.
            if let Some(repos) = &d.dnf_repos {
                install_dnf_repos(name, repos, priv_mode, &pb)?;
            }

            // Issue 2: import GPG keys before installing.
            if let Some(gpg_keys) = &d.gpg_keys {
                for url in gpg_keys {
                    pb.set_message(format!("{name}  importing GPG key..."));
                    import_dnf_gpg_key(url, client, priv_mode).await?;
                }
            }

            pb.set_message(format!("{name}  refreshing package metadata..."));
            dnf(priv_mode, &["makecache", "-y"], &[])?;

            pb.set_message(format!("{name}  installing via dnf..."));
            let extra_flags: Vec<&str> = d
                .dnf_flags
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(String::as_str)
                .collect();
            let ok = dnf(priv_mode, &["install", "-y", &pkg], &extra_flags)
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
            // Issue 3: binary was explicitly declared but not found — fall back to
            // auto-detection instead of failing immediately.
            let candidates = detect_package_executables(&d.package);
            match candidates.as_slice() {
                [single] => match find_in_path(single) {
                    Some(p) => {
                        eprintln!(
                                "warn: binary `{cmd_name}` not found after installing `{}`; \
                                 auto-detected `{single}` — update grip.toml: binary = \"{single}\"",
                                d.package
                            );
                        (p, Some(single.clone()))
                    }
                    None => {
                        return Err(GripError::CommandFailed(format!(
                                "installed package `{}` but `{cmd_name}` is not on PATH; \
                                 set `binary = \"...\"` in grip.toml if the executable uses another name",
                                d.package
                            )));
                    }
                },
                [] => {
                    return Err(GripError::CommandFailed(format!(
                        "installed package `{}` but `{cmd_name}` is not on PATH; \
                         set `binary = \"...\"` in grip.toml if the executable uses another name",
                        d.package
                    )));
                }
                many => {
                    let first = &many[0];
                    let list = many.join(", ");
                    match find_in_path(first) {
                        Some(p) => {
                            eprintln!(
                                "warn: binary `{cmd_name}` not found after installing `{}`; \
                                 multiple candidates: {list}; using `{first}` — \
                                 update grip.toml: binary = \"{first}\"",
                                d.package
                            );
                            (p, Some(first.clone()))
                        }
                        None => {
                            return Err(GripError::CommandFailed(format!(
                                "installed package `{}` but `{cmd_name}` is not on PATH; \
                                 multiple executables found: {list}; \
                                 set `binary = \"...\"` in grip.toml to pick one",
                                d.package
                            )));
                        }
                    }
                }
            }
        };
        symlink_binary(&target, bin_dir, name)?;

        let primary_cmd = cmd_name;
        let mut extra_symlinked: Vec<String> = Vec::new();
        let mut auto_detected_extras: Vec<String> = Vec::new();

        if let Some(extras) = &d.extra_binaries {
            // Manually declared extras — symlink each.
            for extra in extras {
                if let Some(p) = find_in_path(extra) {
                    symlink_binary(&p, bin_dir, extra)?;
                    extra_symlinked.push(extra.clone());
                }
            }
        } else {
            // Auto-detect: symlink every executable the package installed
            // other than the primary binary.
            for exe in detect_package_executables(&d.package) {
                if exe == primary_cmd || exe == name {
                    continue;
                }
                if let Some(p) = find_in_path(&exe) {
                    symlink_binary(&p, bin_dir, &exe)?;
                    extra_symlinked.push(exe.clone());
                    auto_detected_extras.push(exe.clone());
                }
            }
        }

        let version = installed_version(&d.package)
            .or_else(|| version_from_path(&target))
            .unwrap_or_else(|| "unknown".to_string());
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
            extra_binaries: extra_symlinked,
            auto_binary: auto_detected,
            auto_extra_binaries: auto_detected_extras,
        })
    }
}

/// Install a library package via dnf without symlinking any binary.
pub async fn install_dnf_library(
    name: &str,
    entry: &LibDnfEntry,
    client: &Client,
    pb: ProgressBar,
    colored: bool,
) -> Result<LockEntry, GripError> {
    let pkg = if let Some(v) = &entry.version {
        format!("{}-{}", entry.package, v)
    } else {
        entry.package.clone()
    };
    let pkg = pkg.trim_end_matches('-').to_string();

    let already_installed = rpm_provides_installed(&entry.package);

    if !already_installed {
        let priv_mode = check_privileges()?;

        if let Some(repos) = &entry.dnf_repos {
            install_dnf_repos(name, repos, priv_mode, &pb)?;
        }

        if let Some(gpg_keys) = &entry.gpg_keys {
            for url in gpg_keys {
                pb.set_message(format!("{name}  importing GPG key..."));
                import_dnf_gpg_key(url, client, priv_mode).await?;
            }
        }

        pb.set_message(format!("{name}  refreshing package metadata..."));
        dnf(priv_mode, &["makecache", "-y"], &[])?;

        pb.set_message(format!("{name}  installing via dnf..."));
        let extra_flags: Vec<&str> = entry
            .dnf_flags
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        let ok = dnf(priv_mode, &["install", "-y", &pkg], &extra_flags)
            .map(|s| s.success())
            .unwrap_or(false);

        if !ok {
            return Err(GripError::CommandFailed(format!("dnf install {pkg}")));
        }
    }

    // Issue 9: verify installation and compute sha256 of installed library files.
    if !rpm_provides_installed(&entry.package) {
        return Err(GripError::CommandFailed(format!(
            "post-install check failed: package `{}` is not installed according to rpm",
            entry.package
        )));
    }

    let lib_sha256 = compute_library_sha256(&entry.package);

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
        sha256: lib_sha256,
        installed_at: chrono::Utc::now(),
        extra_binaries: vec![],
        auto_binary: None,
        auto_extra_binaries: vec![],
    })
}

fn install_dnf_repos(
    name: &str,
    repos: &[String],
    priv_mode: PrivilegeMode,
    pb: &ProgressBar,
) -> Result<(), GripError> {
    for repo_url in filter_new_dnf_repos(repos) {
        pb.set_message(format!("{name}  installing repo..."));
        let ok = dnf(priv_mode, &["install", "-y", &repo_url], &[])
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            forget_dnf_repo(&repo_url);
            return Err(GripError::CommandFailed(format!(
                "dnf install repo {repo_url}"
            )));
        }
    }
    Ok(())
}

fn filter_new_dnf_repos(repos: &[String]) -> Vec<String> {
    let mut new_repos = Vec::new();
    let seen_lock = installed_dnf_repos().lock();
    let Ok(mut seen) = seen_lock else {
        return repos
            .iter()
            .filter_map(|repo| normalize_dnf_repo_url(repo))
            .collect();
    };

    for repo in repos {
        let Some(normalized) = normalize_dnf_repo_url(repo) else {
            continue;
        };
        if seen.insert(normalized.clone()) {
            new_repos.push(normalized);
        }
    }

    new_repos
}

fn forget_dnf_repo(repo_url: &str) {
    if let Ok(mut seen) = installed_dnf_repos().lock() {
        if let Some(normalized) = normalize_dnf_repo_url(repo_url) {
            seen.remove(&normalized);
        }
    }
}

fn installed_dnf_repos() -> &'static Mutex<HashSet<String>> {
    static INSTALLED_DNF_REPOS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    INSTALLED_DNF_REPOS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn normalize_dnf_repo_url(repo_url: &str) -> Option<String> {
    let trimmed = repo_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Issue 2: Import a GPG key via `rpm --import <url>`.
async fn import_dnf_gpg_key(
    url: &str,
    _client: &Client,
    priv_mode: PrivilegeMode,
) -> Result<(), GripError> {
    let status = match priv_mode {
        PrivilegeMode::Root => Command::new("rpm")
            .args(["--import", url])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map_err(GripError::Io)?,
        PrivilegeMode::Sudo => Command::new("sudo")
            .args(["rpm", "--import", url])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map_err(GripError::Io)?,
    };
    if !status.success() {
        return Err(GripError::CommandFailed(format!("rpm --import {url}")));
    }
    Ok(())
}

/// Issue 9: Compute a combined SHA-256 over the shared-library files installed by a package.
/// Finds `.so` files via `rpm -ql`, sorts them, hashes each, then hashes all the digests
/// together. Returns `None` if no library files are found or hashing fails.
fn compute_library_sha256(package: &str) -> Option<String> {
    let out = Command::new("rpm").args(["-ql", package]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let lib_dirs = &["/usr/lib/", "/lib/", "/usr/lib64/", "/lib64/"];
    let mut so_files: Vec<std::path::PathBuf> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| lib_dirs.iter().any(|d| line.starts_with(d)) && line.contains(".so"))
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_file())
        .collect();
    if so_files.is_empty() {
        return None;
    }
    so_files.sort();

    use sha2::{Digest, Sha256};
    let mut combined = Sha256::new();
    for path in &so_files {
        match sha256_file(path) {
            Ok(file_hash) => combined.update(file_hash.as_bytes()),
            Err(_) => return None,
        }
    }
    Some(format!("{:x}", combined.finalize()))
}

/// Run `dnf` with the given args, using sudo if required by `priv_mode`.
/// `extra_flags` are appended after `args`.
fn dnf(
    priv_mode: PrivilegeMode,
    args: &[&str],
    extra_flags: &[&str],
) -> Result<std::process::ExitStatus, GripError> {
    let status = match priv_mode {
        PrivilegeMode::Root => Command::new("dnf")
            .args(args)
            .args(extra_flags)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()?,
        PrivilegeMode::Sudo => Command::new("sudo")
            .arg("dnf")
            .args(args)
            .args(extra_flags)
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

/// Query the latest available version for a package via `dnf info --quiet`.
/// Parses `Version` and `Release` fields and returns them joined as `version-release`.
/// Returns `None` if the command fails or the package is not known to DNF.
fn dnf_latest_version(package: &str) -> Option<String> {
    let out = Command::new("dnf")
        .args(["info", "--quiet", package])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_dnf_info_output(&String::from_utf8_lossy(&out.stdout))
}

/// Parse the text output of `dnf info --quiet` and return `version-release`.
/// Returns only the `Version` field when `Release` is absent, and `None` when
/// neither field is present.
pub(crate) fn parse_dnf_info_output(output: &str) -> Option<String> {
    let mut version: Option<&str> = None;
    let mut release: Option<&str> = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("Version") {
            if let Some(v) = rest
                .trim_start_matches(|c: char| c == ' ' || c == ':')
                .split_whitespace()
                .next()
            {
                version = Some(v);
            }
        } else if let Some(rest) = line.strip_prefix("Release") {
            if let Some(r) = rest
                .trim_start_matches(|c: char| c == ' ' || c == ':')
                .split_whitespace()
                .next()
            {
                release = Some(r);
            }
        }
        if version.is_some() && release.is_some() {
            break;
        }
    }
    match (version, release) {
        (Some(v), Some(r)) => Some(format!("{v}-{r}")),
        (Some(v), None) => Some(v.to_string()),
        _ => None,
    }
}

/// Query the installed version by looking up which RPM package owns `path`.
/// Used as a fallback when the manifest package name doesn't match the RPM name exactly.
fn version_from_path(path: &std::path::Path) -> Option<String> {
    let path_str = path.to_str()?;
    let out = Command::new("rpm")
        .args([
            "-q",
            "-f",
            path_str,
            "--queryformat",
            "%{VERSION}-%{RELEASE}",
        ])
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

fn rpm_provides_installed(package: &str) -> bool {
    Command::new("rpm")
        .args(["-q", "--whatprovides", package])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Query the actual installed version via rpm.
pub fn installed_version(package: &str) -> Option<String> {
    let out = Command::new("rpm")
        .args(["-q", "--queryformat", "%{VERSION}-%{RELEASE}", package])
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }

    let out = Command::new("rpm")
        .args([
            "-q",
            "--whatprovides",
            "--queryformat",
            "%{VERSION}-%{RELEASE}",
            package,
        ])
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{filter_new_dnf_repos, installed_dnf_repos, parse_dnf_info_output};

    const TYPICAL_OUTPUT: &str = "\
Last metadata expiration check: 0:01:23 ago on Mon Apr 13 10:00:00 2026.
Available Packages
Name         : ripgrep
Version      : 14.1.0
Release      : 2.fc41
Architecture : x86_64
Size         : 1.5 M
Source       : ripgrep-14.1.0-2.fc41.src.rpm
Repository   : fedora
Summary      : Line oriented search tool using Rust's regex library
URL          : https://github.com/BurntSushi/ripgrep
License      : Unlicense
Description  : ripgrep is a line-oriented search tool.";

    #[test]
    fn parses_version_and_release_from_typical_output() {
        assert_eq!(
            parse_dnf_info_output(TYPICAL_OUTPUT),
            Some("14.1.0-2.fc41".to_string())
        );
    }

    #[test]
    fn returns_version_only_when_release_absent() {
        let output = "Version      : 3.2.1\n";
        assert_eq!(parse_dnf_info_output(output), Some("3.2.1".to_string()));
    }

    #[test]
    fn returns_none_when_neither_field_present() {
        let output = "Name         : somepkg\nSummary      : A tool\n";
        assert!(parse_dnf_info_output(output).is_none());
    }

    #[test]
    fn returns_none_for_empty_output() {
        assert!(parse_dnf_info_output("").is_none());
    }

    #[test]
    fn stops_at_first_version_and_release_pair() {
        // Two stanzas — should return the first Version+Release encountered.
        let output = "\
Version      : 1.0.0
Release      : 1.fc40
Version      : 2.0.0
Release      : 1.fc40";
        assert_eq!(
            parse_dnf_info_output(output),
            Some("1.0.0-1.fc40".to_string())
        );
    }

    #[test]
    fn filters_dnf_repo_urls_already_seen_in_this_process() {
        installed_dnf_repos().lock().unwrap().clear();

        let repos = vec![
            " https://example.com/repo-release.rpm ".to_string(),
            "https://example.com/repo-release.rpm".to_string(),
            "".to_string(),
            "https://example.com/other-release.rpm".to_string(),
        ];

        assert_eq!(
            filter_new_dnf_repos(&repos),
            vec![
                "https://example.com/repo-release.rpm",
                "https://example.com/other-release.rpm",
            ]
        );
        assert!(filter_new_dnf_repos(&repos).is_empty());

        installed_dnf_repos().lock().unwrap().clear();
    }
}
