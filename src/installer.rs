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
    pub interactive: bool,
    /// Fail before installing if any entry has no version pin.
    /// Prevents silent auto-upgrades in CI.
    pub require_pins: bool,
}

impl InstallOptions {
    fn plain_progress(&self) -> bool {
        !self.quiet && !self.interactive
    }
}

/// Summary of a completed install run.
#[derive(Debug)]
pub struct InstallResult {
    /// Names of binaries that were newly installed.
    pub installed: Vec<String>,
    /// Names of binaries that were already present and skipped.
    pub skipped: Vec<String>,
    /// Required binaries that failed to install, with their error messages.
    pub failed: Vec<(String, String)>,
    /// Optional binaries that failed to install, with their error messages.
    pub warned: Vec<(String, String)>,
    /// Entries where the binary name was auto-detected and back-patched into grip.toml.
    /// Each element is `(entry_name, detected_binary_name)`.
    pub binary_overrides: Vec<(String, String)>,
    /// Entries where extra binary names were auto-detected and back-patched into grip.toml.
    /// Each element is `(entry_name, detected_extra_names)`.
    pub extra_binary_overrides: Vec<(String, Vec<String>)>,
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

    if manifest.binaries.is_empty() && manifest.libraries.is_empty() {
        if !ui.quiet {
            let has_dockerfile = ["Dockerfile", "dockerfile", "Containerfile"]
                .iter()
                .any(|n| project_root.join(n).exists())
                || std::fs::read_dir(&project_root)
                    .ok()
                    .map(|entries| {
                        entries.flatten().any(|e| {
                            let fname = e.file_name();
                            let name = fname.to_string_lossy().to_ascii_lowercase();
                            name.starts_with("dockerfile") || name.ends_with(".dockerfile")
                        })
                    })
                    .unwrap_or(false);
            if has_dockerfile {
                eprintln!(
                    "hint: grip.toml is empty — run `grip init` to import packages from your Dockerfile."
                );
            } else {
                eprintln!(
                    "hint: grip.toml is empty — try `grip suggest --path src/` to discover candidates."
                );
            }
        }
        return Ok(InstallResult {
            installed: vec![],
            skipped: vec![],
            failed: vec![],
            warned: vec![],
            binary_overrides: vec![],
            extra_binary_overrides: vec![],
        });
    }

    // --require-pins: fail before touching the network if any entry floats.
    if ui.require_pins {
        let unpinned: Vec<String> = manifest
            .binaries
            .iter()
            .filter(|(_, e)| !e.is_version_pinned())
            .map(|(name, _)| name.clone())
            .collect();
        if !unpinned.is_empty() {
            return Err(crate::error::GripError::UnpinnedEntries {
                names: unpinned.join(", "),
            });
        }
    }

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
        binary_overrides: vec![],
        extra_binary_overrides: vec![],
    };

    // Collect required flags before processing entries.
    let required_flags: HashMap<String, bool> = manifest
        .binaries
        .iter()
        .map(|(n, e)| (n.clone(), e.meta().is_required()))
        .collect();

    // First pass: split into skipped vs to-install.
    let mut to_install: Vec<(String, _)> = Vec::new();

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
        if let Some(lock_entry) = lock.get(name) {
            // All extras recorded in the lock must exist on disk.
            let lock_extras_ok = lock_entry
                .extra_binaries
                .iter()
                .all(|b| bin_dir.join(b).exists());
            // All extras declared in the manifest must also exist on disk.
            // This catches the case where extra_binaries was added to grip.toml
            // after the last install, so the lock entry predates the field.
            let manifest_extras_ok = entry
                .extra_binaries()
                .iter()
                .all(|b| bin_dir.join(b).exists());
            if bin_path.exists() && lock_extras_ok && manifest_extras_ok {
                if verify || locked {
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
                outcome.skipped.push(name.clone());
                continue;
            }
        }

        to_install.push((name.clone(), entry.clone()));
    }

    // Split into system PM installs (apt/dnf — must be sequential to avoid lock contention)
    // and download-based installs (github/url — safe to run concurrently).
    let mut pm_installs: Vec<(String, _)> = Vec::new();
    let mut download_installs: Vec<(String, _)> = Vec::new();
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
        if ui.plain_progress() {
            eprintln!("Installing {total} {noun}...");
        } else {
            mp.println(format!("  Installing {total} {noun}...\n")).ok();
        }
    }

    let mut pb_map: HashMap<String, ProgressBar> = HashMap::new();

    // ── Sequential pass: system package manager binaries ───────────────────
    for (idx, (name, entry)) in pm_installs.into_iter().enumerate() {
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
        if ui.plain_progress() {
            eprintln!("[{}/{}] {name}: resolving...", idx + 1, total);
        }

        // Issue 8: always use the locked version when one exists, so the lockfile
        // is enforceable and not merely advisory on fresh machines.
        let entry = if let Some(lock_entry) = lock.get(&name) {
            entry.pin_version(lock_entry.version.as_str())
        } else {
            entry
        };

        let adapter = get_adapter(&entry, cache.clone());
        let res = if !adapter.is_supported() {
            pb.finish_with_message(format!(
                "{} {name}  skipped — unsupported platform",
                output::dim(ui.colored, "-")
            ));
            Err(GripError::UnsupportedPlatform {
                adapter: adapter.name().to_string(),
            })
        } else {
            adapter
                .install(&name, &entry, &bin_dir, &client, pb, ui.colored)
                .await
        };

        handle_install_result(name, res, &required_flags, &mut lock, &mut outcome);
    }

    // ── Concurrent pass: download-based binaries ────────────────────────────
    let pm_count = pb_map.len(); // offset for display index
    let mut futures: FuturesUnordered<_> = FuturesUnordered::new();

    for (idx, (name, entry)) in download_installs.into_iter().enumerate() {
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
        if ui.plain_progress() {
            eprintln!("[{}/{}] {name}: resolving...", pm_count + idx + 1, total);
        }

        pb_map.insert(name.clone(), pb.clone());

        // Issue 8: always use the locked version when one exists.
        let entry = if let Some(lock_entry) = lock.get(&name) {
            entry.pin_version(lock_entry.version.as_str())
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
                    Err(GripError::UnsupportedPlatform {
                        adapter: adapter.name().to_string(),
                    }),
                );
            }
            let res = adapter
                .install(&name, &entry, &bin_dir, &client, pb, colored)
                .await;
            (name, res)
        });
    }

    while let Some((name, res)) = futures.next().await {
        handle_install_result(name, res, &required_flags, &mut lock, &mut outcome);
    }

    // ── Library install pass ────────────────────────────────────────────────
    let library_required_flags: HashMap<String, bool> = manifest
        .libraries
        .iter()
        .map(|(n, e)| (n.clone(), e.meta().is_required()))
        .collect();

    let mut libs_to_install: Vec<(String, LibraryEntry)> = Vec::new();

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
            let on_system = match entry {
                LibraryEntry::Apt(a) => apt_adapter::installed_version(&a.package).is_some(),
                LibraryEntry::Dnf(d) => dnf_adapter::installed_version(&d.package).is_some(),
            };
            if on_system {
                outcome.skipped.push(name.clone());
                continue;
            }
            // Lock entry exists but the library was removed from the system — reinstall.
        }

        libs_to_install.push((name.clone(), entry.clone()));
    }

    let lib_total = libs_to_install.len();
    if lib_total > 0 && !ui.quiet {
        let noun = if lib_total == 1 {
            "library"
        } else {
            "libraries"
        };
        if ui.plain_progress() {
            eprintln!("Installing {lib_total} {noun}...");
        } else {
            mp.println(format!("  Installing {lib_total} {noun}...\n"))
                .ok();
        }
    }

    // Libraries are installed sequentially to avoid package manager lock contention.
    for (idx, (name, entry)) in libs_to_install.into_iter().enumerate() {
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
        if ui.plain_progress() {
            eprintln!("[{}/{}] {name}: resolving...", idx + 1, lib_total);
        }

        let result = match &entry {
            LibraryEntry::Apt(a) => {
                if !platform.is_linux() {
                    Err(GripError::UnsupportedPlatform {
                        adapter: "apt".to_string(),
                    })
                } else {
                    apt_adapter::install_apt_library(&name, a, &client, pb.clone(), ui.colored)
                        .await
                }
            }
            LibraryEntry::Dnf(d) => {
                if !platform.is_linux() || !which_exists("dnf") {
                    Err(GripError::UnsupportedPlatform {
                        adapter: "dnf".to_string(),
                    })
                } else {
                    dnf_adapter::install_dnf_library(&name, d, &client, pb.clone(), ui.colored)
                        .await
                }
            }
        };

        match result {
            Ok(lock_entry) => {
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

    // Back-patch grip.toml with any auto-detected binary names and extra binaries
    // so subsequent runs don't need to re-detect.
    if !outcome.binary_overrides.is_empty() || !outcome.extra_binary_overrides.is_empty() {
        let mut manifest = Manifest::load(&manifest_path)?;
        for (entry_name, detected_binary) in &outcome.binary_overrides {
            match manifest.binaries.get_mut(entry_name) {
                Some(crate::config::manifest::BinaryEntry::Dnf(d)) if d.binary.is_none() => {
                    d.binary = Some(detected_binary.clone());
                }
                Some(crate::config::manifest::BinaryEntry::Apt(a)) if a.binary.is_none() => {
                    a.binary = Some(detected_binary.clone());
                }
                _ => {}
            }
        }
        for (entry_name, extras) in &outcome.extra_binary_overrides {
            match manifest.binaries.get_mut(entry_name) {
                Some(crate::config::manifest::BinaryEntry::Dnf(d))
                    if d.extra_binaries.is_none() =>
                {
                    d.extra_binaries = Some(extras.clone());
                }
                Some(crate::config::manifest::BinaryEntry::Apt(a))
                    if a.extra_binaries.is_none() =>
                {
                    a.extra_binaries = Some(extras.clone());
                }
                _ => {}
            }
        }
        manifest.save(&manifest_path)?;
    }

    Ok(outcome)
}

/// Process the result of a single binary install and update `outcome` and `lock` accordingly.
fn handle_install_result(
    name: String,
    res: Result<crate::config::lockfile::LockEntry, GripError>,
    required_flags: &HashMap<String, bool>,
    lock: &mut crate::config::lockfile::LockFile,
    outcome: &mut InstallResult,
) {
    match res {
        Ok(lock_entry) => {
            if let Some(detected) = lock_entry.auto_binary.clone() {
                outcome.binary_overrides.push((name.clone(), detected));
            }
            if !lock_entry.auto_extra_binaries.is_empty() {
                outcome
                    .extra_binary_overrides
                    .push((name.clone(), lock_entry.auto_extra_binaries.clone()));
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

pub(crate) fn which_exists(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|dir| dir.join(cmd).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn opts(require_pins: bool) -> InstallOptions {
        InstallOptions {
            quiet: true,
            colored: false,
            interactive: false,
            require_pins,
        }
    }

    #[test]
    fn plain_progress_is_enabled_only_for_non_interactive_non_quiet_runs() {
        let mut opts = InstallOptions {
            quiet: false,
            colored: false,
            interactive: false,
            require_pins: false,
        };
        assert!(opts.plain_progress());

        opts.interactive = true;
        assert!(!opts.plain_progress());

        opts.interactive = false;
        opts.quiet = true;
        assert!(!opts.plain_progress());
    }

    // ── require_pins guard ────────────────────────────────────────────────────

    #[tokio::test]
    async fn require_pins_passes_when_all_pinned() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1" }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();

        // jq is pinned so the guard passes; the install may succeed or fail for
        // other reasons (network) but must NOT fail with UnpinnedEntries.
        let result = run_install(
            false,
            false,
            None,
            Some(tmp.path().to_path_buf()),
            opts(true),
        )
        .await;
        if let Err(e) = result {
            assert!(
                !matches!(e, crate::error::GripError::UnpinnedEntries { .. }),
                "expected no UnpinnedEntries error, got: {e:?}"
            );
        }
    }

    #[tokio::test]
    async fn require_pins_fails_when_entry_has_no_version() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq" }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();

        let err = run_install(
            false,
            false,
            None,
            Some(tmp.path().to_path_buf()),
            opts(true),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, crate::error::GripError::UnpinnedEntries { .. }),
            "expected UnpinnedEntries, got: {err:?}"
        );
        assert!(err.to_string().contains("jq"));
    }

    #[tokio::test]
    async fn require_pins_lists_all_unpinned_names() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq" }
rg = { source = "github", repo = "BurntSushi/ripgrep" }
fd = { source = "github", repo = "sharkdp/fd", version = "9.0.0" }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();

        let err = run_install(
            false,
            false,
            None,
            Some(tmp.path().to_path_buf()),
            opts(true),
        )
        .await
        .unwrap_err();

        let msg = err.to_string();
        // jq and rg are unpinned; fd is pinned
        assert!(msg.contains("jq"), "expected jq in: {msg}");
        assert!(msg.contains("rg"), "expected rg in: {msg}");
        assert!(!msg.contains("fd"), "fd should not be in: {msg}");
    }

    #[tokio::test]
    async fn require_pins_allows_url_entries_without_version() {
        // URL entries are always considered pinned — the URL is the pin.
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
mytool = { source = "url", url = "https://example.com/mytool-1.0" }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();

        // The guard passes; the install will likely fail for other reasons but
        // must NOT fail with UnpinnedEntries.
        let result = run_install(
            false,
            false,
            None,
            Some(tmp.path().to_path_buf()),
            opts(true),
        )
        .await;
        if let Err(e) = result {
            assert!(
                !matches!(e, crate::error::GripError::UnpinnedEntries { .. }),
                "URL entries should never be flagged as unpinned, got: {e:?}"
            );
        }
    }

    #[tokio::test]
    async fn no_require_pins_does_not_block_unpinned_entries() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq" }
"#;
        std::fs::write(tmp.path().join("grip.toml"), toml).unwrap();

        // Without require_pins the guard is skipped — install may fail for other
        // reasons (network offline in CI) but not with UnpinnedEntries.
        let result = run_install(
            false,
            false,
            None,
            Some(tmp.path().to_path_buf()),
            opts(false),
        )
        .await;
        if let Err(e) = result {
            assert!(
                !matches!(e, crate::error::GripError::UnpinnedEntries { .. }),
                "should not get UnpinnedEntries without flag, got: {e:?}"
            );
        }
    }
}
