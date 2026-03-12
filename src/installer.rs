//! Core install logic — reads the manifest, runs adapters concurrently, and writes the lock file.

use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::adapters::get_adapter;
use crate::bin_dir::ensure_bin_dir;
use crate::checksum::sha256_file;
use crate::config::lockfile::LockFile;
use crate::config::manifest::{find_manifest_dir, Manifest};
use crate::error::GripError;
use crate::platform::Platform;

/// Summary of a completed install run.
pub struct InstallResult {
    /// Names of binaries that were newly installed.
    pub installed: Vec<String>,
    /// Names of binaries that were already present and skipped.
    pub skipped: Vec<String>,
    /// Required binaries that failed to install, with their error messages.
    pub failed: Vec<(String, String)>,
    /// Optional binaries that failed to install, with their error messages.
    pub warned: Vec<(String, String)>,
}

/// Run a full install pass.
///
/// - `locked`: pin versions to those recorded in the lock file and fail if the lock file would
///   change.
/// - `verify`: re-verify the SHA-256 of already-installed binaries against the lock file.
/// - `tag`: when `Some`, only install entries that carry this tag.
pub async fn run_install(locked: bool, verify: bool, tag: Option<&str>, root: Option<PathBuf>) -> Result<InstallResult, GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };

    let manifest_path = project_root.join("binaries.toml");
    let lock_path = project_root.join("binaries.lock");
    let bin_dir = ensure_bin_dir(&project_root)?;

    let manifest = Manifest::load(&manifest_path)?;
    let mut lock = LockFile::load(&lock_path)?;
    let platform = Platform::current();

    let client = Client::builder()
        .user_agent("grip/0.1")
        .build()
        .map_err(GripError::Http)?;

    let mut outcome = InstallResult {
        installed: vec![],
        skipped: vec![],
        failed: vec![],
        warned: vec![],
    };

    // Collect required flags before processing entries.
    let required_flags: HashMap<String, bool> = manifest
        .binaries
        .iter()
        .map(|(n, e)| (n.clone(), e.meta().is_required()))
        .collect();

    // First pass: split into skipped vs to-install.
    let mut to_install: Vec<(String, _, Option<String>)> = Vec::new();

    for (name, entry) in &manifest.binaries {
        let meta = entry.meta();

        if !meta.matches_platform(platform.os_str()) {
            outcome.skipped.push(name.clone());
            continue;
        }

        if let Some(t) = tag {
            if !meta.has_tag(t) {
                continue;
            }
        }

        let bin_path = bin_dir.join(name);
        if lock.get(name).is_some() && bin_path.exists() {
            if verify || locked {
                if let Some(lock_entry) = lock.get(name) {
                    if let Some(expected) = &lock_entry.sha256 {
                        let got = sha256_file(&bin_path).map_err(GripError::Io)?;
                        if &got != expected {
                            return Err(GripError::ChecksumMismatch {
                                expected: expected.clone(),
                                got,
                            });
                        }
                    }
                }
            }
            outcome.skipped.push(name.clone());
            continue;
        }

        to_install.push((name.clone(), entry.clone(), meta.post_install.clone()));
    }

    let total = to_install.len();

    // Set up multi-progress display.
    let mp = MultiProgress::new();
    let spinner_style = ProgressStyle::with_template("  {prefix:.bold.dim} {spinner:.cyan} {msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]);

    if total > 0 {
        let noun = if total == 1 { "binary" } else { "binaries" };
        mp.println(format!("  Installing {total} {noun}...\n")).ok();
    }

    let mut pb_map: HashMap<String, ProgressBar> = HashMap::new();
    let mut futures: FuturesUnordered<_> = FuturesUnordered::new();

    for (idx, (name, entry, post_install)) in to_install.into_iter().enumerate() {
        let pb = mp.add(ProgressBar::new_spinner());
        pb.set_style(spinner_style.clone());
        pb.set_prefix(format!("[{}/{}]", idx + 1, total));
        pb.set_message(format!("{name}  resolving..."));
        pb.enable_steady_tick(Duration::from_millis(80));

        pb_map.insert(name.clone(), pb.clone());

        let entry = if locked {
            if let Some(lock_entry) = lock.get(&name) {
                entry.pin_version(lock_entry.version.as_str())
            } else {
                entry
            }
        } else {
            entry
        };

        let bin_dir = bin_dir.clone();
        let client = client.clone();

        futures.push(async move {
            let adapter = get_adapter(&entry);
            if !adapter.is_supported() {
                pb.finish_with_message(format!("\x1b[2m-\x1b[0m {name}  skipped — unsupported platform"));
                return (
                    name.clone(),
                    post_install,
                    Err(GripError::UnsupportedPlatform {
                        adapter: adapter.name().to_string(),
                    }),
                );
            }
            let res = adapter.install(&name, &entry, &bin_dir, &client, pb).await;
            (name, post_install, res)
        });
    }

    while let Some((name, post_install, res)) = futures.next().await {
        match res {
            Ok(lock_entry) => {
                if let Some(cmd) = post_install {
                    let status = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(&cmd)
                        .status();
                    match status {
                        Ok(s) if !s.success() => {
                            mp.println(format!("  \x1b[33m⚠\x1b[0m  post_install failed for {name}: {cmd}")).ok();
                        }
                        Err(e) => {
                            mp.println(format!("  \x1b[33m⚠\x1b[0m  post_install error for {name}: {e}")).ok();
                        }
                        _ => {}
                    }
                }
                outcome.installed.push(name);
                lock.upsert(lock_entry);
            }
            Err(GripError::UnsupportedPlatform { .. }) => {
                outcome.skipped.push(name);
            }
            Err(e) => {
                if let Some(pb) = pb_map.get(&name) {
                    if !pb.is_finished() {
                        pb.finish_with_message(format!("\x1b[31m✗\x1b[0m {name}  failed"));
                    }
                }
                let required = required_flags.get(&name).copied().unwrap_or(true);
                if required {
                    outcome.failed.push((name, e.to_string()));
                } else {
                    outcome.warned.push((name, e.to_string()));
                }
            }
        }
    }

    if !locked || !outcome.installed.is_empty() {
        lock.save(&lock_path)?;
    }

    Ok(outcome)
}
