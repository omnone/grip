//! Entry point and CLI command implementations for `grip`.

mod adapters;
mod bin_dir;
mod checker;
mod checksum;
mod config;
mod error;
mod installer;
mod output;
mod cli;
mod platform;

use std::io::{IsTerminal, Write};
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use cli::{Cli, Commands};
use config::manifest::{
    find_manifest_dir, AptEntry, BinaryEntry, DnfEntry, GithubEntry, LibAptEntry,
    LibDnfEntry, LibraryEntry, Manifest, ShellEntry, UrlEntry,
};
use config::lockfile::LockFile;
use error::GripError;
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
        } => {
            cmd_add(
                name,
                source,
                version,
                repo,
                url,
                package,
                binary,
                library,
                root,
                &cfg,
            )?;
        }
        Commands::Install { locked, verify, tag } => {
            let start = std::time::Instant::now();
            let ui = installer::InstallOptions {
                quiet: cfg.quiet,
                colored: color_err,
            };
            let result =
                installer::run_install(locked, verify, tag.as_deref(), root, ui).await?;
            let elapsed = start.elapsed().as_secs_f64();

            if cfg.quiet {
                for (name, err) in &result.failed {
                    eprintln!("error: {name}: {err}");
                }
            } else {
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
                        parts.push(output::green(
                            color_out,
                            &format!("{n_installed} installed"),
                        ));
                    }
                    if n_skipped > 0 {
                        parts.push(output::dim(
                            color_out,
                            &format!("{n_skipped} skipped"),
                        ));
                    }
                    if n_failed > 0 {
                        parts.push(output::red(
                            color_out,
                            &format!("{n_failed} failed"),
                        ));
                    }
                    println!("\n  {}  ({elapsed:.1}s)", parts.join(", "));
                }
            }

            if !result.failed.is_empty() {
                std::process::exit(1);
            }
        }
        Commands::Check { tag } => {
            let r = checker::run_check(tag.as_deref(), root)?;
            cmd_check_print(r, &cfg, color_out, color_err)?;
        }
        Commands::Run { args } => cmd_run(args, root)?,
        Commands::List => cmd_list(root, &cfg)?,
        Commands::Update { name } => cmd_update(name, root, &cfg).await?,
        Commands::Outdated { tag } => cmd_outdated(tag, root, &cfg).await?,
        Commands::Env { shell } => cmd_env(shell, root, &cfg)?,
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

    if r.declared == 0 {
        if !cfg.quiet {
            println!("No binaries declared in grip.toml.");
        }
        return Ok(());
    }

    if r.examined == 0 {
        if !cfg.quiet {
            println!("No binaries matched this check (platform or --tag filter).");
            println!(
                "hint: {}",
                output::dim(
                    color_out,
                    "Adjust `platforms` / `tags` in grip.toml or run without `--tag`.",
                )
            );
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
    } else {
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

        let n_ok = r.passed.len();
        let n_warn = r.warned.len();
        let n_fail = r.failed.len();
        let summary = if n_fail == 0 && n_warn == 0 {
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
            parts.join(", ")
        };
        println!("\n  {summary}");
    }

    if !r.failed.is_empty() {
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
                    "Add tools with `grip add <name>` then run `grip install`.",
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
            output::dim(color, "Run `grip add <name>` then `grip install` to populate .bin/.")
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
    root: Option<std::path::PathBuf>,
    cfg: &OutputCfg,
) -> Result<(), GripError> {
    let (stem, ver_from_at) = parse_name_at_version(name);
    let version = version.or(ver_from_at);

    let (binary_name, github_shorthand_repo) = if stem.contains('/') {
        if let Some(src) = source.as_deref() {
            if src != "github" {
                return Err(GripError::Other(format!(
                    "NAME '{stem}' looks like owner/repo; use `--source github` or omit `--source`, or use a simple binary name."
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

    let default_source = detect_default_source();
    let source_str = source.as_deref().unwrap_or(&default_source);

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
                meta: Default::default(),
            }),
            "dnf" => LibraryEntry::Dnf(LibDnfEntry {
                package: package.unwrap_or_else(|| binary_name.clone()),
                version,
                meta: Default::default(),
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
            meta: Default::default(),
        }),
        "dnf" => BinaryEntry::Dnf(DnfEntry {
            package: package.unwrap_or_else(|| binary_name.clone()),
            binary,
            version,
            meta: Default::default(),
        }),
        "github" => BinaryEntry::Github(GithubEntry {
            repo: repo_resolved.ok_or_else(|| {
                GripError::Other("--repo required for github source (or use `grip add owner/repo`)".into())
            })?,
            version,
            asset_pattern: None,
            binary: None,
            meta: Default::default(),
        }),
        "url" => BinaryEntry::Url(UrlEntry {
            url: url.ok_or_else(|| GripError::Other("--url required for url source".into()))?,
            binary: None,
            sha256: None,
            meta: Default::default(),
        }),
        "shell" => BinaryEntry::Shell(ShellEntry {
            install_cmd: String::new(),
            version,
            meta: Default::default(),
        }),
        other => return Err(GripError::UnknownAdapter(other.to_string())),
    };

    manifest.binaries.insert(binary_name.clone(), entry);
    manifest.save(&manifest_path)?;
    if !cfg.quiet {
        println!("Added '{}' to grip.toml", binary_name);
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

    let status = std::process::Command::new(&args[0])
        .args(&args[1..])
        .env("PATH", new_path)
        .status()?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Choose a sensible default source adapter for the current platform:
/// `dnf` or `apt` on Linux (whichever is on PATH), `github` otherwise.
fn detect_default_source() -> String {
    let platform = platform::Platform::current();
    if platform.is_linux() {
        for cmd in &["dnf", "apt-get", "apt"] {
            if std::process::Command::new("which")
                .arg(cmd)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return match *cmd {
                    "dnf" => "dnf",
                    _ => "apt",
                }
                .to_string();
            }
        }
    }
    "github".to_string()
}

/// Print a formatted table of all entries in `grip.lock`.
fn cmd_list(root: Option<std::path::PathBuf>, cfg: &OutputCfg) -> Result<(), GripError> {
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

    if lock.entries.is_empty() && lock.library_entries.is_empty() {
        if !cfg.quiet {
            println!("No binaries or libraries installed yet.");
            println!(
                "hint: {}",
                output::dim(color, "Run `grip install` to install everything from grip.toml.")
            );
        }
        return Ok(());
    }

    if !cfg.quiet {
        if !lock.entries.is_empty() {
            println!();
            let header = output::dim(color, "Installed binaries (from grip.lock)");
            println!("  {header}");
            println!();
            println!(
                "  {:<18} {:<14} {:<10} {}",
                "NAME", "VERSION", "SOURCE", "INSTALLED AT"
            );
            println!("  {}", "-".repeat(66));
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
            println!();
            let header = output::dim(color, "Installed libraries (from grip.lock)");
            println!("  {header}");
            println!();
            println!(
                "  {:<18} {:<14} {:<10} {}",
                "NAME", "VERSION", "SOURCE", "INSTALLED AT"
            );
            println!("  {}", "-".repeat(66));
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
                let adapter = crate::adapters::get_adapter(&entry);
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
    let name_w = entries.iter().map(|(n, _)| n.len()).max().unwrap_or(6).max(6);
    let col_w = 14usize;

    if !cfg.quiet {
        println!(
            "  {:<name_w$}  {:<col_w$}  {:<col_w$}  STATUS",
            "BINARY", "INSTALLED", "LATEST",
        );
        println!("  {}", output::dim(color, &"─".repeat(name_w + col_w * 2 + 16)));

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

/// Re-install a single named binary, ignoring the lock file, and update the lock entry.
async fn cmd_update(name: String, root: Option<std::path::PathBuf>, cfg: &OutputCfg) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };
    let manifest_path = project_root.join("grip.toml");
    let lock_path = project_root.join("grip.lock");
    let bin_dir = crate::bin_dir::ensure_bin_dir(&project_root)?;

    let manifest = Manifest::load(&manifest_path)?;
    let mut lock = LockFile::load(&lock_path)?;

    let entry = manifest
        .binaries
        .get(&name)
        .ok_or_else(|| GripError::Other(format!("'{name}' not found in grip.toml")))?
        .clone();

    let client = reqwest::Client::builder()
        .user_agent("grip/0.1")
        .build()
        .map_err(GripError::Http)?;

    let adapter = crate::adapters::get_adapter(&entry);
    let color_err = cfg.use_color_stderr();
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
    let lock_entry = adapter
        .install(&name, &entry, &bin_dir, &client, pb, color_err)
        .await?;
    if !cfg.quiet {
        let check = output::success_checkmark(color_err);
        println!("\n  {check}  updated {name} to {}", lock_entry.version);
    }
    lock.upsert(lock_entry);
    lock.save(&lock_path)?;
    Ok(())
}
