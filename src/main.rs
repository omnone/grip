//! Entry point and CLI command implementations for `grip`.

mod adapters;
mod audit;
mod bin_dir;
mod cache;
mod checker;
mod checksum;
mod cli;
mod config;
mod error;
mod gpg;
mod installer;
mod lock_verify;
mod output;
mod platform;
mod privilege;
mod purl;
mod sbom;
mod suggest;

use std::io::{IsTerminal, Write};
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};

use clap::Parser;
use cli::{CacheAction, Cli, Commands, LockAction};
use sbom::SbomFormat;
use config::lockfile::LockFile;
use config::manifest::{
    find_manifest_dir, AptEntry, BinaryEntry, DnfEntry, GithubEntry, LibAptEntry, LibDnfEntry,
    LibraryEntry, Manifest, UrlEntry,
};
use error::GripError;
use indicatif::{ProgressBar, ProgressStyle};
use output::OutputCfg;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let cfg = OutputCfg {
        quiet: cli.quiet,
        verbose: cli.verbose,
        color_when: cli.color,
    };

    if let Err(e) = run_command(cli, cfg).await {
        error::print_grip_error(&e, cfg.verbose);
        std::process::exit(1);
    }
}

async fn run_command(cli: Cli, cfg: OutputCfg) -> Result<(), GripError> {
    let root = cli.root;
    let color_out = cfg.use_color_stdout();
    let color_err = cfg.use_color_stderr();

    match cli.command {
        Commands::Init => cmd_init(&cfg)?,
        Commands::Add {
            name,
            source,
            version,
            repo,
            url,
            package,
            binary,
            library,
            gpg_fingerprint,
            sig_asset_pattern,
            checksums_asset_pattern,
            sig_url,
            signed_checksums_url,
            checksums_sig_url,
        } => {
            let root_for_sync = root.clone();
            cmd_add(
                name,
                source,
                version,
                repo,
                url,
                package,
                binary,
                library,
                gpg_fingerprint,
                sig_asset_pattern,
                checksums_asset_pattern,
                sig_url,
                signed_checksums_url,
                checksums_sig_url,
                root,
                &cfg,
            )?;
            let ui = installer::InstallOptions {
                quiet: cfg.quiet,
                colored: color_err,
                require_pins: false,
            };
            let start = std::time::Instant::now();
            let result = installer::run_install(false, false, None, root_for_sync, ui).await?;
            let elapsed = start.elapsed().as_secs_f64();
            print_install_result(&result, &cfg, color_out, color_err, elapsed);
            if !result.failed.is_empty() {
                std::process::exit(1);
            }
        }
        Commands::Sync {
            locked,
            verify,
            tag,
            require_pins,
        } => {
            let start = std::time::Instant::now();
            let ui = installer::InstallOptions {
                quiet: cfg.quiet,
                colored: color_err,
                require_pins,
            };
            let result = installer::run_install(locked, verify, tag.as_deref(), root, ui).await?;
            let elapsed = start.elapsed().as_secs_f64();
            print_install_result(&result, &cfg, color_out, color_err, elapsed);
            if !result.failed.is_empty() {
                std::process::exit(1);
            }
        }
        Commands::Remove { name, library } => cmd_remove(name, library, root, &cfg)?,
        Commands::Check { tag } => {
            let r = checker::run_check(tag.as_deref(), root)?;
            cmd_check_print(r, &cfg, color_out, color_err)?;
        }
        Commands::Run { args } => cmd_run(args, root)?,
        Commands::List { all } => cmd_list(root, all, &cfg)?,
        Commands::Update { name, all } => cmd_update(name, all, root, &cfg).await?,
        Commands::Outdated { tag } => cmd_outdated(tag, root, &cfg).await?,
        Commands::Pin { dry_run } => cmd_pin(dry_run, root, &cfg)?,
        Commands::Env { shell } => cmd_env(shell, root, &cfg)?,
        Commands::Cache { action } => cmd_cache(action, &cfg)?,
        Commands::Lock { action } => cmd_lock(action, root, &cfg)?,
        Commands::Export { format } => cmd_export(&format, root, &cfg)?,
        Commands::Sbom { format, output } => {
            let fmt = match format.as_str() {
                "spdx" => SbomFormat::Spdx,
                _ => SbomFormat::CycloneDx,
            };
            sbom::run_sbom(root, sbom::SbomOptions { format: fmt, output })?;
        }
        Commands::Audit { no_fail } => {
            audit::run_audit(audit::AuditOptions {
                fail: !no_fail,
                root,
                quiet: cfg.quiet,
                color: color_out,
            })
            .await?;
        }
        Commands::Suggest {
            paths,
            history,
            check,
        } => {
            let opts = suggest::SuggestOptions {
                scan_paths: paths,
                history,
                quiet: cfg.quiet,
                color: color_out,
            };
            let n = suggest::run_suggest(root, opts)?;
            if check && n > 0 {
                if !cfg.quiet {
                    eprintln!(
                        "error: {} unmanaged tool{} found — add them to grip.toml or update the scan sources",
                        n,
                        if n == 1 { "" } else { "s" }
                    );
                }
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn cmd_check_print(
    r: checker::CheckResult,
    cfg: &OutputCfg,
    color_out: bool,
    color_err: bool,
) -> Result<(), GripError> {
    use std::collections::HashSet;
    let no_sha: HashSet<&str> = r.no_checksum.iter().map(|s| s.as_str()).collect();

    if r.declared == 0 && r.issues.is_empty() {
        if !cfg.quiet {
            println!("No binaries declared in grip.toml.");
        }
        return Ok(());
    }

    if cfg.quiet {
        for (name, msg) in &r.failed {
            eprintln!("error: {name}: {msg}");
        }
        for (name, msg) in &r.warned {
            eprintln!("warning: {name}: {msg}");
        }
        for issue in &r.issues {
            eprintln!("warning: {issue}");
        }
    } else {
        if r.examined > 0 {
            println!();
            let header = output::dim(color_out, "Checking installed binaries…");
            println!("  {header}");
            println!();

            for name in &r.passed {
                let mark = output::success_checkmark(color_out);
                if no_sha.contains(name.as_str()) {
                    let note = output::dim(color_out, "(no sha256 in lock)");
                    println!("  {mark}  {name}  {note}");
                } else {
                    println!("  {mark}  {name}");
                }
            }

            for (name, msg) in &r.warned {
                let g = output::warn_glyph(color_err);
                eprintln!("  {g}  {name}: {msg}");
            }
            for (name, msg) in &r.failed {
                let x = output::fail_glyph(color_err);
                eprintln!("  {x}  {name}: {msg}");
            }
        } else if r.declared > 0 {
            println!();
            println!("  No binaries matched this check (platform or --tag filter).");
            println!(
                "  hint: {}",
                output::dim(
                    color_out,
                    "Adjust `platforms` / `tags` in grip.toml or run without `--tag`.",
                )
            );
        }

        if !r.issues.is_empty() {
            println!();
            let issues_header = output::dim(color_out, "Consistency issues");
            println!("  {issues_header}");
            println!();
            for issue in &r.issues {
                let w = output::warn_glyph(color_err);
                eprintln!("  {w}  {issue}");
            }
        }

        let n_ok = r.passed.len();
        let n_warn = r.warned.len();
        let n_fail = r.failed.len();
        let n_issues = r.issues.len();
        let summary = if n_fail == 0 && n_warn == 0 && n_issues == 0 {
            output::green(color_out, &format!("All {n_ok} checks passed"))
        } else {
            let mut parts = Vec::new();
            if n_ok > 0 {
                parts.push(output::green(color_out, &format!("{n_ok} ok")));
            }
            if n_warn > 0 {
                parts.push(output::yellow(color_out, &format!("{n_warn} warnings")));
            }
            if n_fail > 0 {
                parts.push(output::red(color_out, &format!("{n_fail} failed")));
            }
            if n_issues > 0 {
                parts.push(output::yellow(
                    color_out,
                    &format!("{n_issues} consistency issue{}", if n_issues == 1 { "" } else { "s" }),
                ));
            }
            parts.join(", ")
        };
        println!("\n  {summary}");
    }

    if !r.failed.is_empty() || !r.issues.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}

/// Create a `grip.toml` template in the current directory and add `.bin/` to `.gitignore`.
fn cmd_init(cfg: &OutputCfg) -> Result<(), GripError> {
    let path = std::path::Path::new("grip.toml");
    let color = cfg.use_color_stdout();
    if path.exists() {
        if !cfg.quiet {
            println!("grip.toml already exists");
            println!(
                "hint: {}",
                output::dim(
                    color,
                    "Add tools with `grip add <name>` then run `grip sync`.",
                )
            );
        }
        return Ok(());
    }

    let template = r#"# grip.toml — managed by grip
# Add binary dependencies under [binaries.<name>] and system libraries under [libraries.<name>].

[binaries]

# Example:
# [binaries.jq]
# source = "github"
# repo = "jqlang/jq"
# version = "1.7.1"
# asset_pattern = "jq-linux-amd64"

[libraries]

# Example:
# [libraries.libssl-dev]
# source = "apt"
# package = "libssl-dev"
"#;
    std::fs::write(path, template)?;
    if !cfg.quiet {
        println!("Created grip.toml");
    }

    // Add .bin/ to .gitignore
    let gitignore = std::path::Path::new(".gitignore");
    let entry = ".bin/\n";
    if gitignore.exists() {
        let content = std::fs::read_to_string(gitignore)?;
        if !content.contains(".bin/") {
            std::fs::OpenOptions::new()
                .append(true)
                .open(gitignore)?
                .write_all(entry.as_bytes())?;
            if !cfg.quiet {
                println!("Added .bin/ to .gitignore");
            }
        }
    } else {
        std::fs::write(gitignore, entry)?;
        if !cfg.quiet {
            println!("Created .gitignore with .bin/");
        }
    }

    if !cfg.quiet {
        println!(
            "hint: {}",
            output::dim(
                color,
                "Run `grip add <name>` then `grip sync` to populate .bin/."
            )
        );
    }

    Ok(())
}

/// Split `name@version` into stem and optional version (last `@` wins).
fn parse_name_at_version(raw: String) -> (String, Option<String>) {
    if let Some(pos) = raw.rfind('@') {
        let (stem, rest) = raw.split_at(pos);
        let ver = rest.strip_prefix('@').filter(|v| !v.is_empty());
        if stem.is_empty() {
            (raw, None)
        } else {
            (stem.to_string(), ver.map(String::from))
        }
    } else {
        (raw, None)
    }
}

/// Add a new binary or library entry to `grip.toml`.
fn cmd_add(
    name: String,
    source: Option<String>,
    version: Option<String>,
    repo: Option<String>,
    url: Option<String>,
    package: Option<String>,
    binary: Option<String>,
    library: bool,
    gpg_fingerprint: Option<String>,
    sig_asset_pattern: Option<String>,
    checksums_asset_pattern: Option<String>,
    sig_url: Option<String>,
    signed_checksums_url: Option<String>,
    checksums_sig_url: Option<String>,
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let (stem, ver_from_at) = parse_name_at_version(name);
    let version = version.or(ver_from_at);

    let (binary_name, github_shorthand_repo) = if stem.contains('/') {
        // owner/repo shorthand always implies GitHub — require the user to be explicit.
        match source.as_deref() {
            Some("github") => {}
            Some(other) => {
                return Err(GripError::Other(format!(
                    "NAME '{stem}' looks like owner/repo but --source is '{other}'; \
                     use a simple binary name for non-GitHub sources"
                )));
            }
            None => {
                return Err(GripError::Other(format!(
                    "NAME '{stem}' looks like owner/repo; pass --source github explicitly \
                     (e.g. `grip add {stem} --source github`)"
                )));
            }
        }
        let repo_full = stem.clone();
        let bn = stem
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&stem)
            .to_string();
        (bn, Some(repo_full))
    } else {
        (stem, None)
    };

    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).unwrap_or(cwd)
        }
    };
    let manifest_path = project_root.join("grip.toml");

    let mut manifest = if manifest_path.exists() {
        Manifest::load(&manifest_path)?
    } else {
        Manifest::empty()
    };

    let default_source;
    let source_str = if let Some(s) = source.as_deref() {
        s
    } else {
        default_source = detect_default_source()?;
        &default_source
    };

    let repo_resolved: Option<String> = match (&github_shorthand_repo, &repo) {
        (Some(g), None) => Some(g.clone()),
        (None, Some(r)) => Some(r.clone()),
        (Some(g), Some(r)) if g == r => Some(g.clone()),
        (Some(g), Some(r)) => {
            return Err(GripError::Other(format!(
                "Conflicting GitHub repos: NAME implies `{g}` but `--repo` is `{r}`."
            )));
        }
        (None, None) => None,
    };

    if library {
        let lib_entry = match source_str {
            "apt" => LibraryEntry::Apt(LibAptEntry {
                package: package.unwrap_or_else(|| binary_name.clone()),
                version,
                ..Default::default()
            }),
            "dnf" => LibraryEntry::Dnf(LibDnfEntry {
                package: package.unwrap_or_else(|| binary_name.clone()),
                version,
                ..Default::default()
            }),
            other => {
                return Err(GripError::Other(format!(
                    "source `{other}` is not supported for libraries; use `apt` or `dnf`"
                )))
            }
        };
        manifest.libraries.insert(binary_name.clone(), lib_entry);
        manifest.save(&manifest_path)?;
        if !cfg.quiet {
            println!("Added '{}' to [libraries] in grip.toml", binary_name);
        }
        return Ok(());
    }

    let entry = match source_str {
        "apt" => BinaryEntry::Apt(AptEntry {
            package: package.unwrap_or_else(|| binary_name.clone()),
            binary,
            version,
            ..Default::default()
        }),
        "dnf" => BinaryEntry::Dnf(DnfEntry {
            package: package.unwrap_or_else(|| binary_name.clone()),
            binary,
            version,
            ..Default::default()
        }),
        "github" => BinaryEntry::Github(GithubEntry {
            repo: repo_resolved.ok_or(GripError::RepoRequired)?,
            version,
            asset_pattern: None,
            binary: None,
            extra_binaries: None,
            gpg_fingerprint,
            sig_asset_pattern,
            checksums_asset_pattern,
            meta: Default::default(),
        }),
        "url" => BinaryEntry::Url(UrlEntry {
            url: url.ok_or(GripError::UrlRequired)?,
            binary: None,
            extra_binaries: None,
            sha256: None,
            gpg_fingerprint,
            sig_url,
            signed_checksums_url,
            checksums_sig_url,
            meta: Default::default(),
        }),
        other => return Err(GripError::UnknownAdapter(other.to_string())),
    };

    manifest.binaries.insert(binary_name.clone(), entry);
    manifest.save(&manifest_path)?;
    if !cfg.quiet {
        println!("Added '{}' to [binaries] in grip.toml", binary_name);
    }
    Ok(())
}

/// Remove a binary or library entry from `grip.toml`, `grip.lock`, and `.bin/`.
fn cmd_remove(
    name: String,
    library: bool,
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };
    let manifest_path = project_root.join("grip.toml");
    let lock_path = project_root.join("grip.lock");
    let bin_dir = project_root.join(".bin");
    let color = cfg.use_color_stdout();

    let mut manifest = Manifest::load(&manifest_path)?;
    let mut lock = LockFile::load(&lock_path)?;

    if library {
        if !manifest.libraries.contains_key(&name) {
            return Err(GripError::Other(format!(
                "'{name}' not found in [libraries] in grip.toml"
            )));
        }
        manifest.libraries.shift_remove(&name);
        lock.remove_library(&name);
    } else {
        if !manifest.binaries.contains_key(&name) {
            return Err(GripError::Other(format!(
                "'{name}' not found in [binaries] in grip.toml"
            )));
        }
        manifest.binaries.shift_remove(&name);

        // Collect extra binaries before removing the lock entry.
        let extra_binaries: Vec<String> = lock
            .get(&name)
            .map(|e| e.extra_binaries.clone())
            .unwrap_or_default();

        lock.remove(&name);

        // Remove the primary symlink / binary from .bin/ if present.
        let bin_path = bin_dir.join(&name);
        if bin_path.exists() || bin_path.symlink_metadata().is_ok() {
            std::fs::remove_file(&bin_path)?;
            if !cfg.quiet {
                let check = output::success_checkmark(color);
                println!("  {check}  removed .bin/{name}");
            }
        }

        // Remove any extra binary symlinks recorded in the lock entry.
        for extra in &extra_binaries {
            let extra_path = bin_dir.join(extra);
            if extra_path.exists() || extra_path.symlink_metadata().is_ok() {
                std::fs::remove_file(&extra_path)?;
                if !cfg.quiet {
                    let check = output::success_checkmark(color);
                    println!("  {check}  removed .bin/{extra}");
                }
            }
        }
    }

    manifest.save(&manifest_path)?;
    lock.save(&lock_path)?;

    if !cfg.quiet {
        let check = output::success_checkmark(color);
        let section = if library { "[libraries]" } else { "[binaries]" };
        println!("  {check}  removed '{name}' from {section} in grip.toml");
    }
    Ok(())
}

/// Run a command with the project's `.bin/` directory prepended to `PATH`.
fn cmd_run(args: Vec<String>, root: Option<std::path::PathBuf>) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).unwrap_or(cwd)
        }
    };
    let bin_dir = project_root.join(".bin");

    let path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.display(), path);

    let cmd = args
        .first()
        .ok_or_else(|| GripError::Other("no command given to `grip run`".into()))?;

    let status = std::process::Command::new(cmd)
        .args(&args[1..])
        .env("PATH", new_path)
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GripError::Other(format!(
                    "'{cmd}' not found — run `grip add {cmd}` then `grip sync` to install it"
                ))
            } else {
                GripError::Io(e)
            }
        })?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Detect the native package manager for the current platform.
///
/// Returns the adapter name (`"dnf"` or `"apt"`) if a supported package manager
/// is found on PATH. Returns an error if not on Linux or no package manager is
/// detected — callers should use `--source github` explicitly for non-native sources.
fn detect_default_source() -> Result<String, GripError> {
    let platform = platform::Platform::current();
    if platform.is_linux() {
        for cmd in &["dnf", "apt-get", "apt"] {
            if installer::which_exists(cmd) {
                return Ok(match *cmd {
                    "dnf" => "dnf",
                    _ => "apt",
                }
                .to_string());
            }
        }
    }
    Err(GripError::Other(
        "no native package manager found; use --source github to add a GitHub binary, \
         or --source url for direct-download binaries"
            .into(),
    ))
}

/// Print a formatted table of all entries in `grip.lock`.
/// With `--all`, also shows entries declared in `grip.toml` that are not yet installed.
fn cmd_list(root: Option<std::path::PathBuf>, all: bool, cfg: &OutputCfg) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };
    let lock_path = project_root.join("grip.lock");
    let lock = LockFile::load(&lock_path)?;
    let color = cfg.use_color_stdout();

    if all {
        let manifest_path = project_root.join("grip.toml");
        let manifest = Manifest::load(&manifest_path)?;

        if manifest.binaries.is_empty() && manifest.libraries.is_empty() {
            if !cfg.quiet {
                println!("No binaries or libraries declared in grip.toml.");
            }
            return Ok(());
        }

        if !manifest.binaries.is_empty() {
            if !cfg.quiet {
                println!();
                let header = output::dim(color, "Binaries (grip.toml)");
                println!("  {header}");
                println!();
                println!(
                    "  {:<18} {:<14} {:<10} {:<16} {}",
                    "NAME", "VERSION", "SOURCE", "INSTALLED AT", "STATUS"
                );
                println!("  {}", "-".repeat(80));
            }
            for (name, entry) in &manifest.binaries {
                let source = match entry {
                    BinaryEntry::Apt(_) => "apt",
                    BinaryEntry::Dnf(_) => "dnf",
                    BinaryEntry::Github(_) => "github",
                    BinaryEntry::Url(_) => "url",
                };
                if let Some(lock_entry) = lock.get(name) {
                    println!(
                        "  {:<18} {:<14} {:<10} {:<16} {}",
                        name,
                        lock_entry.version,
                        source,
                        lock_entry.installed_at.format("%Y-%m-%d %H:%M").to_string(),
                        output::green(color, "installed"),
                    );
                } else {
                    println!(
                        "  {:<18} {:<14} {:<10} {:<16} {}",
                        name,
                        "—",
                        source,
                        "—",
                        output::yellow(color, "not installed"),
                    );
                }
            }
        }

        if !manifest.libraries.is_empty() {
            if !cfg.quiet {
                println!();
                let header = output::dim(color, "Libraries (grip.toml)");
                println!("  {header}");
                println!();
                println!(
                    "  {:<18} {:<14} {:<10} {:<16} {}",
                    "NAME", "VERSION", "SOURCE", "INSTALLED AT", "STATUS"
                );
                println!("  {}", "-".repeat(80));
            }
            for (name, entry) in &manifest.libraries {
                let source = match entry {
                    LibraryEntry::Apt(_) => "apt",
                    LibraryEntry::Dnf(_) => "dnf",
                };
                if let Some(lock_entry) = lock.get_library(name) {
                    println!(
                        "  {:<18} {:<14} {:<10} {:<16} {}",
                        name,
                        lock_entry.version,
                        source,
                        lock_entry.installed_at.format("%Y-%m-%d %H:%M").to_string(),
                        output::green(color, "installed"),
                    );
                } else {
                    println!(
                        "  {:<18} {:<14} {:<10} {:<16} {}",
                        name,
                        "—",
                        source,
                        "—",
                        output::yellow(color, "not installed"),
                    );
                }
            }
        }

        return Ok(());
    }

    // Default: lock-only view.
    if lock.entries.is_empty() && lock.library_entries.is_empty() {
        if !cfg.quiet {
            println!("No binaries or libraries installed yet.");
            println!(
                "hint: {}",
                output::dim(
                    color,
                    "Run `grip sync` to install everything from grip.toml."
                )
            );
        }
        return Ok(());
    }

    if !lock.entries.is_empty() {
        if !cfg.quiet {
            println!();
            let header = output::dim(color, "Installed binaries (from grip.lock)");
            println!("  {header}");
            println!();
            println!(
                "  {:<18} {:<14} {:<10} {}",
                "NAME", "VERSION", "SOURCE", "INSTALLED AT"
            );
            println!("  {}", "-".repeat(66));
        }
        for e in &lock.entries {
            println!(
                "  {:<18} {:<14} {:<10} {}",
                e.name,
                e.version,
                e.source,
                e.installed_at.format("%Y-%m-%d %H:%M")
            );
        }
    }
    if !lock.library_entries.is_empty() {
        if !cfg.quiet {
            println!();
            let header = output::dim(color, "Installed libraries (from grip.lock)");
            println!("  {header}");
            println!();
            println!(
                "  {:<18} {:<14} {:<10} {}",
                "NAME", "VERSION", "SOURCE", "INSTALLED AT"
            );
            println!("  {}", "-".repeat(66));
        }
        for e in &lock.library_entries {
            println!(
                "  {:<18} {:<14} {:<10} {}",
                e.name,
                e.version,
                e.source,
                e.installed_at.format("%Y-%m-%d %H:%M")
            );
        }
    }
    Ok(())
}

/// Fetch the latest available version for every manifest entry concurrently and print a comparison
/// table against what is currently installed (from `grip.lock`).
async fn cmd_outdated(
    tag: Option<String>,
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };
    let manifest = Manifest::load(&project_root.join("grip.toml"))?;
    let lock = LockFile::load(&project_root.join("grip.lock"))?;
    let color = cfg.use_color_stdout();

    if manifest.binaries.is_empty() {
        if !cfg.quiet {
            println!("No binaries declared in grip.toml.");
        }
        return Ok(());
    }

    let platform = crate::platform::Platform::current();
    let entries: Vec<(String, config::manifest::BinaryEntry)> = manifest
        .binaries
        .iter()
        .filter(|(_, e)| e.meta().matches_platform(platform.os_str()))
        .filter(|(_, e)| tag.as_deref().map(|t| e.meta().has_tag(t)).unwrap_or(true))
        .map(|(n, e)| (n.clone(), e.clone()))
        .collect();

    if entries.is_empty() {
        if !cfg.quiet {
            println!("No binaries matched (platform or --tag filter).");
        }
        return Ok(());
    }

    if !cfg.quiet {
        let header = output::dim(color, "Checking for updates…");
        println!("\n  {header}\n");
    }

    let client = reqwest::Client::builder()
        .user_agent("grip/0.1")
        .build()
        .map_err(GripError::Http)?;

    // Resolve all latest versions concurrently.
    let mut futs: FuturesUnordered<_> = entries
        .iter()
        .map(|(name, entry)| {
            let name = name.clone();
            let entry = entry.clone();
            let client = client.clone();
            async move {
                let adapter = crate::adapters::get_adapter(&entry, None);
                let result = adapter.resolve_latest(&entry, &client).await;
                (name, result)
            }
        })
        .collect();

    let mut latest_map: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    while let Some((name, result)) = futs.next().await {
        latest_map.insert(name, result.ok());
    }

    // Column widths.
    let name_w = entries
        .iter()
        .map(|(n, _)| n.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let col_w = 14usize;

    if !cfg.quiet {
        println!(
            "  {:<name_w$}  {:<col_w$}  {:<col_w$}  STATUS",
            "BINARY", "INSTALLED", "LATEST",
        );
        println!(
            "  {}",
            output::dim(color, &"─".repeat(name_w + col_w * 2 + 16))
        );

        let mut n_outdated = 0usize;
        let mut n_current = 0usize;
        let mut n_unknown = 0usize;

        for (name, _) in &entries {
            let installed: Option<String> = lock.get(name).map(|e| e.version.clone());
            let latest: Option<&str> = latest_map.get(name).and_then(|o| o.as_deref());

            let installed_display = installed.as_deref().unwrap_or("—");

            let (latest_display, status) = match latest {
                None => {
                    n_unknown += 1;
                    ("—".to_string(), output::dim(color, "—"))
                }
                Some(v) => match &installed {
                    None => (v.to_string(), output::yellow(color, "not installed")),
                    Some(inst) => {
                        let norm = |s: &str| s.trim_start_matches('v').to_lowercase();
                        if norm(inst) == norm(v) {
                            n_current += 1;
                            (v.to_string(), output::green(color, "up to date"))
                        } else {
                            n_outdated += 1;
                            (v.to_string(), output::yellow(color, "outdated"))
                        }
                    }
                },
            };

            println!(
                "  {:<name_w$}  {:<col_w$}  {:<col_w$}  {}",
                name, installed_display, latest_display, status,
            );
        }

        println!();
        if n_outdated == 0 && n_unknown == 0 {
            println!(
                "  {}",
                output::green(color, &format!("All {n_current} binaries are up to date"))
            );
        } else {
            let mut parts: Vec<String> = Vec::new();
            if n_outdated > 0 {
                parts.push(output::yellow(color, &format!("{n_outdated} outdated")));
            }
            if n_current > 0 {
                parts.push(output::green(color, &format!("{n_current} up to date")));
            }
            if n_unknown > 0 {
                parts.push(output::dim(color, &format!("{n_unknown} unknown")));
            }
            println!("  {}", parts.join(", "));
            if n_outdated > 0 {
                println!(
                    "  {}",
                    output::dim(
                        color,
                        "hint: run `grip update <name>` to upgrade a single binary"
                    )
                );
            }
        }

        // ── Libraries section ───────────────────────────────────────────────
        let lib_entries: Vec<(&String, &config::manifest::LibraryEntry)> = manifest
            .libraries
            .iter()
            .filter(|(_, e)| e.meta().matches_platform(platform.os_str()))
            .filter(|(_, e)| tag.as_deref().map(|t| e.meta().has_tag(t)).unwrap_or(true))
            .collect();

        if !lib_entries.is_empty() {
            println!();
            let lib_header = output::dim(color, "Libraries");
            println!("  {lib_header}");
            println!();
            println!(
                "  {:<name_w$}  {:<col_w$}  {:<col_w$}  STATUS",
                "LIBRARY", "LOCKED", "SYSTEM",
            );
            println!(
                "  {}",
                output::dim(color, &"─".repeat(name_w + col_w * 2 + 16))
            );

            for (name, entry) in &lib_entries {
                let locked_ver = lock
                    .get_library(name)
                    .map(|e| e.version.as_str())
                    .unwrap_or("—");

                let system_ver: Option<String> = match entry {
                    config::manifest::LibraryEntry::Apt(a) => {
                        crate::adapters::apt::installed_version(&a.package)
                    }
                    config::manifest::LibraryEntry::Dnf(d) => {
                        crate::adapters::dnf::installed_version(&d.package)
                    }
                };

                let (system_display, status) = match &system_ver {
                    None => ("—".to_string(), output::yellow(color, "not installed")),
                    Some(v) => {
                        let norm = |s: &str| s.trim_start_matches('v').to_lowercase();
                        if norm(locked_ver) == norm(v) {
                            (v.clone(), output::green(color, "in sync"))
                        } else {
                            (v.clone(), output::yellow(color, "drifted"))
                        }
                    }
                };

                println!(
                    "  {:<name_w$}  {:<col_w$}  {:<col_w$}  {}",
                    name, locked_ver, system_display, status,
                );
            }
        }
    } else {
        // Quiet / machine-readable: tab-separated lines — name, installed, latest, status.
        let norm = |s: &str| s.trim_start_matches('v').to_lowercase();
        for (name, _) in &entries {
            let installed = lock.get(name).map(|e| e.version.as_str()).unwrap_or("-");
            let latest = latest_map.get(name).and_then(|o| o.as_deref()).unwrap_or("-");
            let status = if latest == "-" {
                "unknown"
            } else if installed == "-" {
                "not-installed"
            } else if norm(installed) == norm(latest) {
                "up-to-date"
            } else {
                "outdated"
            };
            println!("{name}\t{installed}\t{latest}\t{status}");
        }

        let lib_entries: Vec<(&String, &config::manifest::LibraryEntry)> = manifest
            .libraries
            .iter()
            .filter(|(_, e)| e.meta().matches_platform(platform.os_str()))
            .filter(|(_, e)| tag.as_deref().map(|t| e.meta().has_tag(t)).unwrap_or(true))
            .collect();

        for (name, entry) in &lib_entries {
            let locked = lock.get_library(name).map(|e| e.version.as_str()).unwrap_or("-");
            let system_ver: Option<String> = match entry {
                config::manifest::LibraryEntry::Apt(a) => {
                    crate::adapters::apt::installed_version(&a.package)
                }
                config::manifest::LibraryEntry::Dnf(d) => {
                    crate::adapters::dnf::installed_version(&d.package)
                }
            };
            let system = system_ver.as_deref().unwrap_or("-");
            let status = match &system_ver {
                None => "not-installed",
                Some(v) if norm(locked) == norm(v) => "in-sync",
                _ => "drifted",
            };
            println!("{name}\t{locked}\t{system}\t{status}");
        }
    }

    Ok(())
}

/// Check consistency between `grip.toml`, `grip.lock`, and `.bin/`.
fn cmd_lock(
    action: LockAction,
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    match action {
        LockAction::Verify => {
            let color = cfg.use_color_stdout();
            let r = lock_verify::run_lock_verify(root)?;

            if !cfg.quiet {
                println!();
                let header = output::dim(color, "grip lock verify");
                println!("  {header}");
                println!();

                for name in &r.verified {
                    let mark = output::success_checkmark(color);
                    println!("  {mark}  {name}");
                }
                for name in &r.no_checksum {
                    let g = output::warn_glyph(color);
                    let note = output::dim(color, "(no sha256 in lock — cannot verify)");
                    println!("  {g}  {name}  {note}");
                }
                for (name, msg) in &r.failed {
                    let x = output::fail_glyph(color);
                    eprintln!("  {x}  {name}: {msg}");
                }

                let n_ok = r.verified.len();
                let n_skip = r.no_checksum.len();
                let n_fail = r.failed.len();

                let summary = if n_fail > 0 {
                    output::red(
                        color,
                        &format!("{n_fail} mismatch(es) detected — possible tampering!"),
                    )
                } else if n_ok == 0 && n_skip == 0 {
                    output::dim(color, "No binaries in grip.lock")
                } else if n_skip > 0 {
                    format!(
                        "{}  ({} verified, {} without sha256)",
                        output::green(color, "OK"),
                        n_ok,
                        n_skip,
                    )
                } else {
                    output::green(color, &format!("All {n_ok} binaries verified"))
                };
                println!("\n  {summary}");
                println!();
            } else {
                for (name, msg) in &r.failed {
                    eprintln!("error: {name}: {msg}");
                }
            }

            if !r.failed.is_empty() {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

/// Pin all unpinned entries in `grip.toml` to their currently installed versions from `grip.lock`.
fn cmd_pin(
    dry_run: bool,
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };
    let manifest_path = project_root.join("grip.toml");
    let lock_path = project_root.join("grip.lock");
    let color = cfg.use_color_stdout();

    let mut manifest = Manifest::load(&manifest_path)?;
    let lock = LockFile::load(&lock_path)?;

    let mut pinned: Vec<(String, String)> = Vec::new();
    let mut not_installed: Vec<String> = Vec::new();

    // Collect pins for binaries first (can't iterate + mutate at same time).
    let bin_pins: Vec<(String, Option<String>)> = manifest
        .binaries
        .iter()
        .filter(|(_, entry)| !entry.is_version_pinned())
        .map(|(name, _)| {
            let ver = lock.get(name).map(|e| e.version.clone());
            (name.clone(), ver)
        })
        .collect();

    for (name, ver) in bin_pins {
        match ver {
            Some(version) => {
                pinned.push((name.clone(), version.clone()));
                if !dry_run {
                    if let Some(entry) = manifest.binaries.get_mut(&name) {
                        *entry = entry.pin_version(&version);
                    }
                }
            }
            None => not_installed.push(name),
        }
    }

    // Collect pins for libraries.
    let lib_pins: Vec<(String, Option<String>)> = manifest
        .libraries
        .iter()
        .filter(|(_, entry)| match entry {
            LibraryEntry::Apt(a) => a.version.is_none(),
            LibraryEntry::Dnf(d) => d.version.is_none(),
        })
        .map(|(name, _)| {
            let ver = lock.get_library(name).map(|e| e.version.clone());
            (name.clone(), ver)
        })
        .collect();

    for (name, ver) in lib_pins {
        match ver {
            Some(version) => {
                pinned.push((name.clone(), version.clone()));
                if !dry_run {
                    if let Some(entry) = manifest.libraries.get_mut(&name) {
                        match entry {
                            LibraryEntry::Apt(a) => a.version = Some(version),
                            LibraryEntry::Dnf(d) => d.version = Some(version),
                        }
                    }
                }
            }
            None => not_installed.push(name),
        }
    }

    if !dry_run && !pinned.is_empty() {
        manifest.save(&manifest_path)?;
    }

    if cfg.quiet {
        for (name, version) in &pinned {
            println!("{name} {version}");
        }
        for name in &not_installed {
            eprintln!("warning: {name}: not installed, skipped");
        }
        return Ok(());
    }

    if pinned.is_empty() && not_installed.is_empty() {
        println!("All entries are already pinned.");
        return Ok(());
    }

    if !pinned.is_empty() {
        println!();
        let header = if dry_run {
            output::dim(color, "Would pin (dry run)")
        } else {
            output::dim(color, "Pinned")
        };
        println!("  {header}");
        println!();
        for (name, version) in &pinned {
            let mark = output::success_checkmark(color);
            println!("  {mark}  {name}  →  {version}");
        }
    }

    if !not_installed.is_empty() {
        println!();
        let warn_header = output::dim(color, "Not installed — skipped");
        println!("  {warn_header}");
        println!();
        for name in &not_installed {
            let g = output::warn_glyph(color);
            println!("  {g}  {name}");
        }
        println!(
            "\n  hint: {}",
            output::dim(color, "run `grip sync` to install, then `grip pin` to pin")
        );
    }

    if !pinned.is_empty() {
        let n = pinned.len();
        let summary = if dry_run {
            output::dim(
                color,
                &format!(
                    "{n} entr{} would be pinned (re-run without --dry-run to apply)",
                    if n == 1 { "y" } else { "ies" }
                ),
            )
        } else {
            output::green(
                color,
                &format!(
                    "{n} entr{} pinned in grip.toml",
                    if n == 1 { "y" } else { "ies" }
                ),
            )
        };
        println!("\n  {summary}");
    }

    Ok(())
}

/// Print shell code that adds the project's `.bin/` directory to `PATH`.
/// Meant to be captured by `eval "$(grip env)"` (bash/zsh) or `grip env --shell fish | source`.
fn cmd_env(
    shell: Option<String>,
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).unwrap_or(cwd)
        }
    };
    let bin_dir = project_root
        .canonicalize()
        .unwrap_or(project_root)
        .join(".bin");
    let bin_dir_str = bin_dir.display().to_string();

    let shell_name = shell.unwrap_or_else(|| {
        std::env::var("SHELL")
            .unwrap_or_default()
            .rsplit('/')
            .next()
            .unwrap_or("sh")
            .to_string()
    });

    let is_tty = std::io::stdout().is_terminal();

    if shell_name == "fish" {
        println!("set -gx PATH \"{bin_dir_str}\" $PATH;");
        if is_tty && !cfg.quiet {
            let color = cfg.use_color_stderr();
            eprintln!(
                "  {}",
                output::dim(
                    color,
                    "Add to ~/.config/fish/config.fish:  grip env --shell fish | source"
                )
            );
        }
    } else {
        println!("export PATH=\"{bin_dir_str}:$PATH\";");
        if is_tty && !cfg.quiet {
            let color = cfg.use_color_stderr();
            eprintln!(
                "  {}",
                output::dim(color, "Add to ~/.bashrc or ~/.zshrc:  eval \"$(grip env)\"")
            );
        }
    }

    Ok(())
}

/// Re-install one or all binaries/libraries from the manifest and update their lock entries.
async fn cmd_update(
    name: Option<String>,
    all: bool,
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };

    match (name, all) {
        (Some(n), false) => cmd_update_one(n, &project_root, cfg).await,
        (None, true) => cmd_update_all(&project_root, cfg).await,
        (Some(_), true) => Err(GripError::Other(
            "pass either a name or --all, not both".into(),
        )),
        (None, false) => Err(GripError::Other(
            "specify a binary name or pass --all to update everything".into(),
        )),
    }
}

async fn cmd_update_one(
    name: String,
    project_root: &std::path::Path,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let manifest_path = project_root.join("grip.toml");
    let lock_path = project_root.join("grip.lock");
    let bin_dir = crate::bin_dir::ensure_bin_dir(project_root)?;

    let manifest = Manifest::load(&manifest_path)?;
    let mut lock = LockFile::load(&lock_path)?;

    let color_err = cfg.use_color_stderr();

    let client = reqwest::Client::builder()
        .user_agent("grip/0.1")
        .build()
        .map_err(GripError::Http)?;

    let pb = if cfg.quiet {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new_spinner();
        let tpl = output::install_spinner_template(color_err);
        pb.set_style(
            ProgressStyle::with_template(tpl)
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]),
        );
        pb.set_message(format!("{name}  resolving..."));
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    };

    // Check libraries first, then binaries.
    if let Some(lib_entry) = manifest.libraries.get(&name).cloned() {
        let old_version = lock.get_library(&name).map(|e| e.version.clone());
        let lock_entry = match &lib_entry {
            LibraryEntry::Apt(a) => {
                crate::adapters::apt::install_apt_library(&name, a, &client, pb, color_err).await?
            }
            LibraryEntry::Dnf(d) => {
                crate::adapters::dnf::install_dnf_library(&name, d, &client, pb, color_err).await?
            }
        };
        if !cfg.quiet {
            let check = output::success_checkmark(color_err);
            if old_version.as_deref() == Some(lock_entry.version.as_str()) {
                println!(
                    "\n  {check}  library {name} is already at the latest version ({})",
                    lock_entry.version
                );
            } else {
                println!(
                    "\n  {check}  updated library {name} to {}",
                    lock_entry.version
                );
            }
        }
        lock.upsert_library(lock_entry);
        lock.save(&lock_path)?;
        return Ok(());
    }

    let entry = manifest
        .binaries
        .get(&name)
        .ok_or_else(|| GripError::EntryNotFound(name.clone()))?
        .clone();

    let old_version = lock.get(&name).map(|e| e.version.clone());

    let update_cache = match cache::Cache::open() {
        None => None,
        Some(Ok(c)) => Some(std::sync::Arc::new(c)),
        Some(Err(_)) => None,
    };
    let adapter = crate::adapters::get_adapter(&entry, update_cache);
    let lock_entry = adapter
        .install(&name, &entry, &bin_dir, &client, pb, color_err)
        .await?;
    if !cfg.quiet {
        let check = output::success_checkmark(color_err);
        if old_version.as_deref() == Some(lock_entry.version.as_str()) {
            println!(
                "\n  {check}  {name} is already at the latest version ({})",
                lock_entry.version
            );
        } else {
            println!("\n  {check}  updated {name} to {}", lock_entry.version);
        }
    }
    lock.upsert(lock_entry);
    lock.save(&lock_path)?;
    Ok(())
}

async fn cmd_update_all(project_root: &std::path::Path, cfg: &OutputCfg) -> Result<(), GripError> {
    let manifest_path = project_root.join("grip.toml");
    let lock_path = project_root.join("grip.lock");
    let bin_dir = crate::bin_dir::ensure_bin_dir(project_root)?;

    let manifest = Manifest::load(&manifest_path)?;
    let mut lock = LockFile::load(&lock_path)?;

    if manifest.binaries.is_empty() && manifest.libraries.is_empty() {
        if !cfg.quiet {
            println!("Nothing declared in grip.toml.");
        }
        return Ok(());
    }

    let color_err = cfg.use_color_stderr();
    let color_out = cfg.use_color_stdout();

    let client = std::sync::Arc::new(
        reqwest::Client::builder()
            .user_agent("grip/0.1")
            .build()
            .map_err(GripError::Http)?,
    );
    let update_cache = match cache::Cache::open() {
        None => None,
        Some(Ok(c)) => Some(std::sync::Arc::new(c)),
        Some(Err(_)) => None,
    };
    let bin_dir = std::sync::Arc::new(bin_dir);

    // Snapshot old versions so we can report "already at latest" after re-install.
    let old_bin_versions: std::collections::HashMap<String, String> = manifest
        .binaries
        .keys()
        .filter_map(|n| lock.get(n).map(|e| (n.clone(), e.version.clone())))
        .collect();

    // --- binaries (concurrent) ---
    let mut bin_futs: FuturesUnordered<_> = manifest
        .binaries
        .iter()
        .map(|(name, entry)| {
            let name = name.clone();
            let entry = entry.clone();
            let client = client.clone();
            let cache = update_cache.clone();
            let bin_dir = bin_dir.clone();
            let pb = if cfg.quiet {
                ProgressBar::hidden()
            } else {
                let pb = ProgressBar::new_spinner();
                let tpl = output::install_spinner_template(color_err);
                pb.set_style(
                    ProgressStyle::with_template(tpl)
                        .unwrap()
                        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]),
                );
                pb.set_message(format!("{name}  updating..."));
                pb.enable_steady_tick(Duration::from_millis(80));
                pb
            };
            async move {
                let adapter = crate::adapters::get_adapter(&entry, cache);
                let result = adapter
                    .install(&name, &entry, &bin_dir, &client, pb, color_err)
                    .await;
                (name, result)
            }
        })
        .collect();

    let mut updated: Vec<String> = Vec::new();
    let mut already_current: Vec<String> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();

    while let Some((name, result)) = bin_futs.next().await {
        match result {
            Ok(lock_entry) => {
                if !cfg.quiet {
                    let check = output::success_checkmark(color_err);
                    let already = old_bin_versions
                        .get(&name)
                        .map(|v| v == &lock_entry.version)
                        .unwrap_or(false);
                    if already {
                        eprintln!("  {check}  {name}  {} (already at latest)", lock_entry.version);
                    } else {
                        eprintln!("  {check}  {name}  {}", lock_entry.version);
                    }
                }
                let already = old_bin_versions
                    .get(&name)
                    .map(|v| v == &lock_entry.version)
                    .unwrap_or(false);
                lock.upsert(lock_entry);
                if already {
                    already_current.push(name);
                } else {
                    updated.push(name);
                }
            }
            Err(e) => {
                if !cfg.quiet {
                    let x = output::fail_glyph(color_err);
                    eprintln!("  {x}  {name}: {e}");
                }
                failed.push((name, e.to_string()));
            }
        }
    }

    // --- libraries (sequential, need privilege) ---
    for (name, lib_entry) in &manifest.libraries {
        let old_lib_version = lock.get_library(name).map(|e| e.version.clone());
        let pb = if cfg.quiet {
            ProgressBar::hidden()
        } else {
            let pb = ProgressBar::new_spinner();
            let tpl = output::install_spinner_template(color_err);
            pb.set_style(
                ProgressStyle::with_template(tpl)
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]),
            );
            pb.set_message(format!("{name}  updating..."));
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        };
        let result = match lib_entry {
            LibraryEntry::Apt(a) => {
                crate::adapters::apt::install_apt_library(name, a, &*client, pb, color_err).await
            }
            LibraryEntry::Dnf(d) => {
                crate::adapters::dnf::install_dnf_library(name, d, &*client, pb, color_err).await
            }
        };
        match result {
            Ok(lock_entry) => {
                let already = old_lib_version.as_deref() == Some(lock_entry.version.as_str());
                if !cfg.quiet {
                    let check = output::success_checkmark(color_err);
                    if already {
                        eprintln!(
                            "  {check}  {name}  {} (library, already at latest)",
                            lock_entry.version
                        );
                    } else {
                        eprintln!("  {check}  {name}  {} (library)", lock_entry.version);
                    }
                }
                lock.upsert_library(lock_entry);
                if already {
                    already_current.push(name.clone());
                } else {
                    updated.push(name.clone());
                }
            }
            Err(e) => {
                if !cfg.quiet {
                    let x = output::fail_glyph(color_err);
                    eprintln!("  {x}  {name}: {e}");
                }
                failed.push((name.clone(), e.to_string()));
            }
        }
    }

    lock.save(&lock_path)?;

    if !cfg.quiet {
        println!();
        let n_updated = updated.len();
        let n_current = already_current.len();
        let n_fail = failed.len();
        if n_fail == 0 {
            let mut parts = Vec::new();
            if n_updated > 0 {
                parts.push(output::green(color_out, &format!("{n_updated} updated")));
            }
            if n_current > 0 {
                parts.push(format!("{n_current} already at latest"));
            }
            if parts.is_empty() {
                parts.push("nothing to update".to_string());
            }
            println!("  {}", parts.join(", "));
        } else {
            let mut parts = Vec::new();
            if n_updated > 0 {
                parts.push(output::green(color_out, &format!("{n_updated} updated")));
            }
            if n_current > 0 {
                parts.push(format!("{n_current} already at latest"));
            }
            parts.push(output::red(color_out, &format!("{n_fail} failed")));
            println!("  {}", parts.join(", "));
        }
    } else {
        for (name, err) in &failed {
            eprintln!("error: {name}: {err}");
        }
    }

    if !failed.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_cache(action: CacheAction, cfg: &OutputCfg) -> Result<(), GripError> {
    let color = cfg.use_color_stdout();
    match cache::Cache::open() {
        None => {
            if !cfg.quiet {
                println!("Cache is disabled (GRIP_CACHE_DIR is set to empty string).");
            }
            return Ok(());
        }
        Some(Err(e)) => return Err(e),
        Some(Ok(c)) => match action {
            CacheAction::Clean => {
                let (count, bytes) = c.clean()?;
                if cfg.quiet {
                    println!("{count} {bytes}");
                } else {
                    let freed = format_bytes(bytes);
                    println!(
                        "  {}  Removed {count} file{} ({freed})",
                        output::success_checkmark(color),
                        if count == 1 { "" } else { "s" }
                    );
                }
            }
            CacheAction::Info => {
                let (count, bytes) = c.stats();
                if cfg.quiet {
                    println!("{count} {bytes}");
                } else {
                    let size = format_bytes(bytes);
                    println!("  Cache entries : {count}");
                    println!("  Total size    : {size}");
                }
            }
        },
    }
    Ok(())
}

fn cmd_export(
    format: &str,
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };
    let manifest = Manifest::load(&project_root.join("grip.toml"))?;
    let lock = LockFile::load(&project_root.join("grip.lock"))?;

    // Collect apt/dnf package specs (binaries + libraries combined)
    let mut apt_pkgs: Vec<String> = Vec::new();
    let mut dnf_pkgs: Vec<String> = Vec::new();
    // (name, url) for curl-based installs
    let mut curl_installs: Vec<(String, String)> = Vec::new();

    for (name, entry) in &manifest.binaries {
        match entry {
            BinaryEntry::Apt(a) => {
                let ver = lock
                    .get(name)
                    .map(|le| le.version.clone())
                    .or_else(|| a.version.clone());
                let spec = match ver {
                    Some(v) if !v.is_empty() => format!("{}={}", a.package, v),
                    _ => a.package.clone(),
                };
                apt_pkgs.push(spec);
            }
            BinaryEntry::Dnf(d) => {
                let ver = lock
                    .get(name)
                    .map(|le| le.version.clone())
                    .or_else(|| d.version.clone());
                let spec = match ver {
                    Some(v) if !v.is_empty() => format!("{}-{}", d.package, v),
                    _ => d.package.clone(),
                };
                dnf_pkgs.push(spec);
            }
            BinaryEntry::Github(g) => {
                let url = lock
                    .get(name)
                    .and_then(|le| le.url.clone())
                    .unwrap_or_else(|| {
                        let ver = g.version.as_deref().unwrap_or("latest");
                        format!(
                            "https://github.com/{}/releases/download/v{}/{}",
                            g.repo, ver, name
                        )
                    });
                curl_installs.push((name.clone(), url));
            }
            BinaryEntry::Url(u) => {
                curl_installs.push((name.clone(), u.url.clone()));
            }
        }
    }

    for (name, entry) in &manifest.libraries {
        match entry {
            LibraryEntry::Apt(a) => {
                let ver = lock
                    .get_library(name)
                    .map(|le| le.version.clone())
                    .or_else(|| a.version.clone());
                let spec = match ver {
                    Some(v) if !v.is_empty() => format!("{}={}", a.package, v),
                    _ => a.package.clone(),
                };
                apt_pkgs.push(spec);
            }
            LibraryEntry::Dnf(d) => {
                let ver = lock
                    .get_library(name)
                    .map(|le| le.version.clone())
                    .or_else(|| d.version.clone());
                let spec = match ver {
                    Some(v) if !v.is_empty() => format!("{}-{}", d.package, v),
                    _ => d.package.clone(),
                };
                dnf_pkgs.push(spec);
            }
        }
    }

    // Issue 7: sort for reproducible, diff-friendly output.
    apt_pkgs.sort();
    dnf_pkgs.sort();

    match format {
        "dockerfile" => {
            println!("# Generated by grip export --format dockerfile");
            if !apt_pkgs.is_empty() {
                let pkgs = apt_pkgs.join(" \\\n    ");
                println!("RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \\");
                println!("    {pkgs} \\");
                println!("    && rm -rf /var/lib/apt/lists/*");
            }
            if !dnf_pkgs.is_empty() {
                let pkgs = dnf_pkgs.join(" \\\n    ");
                println!("RUN dnf install -y \\");
                println!("    {pkgs} \\");
                println!("    && dnf clean all");
            }
            for (name, url) in &curl_installs {
                println!("RUN curl -fsSL -o /usr/local/bin/{name} \\\n    \"{url}\" \\");
                println!("    && chmod +x /usr/local/bin/{name}");
            }
        }
        "makefile" => {
            println!("# Generated by grip export --format makefile");
            println!(".PHONY: install-deps");
            println!("install-deps:");
            if !apt_pkgs.is_empty() {
                println!("\tapt-get update");
                let pkgs = apt_pkgs.join(" \\\n\t\t");
                println!("\tDEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \\\n\t\t{pkgs}");
            }
            if !dnf_pkgs.is_empty() {
                let pkgs = dnf_pkgs.join(" \\\n\t\t");
                println!("\tdnf install -y \\\n\t\t{pkgs}");
                println!("\tdnf clean all");
            }
            for (name, url) in &curl_installs {
                println!("\tcurl -fsSL -o /usr/local/bin/{name} \"{url}\" && chmod +x /usr/local/bin/{name}");
            }
        }
        _ => {
            // default: shell script
            println!("#!/bin/sh");
            println!("# Generated by grip export --format shell");
            println!("set -eu");
            if !apt_pkgs.is_empty() {
                let pkgs = apt_pkgs.join(" \\\n  ");
                println!("apt-get update");
                println!("DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \\");
                println!("  {pkgs}");
            }
            if !dnf_pkgs.is_empty() {
                let pkgs = dnf_pkgs.join(" \\\n  ");
                println!("dnf install -y \\");
                println!("  {pkgs}");
            }
            for (name, url) in &curl_installs {
                println!("curl -fsSL -o /usr/local/bin/{name} \"{url}\" && chmod +x /usr/local/bin/{name}");
            }
        }
    }

    let _ = cfg;
    Ok(())
}

/// Print the result of a `sync` or `add` install pass (quiet and verbose paths).
fn print_install_result(
    result: &installer::InstallResult,
    cfg: &OutputCfg,
    color_out: bool,
    color_err: bool,
    elapsed: f64,
) {
    if cfg.quiet {
        for (name, err) in &result.failed {
            eprintln!("error: {name}: {err}");
        }
        return;
    }
    for (name, detected) in &result.binary_overrides {
        let check = output::success_checkmark(color_err);
        eprintln!(
            "  {check}  {name}: auto-detected binary `{detected}`; updated grip.toml"
        );
    }
    for (name, extras) in &result.extra_binary_overrides {
        let check = output::success_checkmark(color_err);
        let list = extras.join(", ");
        eprintln!(
            "  {check}  {name}: auto-detected extra binaries [{list}]; updated grip.toml"
        );
    }
    for (name, err) in &result.warned {
        let g = output::warn_glyph(color_err);
        eprintln!("  {g}  {name}: {err}");
    }
    for (name, err) in &result.failed {
        let x = output::fail_glyph(color_err);
        eprintln!("  {x}  {name}: {err}");
    }
    let n_installed = result.installed.len();
    let n_skipped = result.skipped.len();
    let n_failed = result.failed.len() + result.warned.len();
    if n_installed == 0 && n_failed == 0 {
        let dim = output::dim(color_out, "All up to date");
        println!("\n  {dim}  ({n_skipped} skipped, {elapsed:.1}s)");
    } else {
        let mut parts: Vec<String> = Vec::new();
        if n_installed > 0 {
            parts.push(output::green(color_out, &format!("{n_installed} installed")));
        }
        if n_skipped > 0 {
            parts.push(output::dim(color_out, &format!("{n_skipped} skipped")));
        }
        if n_failed > 0 {
            parts.push(output::red(color_out, &format!("{n_failed} failed")));
        }
        println!("\n  {}  ({elapsed:.1}s)", parts.join(", "));
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use output::ColorWhen;
    use tempfile::TempDir;

    fn silent_cfg() -> OutputCfg {
        OutputCfg {
            quiet: true,
            verbose: false,
            color_when: ColorWhen::Never,
        }
    }

    // ── parse_name_at_version ─────────────────────────────────────────────────

    #[test]
    fn parse_plain_name() {
        let (stem, ver) = parse_name_at_version("jq".into());
        assert_eq!(stem, "jq");
        assert!(ver.is_none());
    }

    #[test]
    fn parse_name_with_version() {
        let (stem, ver) = parse_name_at_version("jq@1.7.1".into());
        assert_eq!(stem, "jq");
        assert_eq!(ver.as_deref(), Some("1.7.1"));
    }

    #[test]
    fn parse_last_at_wins() {
        let (stem, ver) = parse_name_at_version("org@example@1.0.0".into());
        assert_eq!(stem, "org@example");
        assert_eq!(ver.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_trailing_at_returns_no_version() {
        // "@" at the end with nothing after — version is None and the "@" is stripped from stem.
        let (stem, ver) = parse_name_at_version("jq@".into());
        assert_eq!(stem, "jq");
        assert!(ver.is_none());
    }

    // ── cmd_remove ────────────────────────────────────────────────────────────

    #[test]
    fn cmd_remove_also_removes_extra_binaries() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Minimal grip.toml with one dnf entry.
        std::fs::write(
            root.join("grip.toml"),
            r#"[binaries]
chromium = { source = "dnf", package = "chromium-browser" }
"#,
        )
        .unwrap();

        // grip.lock with extra_binaries recorded.
        std::fs::write(
            root.join("grip.lock"),
            r#"[[binary]]
name = "chromium"
version = "1.0.0"
source = "dnf"
sha256 = "aabbcc"
installed_at = "2026-01-01T00:00:00Z"
extra_binaries = ["chromium-browser", "chromedriver"]
"#,
        )
        .unwrap();

        // Create the .bin/ directory with fake symlinks.
        let bin_dir = root.join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        for name in &["chromium", "chromium-browser", "chromedriver"] {
            std::fs::write(bin_dir.join(name), "fake").unwrap();
        }

        cmd_remove(
            "chromium".into(),
            false,
            Some(root.to_path_buf()),
            &silent_cfg(),
        )
        .unwrap();

        assert!(
            !bin_dir.join("chromium").exists(),
            "primary binary should be removed"
        );
        assert!(
            !bin_dir.join("chromium-browser").exists(),
            "extra binary chromium-browser should be removed"
        );
        assert!(
            !bin_dir.join("chromedriver").exists(),
            "extra binary chromedriver should be removed"
        );
    }

    // ── format_bytes ──────────────────────────────────────────────────────────

    #[test]
    fn format_bytes_under_1024() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn format_bytes_kib() {
        assert_eq!(format_bytes(2048), "2.0 KiB");
    }

    #[test]
    fn format_bytes_mib() {
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MiB");
    }
}
