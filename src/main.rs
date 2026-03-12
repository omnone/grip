//! Entry point and CLI command implementations for `grip`.

mod adapters;
mod bin_dir;
mod checksum;
mod cli;
mod config;
mod error;
mod installer;
mod platform;

use std::io::Write;
use std::time::Duration;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use cli::{Cli, Commands};
use config::manifest::{
    find_manifest_dir, AptEntry, BinaryEntry, DnfEntry, GithubEntry, Manifest,
    ShellEntry, UrlEntry,
};
use config::lockfile::LockFile;
use error::GripError;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let root = cli.root;

    match cli.command {
        Commands::Init => cmd_init()?,
        Commands::Add {
            name,
            source,
            version,
            repo,
            url,
            package,
        } => {
            cmd_add(name, source, version, repo, url, package, root)?;
        }
        Commands::Install { locked, verify, tag } => {
            let start = std::time::Instant::now();
            let result = installer::run_install(locked, verify, tag.as_deref(), root).await?;
            let elapsed = start.elapsed().as_secs_f64();

            // Print errors/warnings detail
            for (name, err) in &result.warned {
                eprintln!("  \x1b[33m⚠\x1b[0m  {name}: {err}");
            }
            for (name, err) in &result.failed {
                eprintln!("  \x1b[31m✗\x1b[0m  {name}: {err}");
            }

            // Summary line
            let n_installed = result.installed.len();
            let n_skipped = result.skipped.len();
            let n_failed = result.failed.len() + result.warned.len();

            if n_installed == 0 && n_failed == 0 {
                println!("\n  \x1b[2mAll up to date\x1b[0m  ({n_skipped} skipped, {elapsed:.1}s)");
            } else {
                let mut parts: Vec<String> = Vec::new();
                if n_installed > 0 {
                    parts.push(format!("\x1b[32m{n_installed} installed\x1b[0m"));
                }
                if n_skipped > 0 {
                    parts.push(format!("\x1b[2m{n_skipped} skipped\x1b[0m"));
                }
                if n_failed > 0 {
                    parts.push(format!("\x1b[31m{n_failed} failed\x1b[0m"));
                }
                println!("\n  {}  ({elapsed:.1}s)", parts.join(", "));
            }

            if !result.failed.is_empty() {
                std::process::exit(1);
            }
        }
        Commands::Run { args } => cmd_run(args, root)?,
        Commands::List => cmd_list(root)?,
        Commands::Update { name } => cmd_update(name, root).await?,
    }

    Ok(())
}

/// Create a `binaries.toml` template in the current directory and add `.bin/` to `.gitignore`.
fn cmd_init() -> Result<(), GripError> {
    let path = std::path::Path::new("binaries.toml");
    if path.exists() {
        println!("binaries.toml already exists");
        return Ok(());
    }

    let template = r#"# binaries.toml — managed by grip
# Add your binary dependencies here.

# Example:
# [binaries.jq]
# source = "github"
# repo = "jqlang/jq"
# version = "1.7.1"
# asset_pattern = "jq-linux-amd64"
"#;
    std::fs::write(path, template)?;
    println!("Created binaries.toml");

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
            println!("Added .bin/ to .gitignore");
        }
    } else {
        std::fs::write(gitignore, entry)?;
        println!("Created .gitignore with .bin/");
    }

    Ok(())
}

/// Add a new binary entry to `binaries.toml`, inferring the source adapter from the platform when
/// `--source` is not provided.
fn cmd_add(
    name: String,
    source: Option<String>,
    version: Option<String>,
    repo: Option<String>,
    url: Option<String>,
    package: Option<String>,
    root: Option<std::path::PathBuf>,
) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).unwrap_or(cwd)
        }
    };
    let manifest_path = project_root.join("binaries.toml");

    let mut manifest = if manifest_path.exists() {
        Manifest::load(&manifest_path)?
    } else {
        Manifest::empty()
    };

    let default_source = detect_default_source();
    let source_str = source.as_deref().unwrap_or(&default_source);

    let entry = match source_str {
        "apt" => BinaryEntry::Apt(AptEntry {
            package: package.unwrap_or_else(|| name.clone()),
            version,
            meta: Default::default(),
        }),
        "dnf" => BinaryEntry::Dnf(DnfEntry {
            package: package.unwrap_or_else(|| name.clone()),
            version,
            meta: Default::default(),
        }),
        "github" => BinaryEntry::Github(GithubEntry {
            repo: repo
                .ok_or_else(|| GripError::Other("--repo required for github source".into()))?,
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

    manifest.binaries.insert(name.clone(), entry);
    manifest.save(&manifest_path)?;
    println!("Added '{}' to binaries.toml", name);
    Ok(())
}

/// Run a command with the project's `.bin/` directory prepended to `PATH`.
fn cmd_run(args: Vec<String>, root: Option<std::path::PathBuf>) -> Result<(), GripError> {
    if args.is_empty() {
        eprintln!("Usage: grip run <command> [args...]");
        std::process::exit(1);
    }

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
/// `brew` on macOS, `dnf` or `apt` on Linux (whichever is on PATH), `github` otherwise.
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

/// Print a formatted table of all entries in `binaries.lock`.
fn cmd_list(root: Option<std::path::PathBuf>) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };
    let lock_path = project_root.join("binaries.lock");
    let lock = LockFile::load(&lock_path)?;

    if lock.entries.is_empty() {
        println!("No binaries installed yet. Run 'grip install'.");
        return Ok(());
    }

    println!(
        "{:<20} {:<15} {:<10} {}",
        "NAME", "VERSION", "SOURCE", "INSTALLED AT"
    );
    println!("{}", "-".repeat(70));
    for e in &lock.entries {
        println!(
            "{:<20} {:<15} {:<10} {}",
            e.name,
            e.version,
            e.source,
            e.installed_at.format("%Y-%m-%d %H:%M")
        );
    }
    Ok(())
}

/// Re-install a single named binary, ignoring the lock file, and update the lock entry.
async fn cmd_update(name: String, root: Option<std::path::PathBuf>) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).ok_or(GripError::ManifestNotFound)?
        }
    };
    let manifest_path = project_root.join("binaries.toml");
    let lock_path = project_root.join("binaries.lock");
    let bin_dir = crate::bin_dir::ensure_bin_dir(&project_root)?;

    let manifest = Manifest::load(&manifest_path)?;
    let mut lock = LockFile::load(&lock_path)?;

    let entry = manifest
        .binaries
        .get(&name)
        .ok_or_else(|| GripError::Other(format!("'{}' not found in binaries.toml", name)))?
        .clone();

    let client = reqwest::Client::builder()
        .user_agent("grip/0.1")
        .build()
        .map_err(GripError::Http)?;

    let adapter = crate::adapters::get_adapter(&entry);
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]),
    );
    pb.set_message(format!("{name}  resolving..."));
    pb.enable_steady_tick(Duration::from_millis(80));
    let lock_entry = adapter.install(&name, &entry, &bin_dir, &client, pb).await?;
    println!("\n  \x1b[32m✓\x1b[0m  updated {name} to {}", lock_entry.version);
    lock.upsert(lock_entry);
    lock.save(&lock_path)?;
    Ok(())
}
