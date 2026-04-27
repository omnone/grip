//! Adapter that installs binaries via APT (`apt-get install`).

use async_trait::async_trait;
use indicatif::ProgressBar;
use reqwest::Client;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::adapters::SourceAdapter;
use crate::bin_dir::{sha256_of_installed, symlink_binary};
use crate::checksum::sha256_file;
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

    async fn resolve_latest(
        &self,
        entry: &BinaryEntry,
        _client: &Client,
    ) -> Result<String, GripError> {
        let BinaryEntry::Apt(a) = entry else {
            return Err(GripError::Other("expected apt entry".into()));
        };
        if let Some(v) = &a.version {
            return Ok(v.clone());
        }
        if let Some(v) = apt_cache_candidate(&a.package) {
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

            // Issue 1: add custom apt sources before updating index.
            if let Some(sources) = &a.apt_sources {
                pb.set_message(format!("{name}  adding apt sources..."));
                add_apt_sources(name, sources, priv_mode)?;
            }

            // Issue 2: import GPG keys before installing.
            if let Some(gpg_keys) = &a.gpg_keys {
                for url in gpg_keys {
                    pb.set_message(format!("{name}  importing GPG key..."));
                    import_apt_gpg_key(name, url, client, priv_mode).await?;
                }
            }

            // Issue 2: feed debconf selections before installing.
            if let Some(selections) = &a.debconf_selections {
                pb.set_message(format!("{name}  setting debconf selections..."));
                run_debconf_selections(selections, priv_mode)?;
            }

            pb.set_message(format!("{name}  updating package index..."));
            apt_get(priv_mode, &["-y", "update"], &[], &[])?;

            pb.set_message(format!("{name}  installing via apt..."));
            let extra_flags: Vec<&str> = a
                .apt_flags
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(String::as_str)
                .collect();
            let extra_env: Vec<(String, String)> = a
                .apt_env
                .as_ref()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();
            let ok = apt_get(priv_mode, &["install", "-y", &pkg], &extra_flags, &extra_env)
                .map(|s| s.success())
                .unwrap_or(false);

            if !ok {
                return Err(GripError::CommandFailed(format!("apt-get install {pkg}")));
            }
        }

        let which_post = Command::new("which").arg(cmd_name).output()?;
        let (target, auto_detected) = if which_post.status.success() {
            let path_str = String::from_utf8_lossy(&which_post.stdout)
                .trim()
                .to_string();
            (std::path::PathBuf::from(path_str), None)
        } else if a.binary.is_none() {
            // No explicit binary override — try to discover the executable from
            // the package file list so the user doesn't have to set it manually.
            let candidates = detect_package_executables(&a.package);
            match candidates.as_slice() {
                [single] => {
                    let which_cand = Command::new("which").arg(single).output()?;
                    if which_cand.status.success() {
                        let path_str = String::from_utf8_lossy(&which_cand.stdout)
                            .trim()
                            .to_string();
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
            // Issue 3: binary was explicitly declared but not found — fall back to
            // auto-detection instead of failing immediately.
            let candidates = detect_package_executables(&a.package);
            match candidates.as_slice() {
                [single] => {
                    let which_cand = Command::new("which").arg(single).output()?;
                    if which_cand.status.success() {
                        let path_str = String::from_utf8_lossy(&which_cand.stdout)
                            .trim()
                            .to_string();
                        eprintln!(
                            "warn: binary `{cmd_name}` not found after installing `{}`; \
                             auto-detected `{single}` — update grip.toml: binary = \"{single}\"",
                            a.package
                        );
                        (std::path::PathBuf::from(path_str), Some(single.clone()))
                    } else {
                        return Err(GripError::CommandFailed(format!(
                            "installed package `{}` but `{cmd_name}` is not on PATH; \
                             set `binary = \"...\"` in grip.toml if the executable uses another name",
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
                    let first = &many[0];
                    let list = many.join(", ");
                    let which_cand = Command::new("which").arg(first).output()?;
                    if which_cand.status.success() {
                        let path_str = String::from_utf8_lossy(&which_cand.stdout)
                            .trim()
                            .to_string();
                        eprintln!(
                            "warn: binary `{cmd_name}` not found after installing `{}`; \
                             multiple candidates: {list}; using `{first}` — \
                             update grip.toml: binary = \"{first}\"",
                            a.package
                        );
                        (std::path::PathBuf::from(path_str), Some(first.clone()))
                    } else {
                        return Err(GripError::CommandFailed(format!(
                            "installed package `{}` but `{cmd_name}` is not on PATH; \
                             multiple executables found: {list}; \
                             set `binary = \"...\"` in grip.toml to pick one",
                            a.package
                        )));
                    }
                }
            }
        };
        symlink_binary(&target, bin_dir, name)?;

        let primary_cmd = cmd_name;
        let mut extra_symlinked: Vec<String> = Vec::new();
        let mut auto_detected_extras: Vec<String> = Vec::new();

        if let Some(extras) = &a.extra_binaries {
            // Manually declared extras — symlink each.
            for extra in extras {
                let which_extra = Command::new("which").arg(extra).output()?;
                if which_extra.status.success() {
                    let path_str = String::from_utf8_lossy(&which_extra.stdout)
                        .trim()
                        .to_string();
                    symlink_binary(&std::path::PathBuf::from(path_str), bin_dir, extra)?;
                    extra_symlinked.push(extra.clone());
                }
            }
        } else {
            // Auto-detect: symlink every executable the package installed
            // other than the primary binary.
            for exe in detect_package_executables(&a.package) {
                if exe == primary_cmd || exe == name {
                    continue;
                }
                let which_extra = Command::new("which").arg(&exe).output()?;
                if which_extra.status.success() {
                    let path_str = String::from_utf8_lossy(&which_extra.stdout)
                        .trim()
                        .to_string();
                    symlink_binary(&std::path::PathBuf::from(path_str), bin_dir, &exe)?;
                    extra_symlinked.push(exe.clone());
                    auto_detected_extras.push(exe.clone());
                }
            }
        }

        let version = installed_version(&a.package)
            .or_else(|| version_from_path(&target))
            .unwrap_or_else(|| "unknown".to_string());
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
            extra_binaries: extra_symlinked,
            auto_binary: auto_detected,
            auto_extra_binaries: auto_detected_extras,
        })
    }
}

/// Install a library package via apt-get without symlinking any binary.
pub async fn install_apt_library(
    name: &str,
    entry: &LibAptEntry,
    client: &Client,
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

        if let Some(sources) = &entry.apt_sources {
            pb.set_message(format!("{name}  adding apt sources..."));
            add_apt_sources(name, sources, priv_mode)?;
        }

        if let Some(gpg_keys) = &entry.gpg_keys {
            for url in gpg_keys {
                pb.set_message(format!("{name}  importing GPG key..."));
                import_apt_gpg_key(name, url, client, priv_mode).await?;
            }
        }

        if let Some(selections) = &entry.debconf_selections {
            pb.set_message(format!("{name}  setting debconf selections..."));
            run_debconf_selections(selections, priv_mode)?;
        }

        pb.set_message(format!("{name}  updating package index..."));
        apt_get(priv_mode, &["-y", "update"], &[], &[])?;

        pb.set_message(format!("{name}  installing via apt..."));
        let extra_flags: Vec<&str> = entry
            .apt_flags
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        let extra_env: Vec<(String, String)> = entry
            .apt_env
            .as_ref()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let ok = apt_get(priv_mode, &["install", "-y", &pkg], &extra_flags, &extra_env)
            .map(|s| s.success())
            .unwrap_or(false);

        if !ok {
            return Err(GripError::CommandFailed(format!("apt-get install {pkg}")));
        }
    }

    // Issue 9: verify installation and compute sha256 of installed library files.
    let install_ok = Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", &entry.package])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("install ok installed"))
        .unwrap_or(false);
    if !install_ok {
        return Err(GripError::CommandFailed(format!(
            "post-install check failed: package `{}` is not in state 'install ok installed'",
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
        source: "apt".to_string(),
        url: None,
        sha256: lib_sha256,
        installed_at: chrono::Utc::now(),
        extra_binaries: vec![],
        auto_binary: None,
        auto_extra_binaries: vec![],
    })
}

/// Issue 1: Write additional apt source entries to /etc/apt/sources.list.d/.
fn add_apt_sources(
    name: &str,
    sources: &[String],
    priv_mode: PrivilegeMode,
) -> Result<(), GripError> {
    let safe_name: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let list_path = format!("/etc/apt/sources.list.d/grip-{safe_name}.list");
    let content = sources.join("\n") + "\n";
    match priv_mode {
        PrivilegeMode::Root => {
            std::fs::write(&list_path, &content).map_err(GripError::Io)?;
        }
        PrivilegeMode::Sudo => {
            let mut child = Command::new("sudo")
                .args(["tee", &list_path])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(GripError::Io)?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(content.as_bytes()).map_err(GripError::Io)?;
            }
            child.wait().map_err(GripError::Io)?;
        }
    }
    Ok(())
}

/// Issue 2: Download a GPG key and dearmor it into /usr/share/keyrings/.
async fn import_apt_gpg_key(
    name: &str,
    url: &str,
    client: &Client,
    priv_mode: PrivilegeMode,
) -> Result<(), GripError> {
    let safe_name: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let keyring_path = format!("/usr/share/keyrings/grip-{safe_name}.gpg");

    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|e| GripError::Other(format!("failed to download GPG key {url}: {e}")))?
        .bytes()
        .await
        .map_err(|e| GripError::Other(format!("failed to read GPG key {url}: {e}")))?;

    // Dearmor the key in-memory via gpg --dearmor.
    let mut gpg_child = Command::new("gpg")
        .args(["--dearmor"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(GripError::Io)?;
    if let Some(mut stdin) = gpg_child.stdin.take() {
        stdin.write_all(&bytes).map_err(GripError::Io)?;
    }
    let gpg_output = gpg_child
        .wait_with_output()
        .map_err(GripError::Io)?
        .stdout;

    // Write the dearmored key to the keyring path using privilege.
    match priv_mode {
        PrivilegeMode::Root => {
            std::fs::write(&keyring_path, &gpg_output).map_err(GripError::Io)?;
        }
        PrivilegeMode::Sudo => {
            let mut child = Command::new("sudo")
                .args(["tee", &keyring_path])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(GripError::Io)?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&gpg_output).map_err(GripError::Io)?;
            }
            child.wait().map_err(GripError::Io)?;
        }
    }
    Ok(())
}

/// Issue 2: Feed debconf selection strings to `debconf-set-selections`.
fn run_debconf_selections(
    selections: &[String],
    priv_mode: PrivilegeMode,
) -> Result<(), GripError> {
    let content = selections.join("\n") + "\n";
    let mut child = match priv_mode {
        PrivilegeMode::Root => Command::new("debconf-set-selections")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(GripError::Io)?,
        PrivilegeMode::Sudo => Command::new("sudo")
            .arg("debconf-set-selections")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(GripError::Io)?,
    };
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(content.as_bytes()).map_err(GripError::Io)?;
    }
    child.wait().map_err(GripError::Io)?;
    Ok(())
}

/// Issue 9: Compute a combined SHA-256 over the shared-library files installed by a package.
/// Finds `.so` files via `dpkg -L`, sorts them, hashes each, then hashes all the digests
/// together. Returns `None` if no library files are found or hashing fails.
fn compute_library_sha256(package: &str) -> Option<String> {
    let out = Command::new("dpkg").args(["-L", package]).output().ok()?;
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
/// `extra_flags` are appended after `args`; `extra_env` vars are set in addition to
/// the default `DEBIAN_FRONTEND=noninteractive`.
fn apt_get(
    priv_mode: PrivilegeMode,
    args: &[&str],
    extra_flags: &[&str],
    extra_env: &[(String, String)],
) -> Result<std::process::ExitStatus, GripError> {
    let status = match priv_mode {
        PrivilegeMode::Root => {
            let mut cmd = Command::new("apt-get");
            cmd.env("DEBIAN_FRONTEND", "noninteractive");
            for (k, v) in extra_env {
                cmd.env(k, v);
            }
            cmd.args(args)
                .args(extra_flags)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()?
        }
        PrivilegeMode::Sudo => {
            let mut cmd = Command::new("sudo");
            cmd.args(["env", "DEBIAN_FRONTEND=noninteractive"]);
            for (k, v) in extra_env {
                cmd.arg(format!("{k}={v}"));
            }
            cmd.arg("apt-get")
                .args(args)
                .args(extra_flags)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()?
        }
    };
    Ok(status)
}

/// Query the candidate (latest available) version for a package via `apt-cache policy`.
/// Returns `None` if the command fails or the package is not known to APT.
fn apt_cache_candidate(package: &str) -> Option<String> {
    let out = Command::new("apt-cache")
        .args(["policy", package])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_apt_cache_policy_output(&String::from_utf8_lossy(&out.stdout))
}

/// Parse the text output of `apt-cache policy` and return the `Candidate:` version.
/// Returns `None` when the field is absent, empty, or `(none)`.
pub(crate) fn parse_apt_cache_policy_output(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Candidate:") {
            let v = rest.trim();
            if !v.is_empty() && v != "(none)" {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Query the installed version by looking up which dpkg package owns `path`.
/// Used as a fallback when the manifest package name doesn't match the dpkg name exactly.
fn version_from_path(path: &std::path::Path) -> Option<String> {
    let path_str = path.to_str()?;
    let out = Command::new("dpkg").args(["-S", path_str]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    // Output format: "package-name: /path/to/file" or "package:arch: /path/to/file"
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next()?;
    // Split on ':' — first token is package name (possibly with ":arch" stripped)
    let pkg = line.split(':').next()?.trim();
    if pkg.is_empty() {
        return None;
    }
    installed_version(pkg)
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

#[cfg(test)]
mod tests {
    use super::parse_apt_cache_policy_output;

    const TYPICAL_OUTPUT: &str = "\
ripgrep:
  Installed: 13.0.0-4build2
  Candidate: 14.1.0-2
  Version table:
     14.1.0-2 500
        500 http://archive.ubuntu.com/ubuntu jammy/universe amd64 Packages
 *** 13.0.0-4build2 100
        100 /var/lib/dpkg/status";

    #[test]
    fn parses_candidate_from_typical_output() {
        assert_eq!(
            parse_apt_cache_policy_output(TYPICAL_OUTPUT),
            Some("14.1.0-2".to_string())
        );
    }

    #[test]
    fn returns_none_when_candidate_is_none_literal() {
        let output = "somepkg:\n  Installed: (none)\n  Candidate: (none)\n";
        assert!(parse_apt_cache_policy_output(output).is_none());
    }

    #[test]
    fn returns_none_when_candidate_line_is_absent() {
        let output = "somepkg:\n  Installed: 1.0.0\n";
        assert!(parse_apt_cache_policy_output(output).is_none());
    }

    #[test]
    fn returns_none_for_empty_output() {
        assert!(parse_apt_cache_policy_output("").is_none());
    }

    #[test]
    fn handles_extra_whitespace_around_version() {
        let output = "pkg:\n  Candidate:   2.0.0  \n";
        assert_eq!(
            parse_apt_cache_policy_output(output),
            Some("2.0.0".to_string())
        );
    }
}
