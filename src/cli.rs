//! CLI argument definitions parsed by clap.

use clap::{ColorChoice, Parser, Subcommand};

use crate::output::ColorWhen;

const LONG_ABOUT: &str = "\
Grip installs CLI tools declared in grip.toml into a project-local .bin/ directory, \
similar to how a Python project pins dependencies. A grip.lock file records exact versions \
and checksums for reproducible installs.";

const AFTER_LONG_HELP: &str = "\
Examples:
  grip init
  grip add BurntSushi/ripgrep
  grip add jq@1.7.1 --repo jqlang/jq
  grip install
  grip check
  grip outdated
  grip run jq --version
  eval \"$(grip env)\"

Documentation: https://github.com/omnone/grip (see README in the repository)";

#[derive(Parser)]
#[command(
    name = "grip",
    about = "Per-project binary dependency manager",
    long_about = LONG_ABOUT,
    after_long_help = AFTER_LONG_HELP,
    version,
    color = ColorChoice::Always
)]
pub struct Cli {
    /// Suppress non-essential output (install spinners and decorative lines).
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// Print more detail on errors (e.g. underlying I/O or HTTP messages).
    #[arg(short, long, global = true)]
    pub verbose: bool,
    /// When to use colors for grip output (`always` by default; respect NO_COLOR).
    #[arg(long, global = true, value_name = "WHEN", default_value_t = ColorWhen::Always)]
    pub color: ColorWhen,
    /// Override the project root directory (skips the grip.toml walk).
    /// Useful inside containers where the project root is known.
    #[arg(long, global = true, value_name = "DIR")]
    pub root: Option<std::path::PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create grip.toml (and .gitignore entry for .bin/) in the current directory
    Init,
    /// Add a binary or library entry to grip.toml
    ///
    /// For GitHub, you can pass `owner/repo` as NAME (binary name becomes the last segment),
    /// or `name@version` to pin a version. On Linux, the default source is often apt/dnf unless
    /// you pass `--source github`. Use `--library` to add to [libraries] instead of [binaries].
    Add {
        /// Binary name, or `owner/repo` for GitHub, optionally `name@version`
        name: String,
        #[arg(long, help = "github | url | apt | dnf | shell (default: OS-specific)")]
        source: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long, help = "GitHub `owner/repo` (optional if NAME is already owner/repo)")]
        repo: Option<String>,
        #[arg(long, help = "Direct download URL (required for --source url)")]
        url: Option<String>,
        #[arg(long, help = "Package name for apt/dnf (defaults to binary name)")]
        package: Option<String>,
        #[arg(
            long,
            help = "On-PATH command for apt/dnf when it differs from NAME (e.g. ripgrep → rg)"
        )]
        binary: Option<String>,
        #[arg(long, help = "Add to [libraries] instead of [binaries] (apt/dnf only)")]
        library: bool,
    },
    /// Install all binaries from grip.toml into .bin/ and update grip.lock
    #[command(visible_alias = "sync")]
    Install {
        /// Fail if the lock file would change (for CI)
        #[arg(long)]
        locked: bool,
        /// Re-verify SHA256 of on-disk binaries against the lock file
        #[arg(long)]
        verify: bool,
        /// Only install entries that include this tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// Run a command with .bin/ prepended to PATH
    Run {
        #[arg(required = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Verify `.bin/` matches grip.lock (version pins + SHA256); does not install or modify files
    Check {
        /// Only check entries that include this tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// List installed binaries from grip.lock
    List,
    /// Remove a binary or library entry from grip.toml, grip.lock, and .bin/
    Remove {
        /// Name of the entry to remove (must match the key in grip.toml)
        name: String,
        /// Remove from [libraries] instead of [binaries]
        #[arg(long)]
        library: bool,
    },
    /// Re-install one binary from the manifest and refresh its lock entry
    Update {
        name: String,
    },
    /// Check whether newer versions of installed binaries are available
    Outdated {
        /// Only check entries that include this tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// Check consistency of grip.toml, grip.lock, and .bin/
    Doctor,
    /// Manage the local download cache (~/.cache/grip/downloads/)
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Export install commands for use in Dockerfiles or CI scripts
    Export {
        /// Output format: dockerfile | shell | makefile
        #[arg(long, default_value = "shell")]
        format: String,
    },
    /// Print shell code to add .bin/ to PATH (for use with eval)
    ///
    /// Bash / zsh — add to ~/.bashrc or ~/.zshrc:
    ///   eval "$(grip env)"
    ///
    /// Fish — add to ~/.config/fish/config.fish:
    ///   grip env --shell fish | source
    Env {
        /// Shell type: bash, zsh, fish, sh (auto-detected from $SHELL if omitted)
        #[arg(long, value_name = "SHELL")]
        shell: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum CacheAction {
    /// Remove all cached downloads and print how much was freed
    Clean,
    /// Show the number of cached files and total disk usage
    Info,
}
