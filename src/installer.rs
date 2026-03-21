//! Core install logic — reads the manifest, runs adapters concurrently, and writes the lock file.

use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use std::sync::Arc;

use crate::adapters::{apt as apt_adapter, dnf as dnf_adapter, get_adapter};
use crate::bin_dir::ensure_bin_dir;
use crate::checksum::sha256_file;
use crate::config::lockfile::LockFile;
use crate::config::manifest::{find_manifest_dir, LibraryEntry, Manifest};
use crate::error::GripError;
use crate::output;
use crate::platform::Platform;

/// UI options for [`run_install`].
#[derive(Clone, Copy, Debug)]
pub struct InstallOptions {
    pub quiet: bool,
    pub colored: bool,
}

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
pub async fn run_install(
    locked: bool,
    verify: bool,
    tag: Option<&str>,
    root: Option<PathBuf>,
    ui: InstallOptions,
) -> Result<InstallResult, GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };

    let manifest_path = project_root.join("grip.toml");
    let lock_path = project_root.join("grip.lock");
    let bin_dir = ensure_bin_dir(&project_root)?;

    let manifest = Manifest::load(&manifest_path)?;
    let mut lock = LockFile::load(&lock_path)?;
    let platform = Platform::current();

    let cache: Option<Arc<crate::cache::Cache>> = match crate::cache::Cache::open() {
        None => None,
        Some(Ok(c)) => Some(Arc::new(c)),
        Some(Err(_)) => None,
    };

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

    // Split into system PM installs (apt/dnf — must be sequential to avoid lock contention)
    // and download-based installs (github/url/shell — safe to run concurrently).
    let mut pm_installs: Vec<(String, _, Option<String>)> = Vec::new();
    let mut download_installs: Vec<(String, _, Option<String>)> = Vec::new();
    for item in to_install {
        match &item.1 {
            crate::config::manifest::BinaryEntry::Apt(_)
            | crate::config::manifest::BinaryEntry::Dnf(_) => pm_installs.push(item),
            _ => download_installs.push(item),
        }
    }

    let total = pm_installs.len() + download_installs.len();

    // Set up multi-progress display.
    let mp = MultiProgress::new();
    let spinner_style = ProgressStyle::with_template(output::install_spinner_template(ui.colored))
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]);

    if total > 0 && !ui.quiet {
        let noun = if total == 1 { "binary" } else { "binaries" };
        mp.println(format!("  Installing {total} {noun}...\n")).ok();
    }

    let mut pb_map: HashMap<String, ProgressBar> = HashMap::new();

    // ── Sequential pass: system package manager binaries ───────────────────
    for (idx, (name, entry, post_install)) in pm_installs.into_iter().enumerate() {
        let pb = if ui.quiet {
            ProgressBar::hidden()
        } else {
            mp.add(ProgressBar::new_spinner())
        };
        if !ui.quiet {
            pb.set_style(spinner_style.clone());
        }
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

        let adapter = get_adapter(&entry, cache.clone());
        let res = if !adapter.is_supported() {
            pb.finish_with_message(format!(
                "{} {name}  skipped — unsupported platform",
                output::dim(ui.colored, "-")
            ));
            Err(GripError::UnsupportedPlatform { adapter: adapter.name().to_string() })
        } else {
            adapter.install(&name, &entry, &bin_dir, &client, pb, ui.colored).await
        };

        handle_install_result(
            name,
            post_install,
            res,
            &required_flags,
            &mut lock,
            &mut outcome,
            &mp,
            ui,
        );
    }

    // ── Concurrent pass: download-based binaries ────────────────────────────
    let pm_count = pb_map.len(); // offset for display index
    let mut futures: FuturesUnordered<_> = FuturesUnordered::new();

    for (idx, (name, entry, post_install)) in download_installs.into_iter().enumerate() {
        let pb = if ui.quiet {
            ProgressBar::hidden()
        } else {
            mp.add(ProgressBar::new_spinner())
        };
        if !ui.quiet {
            pb.set_style(spinner_style.clone());
        }
        pb.set_prefix(format!("[{}/{}]", pm_count + idx + 1, total));
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
        let colored = ui.colored;
        let cache = cache.clone();

        futures.push(async move {
            let adapter = get_adapter(&entry, cache);
            if !adapter.is_supported() {
                pb.finish_with_message(format!(
                    "{} {name}  skipped — unsupported platform",
                    output::dim(colored, "-")
                ));
                return (
                    name.clone(),
                    post_install,
                    Err(GripError::UnsupportedPlatform {
                        adapter: adapter.name().to_string(),
                    }),
                );
            }
            let res = adapter
                .install(&name, &entry, &bin_dir, &client, pb, colored)
                .await;
            (name, post_install, res)
        });
    }

    while let Some((name, post_install, res)) = futures.next().await {
        handle_install_result(
            name,
            post_install,
            res,
            &required_flags,
            &mut lock,
            &mut outcome,
            &mp,
            ui,
        );
    }

    // ── Library install pass ────────────────────────────────────────────────
    let library_required_flags: HashMap<String, bool> = manifest
        .libraries
        .iter()
        .map(|(n, e)| (n.clone(), e.meta().is_required()))
        .collect();

    let mut libs_to_install: Vec<(String, LibraryEntry, Option<String>)> = Vec::new();

    for (name, entry) in &manifest.libraries {
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

        if lock.get_library(name).is_some() {
            outcome.skipped.push(name.clone());
            continue;
        }

        libs_to_install.push((name.clone(), entry.clone(), meta.post_install.clone()));
    }

    let lib_total = libs_to_install.len();
    if lib_total > 0 && !ui.quiet {
        let noun = if lib_total == 1 { "library" } else { "libraries" };
        mp.println(format!("  Installing {lib_total} {noun}...\n")).ok();
    }

    // Libraries are installed sequentially to avoid package manager lock contention.
    for (idx, (name, entry, post_install)) in libs_to_install.into_iter().enumerate() {
        let pb = if ui.quiet {
            indicatif::ProgressBar::hidden()
        } else {
            mp.add(indicatif::ProgressBar::new_spinner())
        };
        if !ui.quiet {
            pb.set_style(spinner_style.clone());
        }
        pb.set_prefix(format!("[{}/{}]", idx + 1, lib_total));
        pb.set_message(format!("{name}  resolving..."));
        pb.enable_steady_tick(std::time::Duration::from_millis(80));

        let result = match &entry {
            LibraryEntry::Apt(a) => {
                if !platform.is_linux() {
                    Err(GripError::UnsupportedPlatform { adapter: "apt".to_string() })
                } else {
                    apt_adapter::install_apt_library(&name, a, pb.clone(), ui.colored).await
                }
            }
            LibraryEntry::Dnf(d) => {
                if !platform.is_linux() || !which_exists("dnf") {
                    Err(GripError::UnsupportedPlatform { adapter: "dnf".to_string() })
                } else {
                    dnf_adapter::install_dnf_library(&name, d, pb.clone(), ui.colored).await
                }
            }
        };

        match result {
            Ok(lock_entry) => {
                if let Some(cmd) = post_install {
                    let status = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(&cmd)
                        .status();
                    match status {
                        Ok(s) if !s.success() => {
                            if ui.quiet {
                                eprintln!("warning: post_install failed for {name}: {cmd}");
                            } else {
                                let g = output::warn_glyph(ui.colored);
                                mp.println(format!("  {g}  post_install failed for {name}: {cmd}")).ok();
                            }
                        }
                        Err(e) => {
                            if ui.quiet {
                                eprintln!("warning: post_install error for {name}: {e}");
                            } else {
                                let g = output::warn_glyph(ui.colored);
                                mp.println(format!("  {g}  post_install error for {name}: {e}")).ok();
                            }
                        }
                        _ => {}
                    }
                }
                outcome.installed.push(name);
                lock.upsert_library(lock_entry);
            }
            Err(GripError::UnsupportedPlatform { .. }) => {
                outcome.skipped.push(name);
            }
            Err(e) => {
                let required = library_required_flags.get(&name).copied().unwrap_or(true);
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

/// Process the result of a single binary install and update `outcome` and `lock` accordingly.
fn handle_install_result(
    name: String,
    post_install: Option<String>,
    res: Result<crate::config::lockfile::LockEntry, GripError>,
    required_flags: &HashMap<String, bool>,
    lock: &mut crate::config::lockfile::LockFile,
    outcome: &mut InstallResult,
    mp: &indicatif::MultiProgress,
    ui: InstallOptions,
) {
    match res {
        Ok(lock_entry) => {
            if let Some(cmd) = post_install {
                let status = std::process::Command::new("sh").arg("-c").arg(&cmd).status();
                match status {
                    Ok(s) if !s.success() => {
                        if ui.quiet {
                            eprintln!("warning: post_install failed for {name}: {cmd}");
                        } else {
                            let g = output::warn_glyph(ui.colored);
                            mp.println(format!("  {g}  post_install failed for {name}: {cmd}")).ok();
                        }
                    }
                    Err(e) => {
                        if ui.quiet {
                            eprintln!("warning: post_install error for {name}: {e}");
                        } else {
                            let g = output::warn_glyph(ui.colored);
                            mp.println(format!("  {g}  post_install error for {name}: {e}")).ok();
                        }
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
            let required = required_flags.get(&name).copied().unwrap_or(true);
            if required {
                outcome.failed.push((name, e.to_string()));
            } else {
                outcome.warned.push((name, e.to_string()));
            }
        }
    }
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
