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
  grip lock
  grip sync
  grip lock --check
  grip sync --check
  grip lock verify
  grip lock --upgrade
  grip suggest --path src/
  grip export --format cyclonedx -o sbom.json
  grip audit
  grip run jq --version
  grip run -- fd --hidden .
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
    /// When to use colors for grip output (default: `auto`; respects NO_COLOR).
    #[arg(long, global = true, value_name = "WHEN", default_value_t = ColorWhen::Auto)]
    pub color: ColorWhen,
    /// Override the project root directory (skips the grip.toml walk).
    /// Useful inside containers where the project root is known.
    #[arg(long, global = true, value_name = "DIR")]
    pub project: Option<std::path::PathBuf>,
    /// Change to DIR before running any command.
    #[arg(long, global = true, value_name = "DIR")]
    pub directory: Option<std::path::PathBuf>,
    /// Disable all network access; rely only on local cache and installed state.
    #[arg(long, global = true)]
    pub offline: bool,
    /// Bypass the local download cache for this run.
    #[arg(long, global = true)]
    pub no_cache: bool,
    /// Hide all progress output (spinners, progress bars).
    #[arg(long, global = true)]
    pub no_progress: bool,
    /// Override the cache directory. Also settable via GRIP_CACHE_DIR.
    #[arg(long, global = true, value_name = "DIR")]
    pub cache_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create grip.toml (and .gitignore entry for .bin/) in the current directory or PATH
    ///
    /// When a Dockerfile is detected (or passed via --from), grip parses it for
    /// `RUN apt-get install` / `RUN dnf install` lines, classifies each package as
    /// a binary tool or a library, verifies the findings against a curated list and
    /// the host package manager, and offers to import the verified set into grip.toml.
    Init {
        /// Directory to initialize (defaults to current directory)
        path: Option<std::path::PathBuf>,
        /// Explicit Dockerfile path(s) to import from; may be repeated.
        /// When omitted, grip auto-detects Dockerfile / Dockerfile.* / *.dockerfile.
        #[arg(long = "from", short = 'f', value_name = "PATH")]
        from: Vec<std::path::PathBuf>,
        /// Accept all verified entries without prompting (also the default for non-TTY)
        #[arg(long, short = 'y')]
        yes: bool,
        /// Blank template only; skip Dockerfile scanning
        #[arg(long)]
        bare: bool,
        /// Skip GitHub repo-existence checks; rely only on the curated list and host package manager
        #[arg(long)]
        offline: bool,
    },
    /// Add one or more binaries or libraries to grip.toml and install them
    ///
    /// For GitHub, pass `owner/repo` as NAME (binary name becomes the last segment),
    /// or `name@version` to pin a specific version. Multiple names install in one shot.
    /// Use `--library` to add to [libraries] instead of [binaries].
    Add {
        /// Binary name(s), `owner/repo` for GitHub, or `name@version` to pin
        #[arg(required = true)]
        names: Vec<String>,
        #[arg(long, help = "github | url | apt | dnf (default: OS-specific)")]
        source: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(
            long,
            help = "GitHub `owner/repo` (optional if NAME is already owner/repo)"
        )]
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
        /// Write to grip.toml and grip.lock but skip installing into .bin/
        #[arg(long)]
        no_sync: bool,
        /// Write to grip.toml only; do not update grip.lock or install
        #[arg(long)]
        frozen: bool,
        /// GPG key fingerprint (or long key ID) to verify GitHub/URL release signatures
        #[arg(long, value_name = "FINGERPRINT")]
        gpg_fingerprint: Option<String>,
        /// Glob pattern to find the detached signature asset in a GitHub release (e.g. "*.asc");
        /// auto-detected if omitted (GitHub source only)
        #[arg(long, value_name = "PATTERN")]
        sig_asset_pattern: Option<String>,
        /// Glob pattern to find the signed checksums file in a GitHub release
        /// (e.g. "*SHA256SUMS*"); activates signed-checksums verification (GitHub source only)
        #[arg(long, value_name = "PATTERN")]
        checksums_asset_pattern: Option<String>,
        /// URL of the detached GPG signature file (URL source only)
        #[arg(long, value_name = "URL")]
        sig_url: Option<String>,
        /// URL of a signed checksums file (URL source only); activates signed-checksums verification
        #[arg(long, value_name = "URL")]
        signed_checksums_url: Option<String>,
        /// URL of the GPG signature for the checksums file (URL source only)
        #[arg(long, value_name = "URL")]
        checksums_sig_url: Option<String>,
    },
    /// Remove one or more binary or library entries from grip.toml, grip.lock, and .bin/
    Remove {
        /// Name(s) of the entries to remove (must match keys in grip.toml)
        #[arg(required = true)]
        names: Vec<String>,
        /// Remove from [libraries] instead of [binaries]
        #[arg(long)]
        library: bool,
        /// Remove from grip.toml and grip.lock but leave .bin/ untouched
        #[arg(long)]
        no_sync: bool,
        /// Remove from grip.toml only; do not update grip.lock
        #[arg(long)]
        frozen: bool,
    },
    /// Manage grip.lock: update, check, upgrade, pin, or verify
    ///
    /// Without a subcommand, resolves every entry in grip.toml and writes grip.lock.
    /// Use `grip lock verify` for tamper detection and `grip lock pin` to write
    /// versions back into grip.toml.
    Lock {
        #[command(subcommand)]
        action: Option<LockAction>,
        /// Assert grip.lock is up to date; exit 1 if a re-lock would modify it
        #[arg(long, conflicts_with_all = ["upgrade", "upgrade_package", "dry_run"])]
        check: bool,
        /// Show what would be written to grip.lock without modifying the file
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Re-resolve all entries to the latest version available from their source
        #[arg(long, conflicts_with = "check")]
        upgrade: bool,
        /// Re-resolve the named entry to the latest available version (repeatable)
        #[arg(
            long = "upgrade-package",
            value_name = "NAME",
            conflicts_with = "check"
        )]
        upgrade_package: Vec<String>,
        /// Only consider entries that carry this tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// Download and install any missing binaries from grip.toml into .bin/
    ///
    /// The project is re-locked before syncing unless --locked or --frozen is provided.
    Sync {
        /// Fail if grip.lock would change (for CI)
        #[arg(long)]
        locked: bool,
        /// Do not update grip.lock; install exactly what is already recorded in it
        #[arg(long)]
        frozen: bool,
        /// Verify .bin/ matches grip.lock without installing or modifying anything; exit 1 on mismatch
        #[arg(long)]
        check: bool,
        /// Show what would be installed without writing anything to disk
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Re-verify SHA256 of on-disk binaries against the lock file
        #[arg(long)]
        verify: bool,
        /// Only install entries that include this tag
        #[arg(long)]
        tag: Option<String>,
        /// Fail before touching the network if any entry has no version pin (prevents silent auto-upgrades in CI)
        #[arg(long)]
        require_pins: bool,
    },
    /// Run a command with .bin/ prepended to PATH
    ///
    /// All arguments after -- are passed directly to the command.
    Run {
        /// Skip the pre-run sync; run with whatever is already in .bin/
        #[arg(long)]
        no_sync: bool,
        /// When syncing before run, fail if grip.lock would change
        #[arg(long)]
        locked: bool,
        /// When syncing before run, do not update grip.lock
        #[arg(long)]
        frozen: bool,
        #[arg(required = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Print installed entries from grip.lock with versions, sources, and timestamps
    Tree {
        /// Also show entries declared in grip.toml that are not yet installed
        #[arg(long)]
        all: bool,
    },
    /// Manage the local download cache (~/.cache/grip/downloads/)
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Suggest CLI tools to add based on project scripts, CI files, and source code
    ///
    /// Scans Makefile, scripts/, .github/workflows/, and any source paths you pass with --path.
    /// Cross-references findings against a curated list of known tools and your
    /// existing grip.toml, then prints suggested `grip add` commands.
    Suggest {
        /// Source-code paths to scan for binary invocations (repeatable)
        #[arg(long = "path", short = 'p', value_name = "PATH")]
        paths: Vec<std::path::PathBuf>,
        /// Also scan shell history files (~/.bash_history, ~/.zsh_history, fish history)
        #[arg(long)]
        history: bool,
        /// Exit with status 1 if any unmanaged tools are found; useful in CI
        #[arg(long)]
        check: bool,
    },
    /// Export install commands or a machine-readable dependency artifact from grip.lock
    ///
    /// Formats: shell (default), dockerfile, makefile, cyclonedx, spdx.
    Export {
        /// Output format: shell | dockerfile | makefile | cyclonedx | spdx
        #[arg(long, default_value = "shell")]
        format: String,
        /// Write output to FILE instead of stdout
        #[arg(long, short = 'o', value_name = "FILE")]
        output: Option<std::path::PathBuf>,
    },
    /// Check installed tool versions against the OSV vulnerability database
    ///
    /// Sends a single batch query to https://api.osv.dev/v1/querybatch using
    /// the purl of each entry in grip.lock.  Prints a table of findings and
    /// exits non-zero if any are found (suitable for CI).
    ///
    /// Examples:
    ///   grip audit
    ///   grip audit --no-fail    # report findings but always exit 0
    Audit {
        /// Exit 0 even when vulnerabilities are found (default: exit 1 on findings)
        #[arg(long)]
        no_fail: bool,
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
pub enum LockAction {
    /// Re-hash every binary in .bin/ and compare against grip.lock; exits non-zero on any mismatch
    ///
    /// Designed for CI: reads only the lock file (no network, no manifest walk) and
    /// rehashes every .bin/ binary that has a recorded sha256. Detects binaries that
    /// were swapped or modified after installation.
    Verify,
    /// Pin all unpinned entries in grip.toml to their currently installed versions (from grip.lock)
    ///
    /// Reads each binary and library that has no `version` field and writes the exact version
    /// recorded in grip.lock back into grip.toml. Entries not yet installed are skipped with a
    /// warning — run `grip sync` first, then re-run `grip lock pin`.
    ///
    /// Examples:
    ///   grip lock pin              # pin everything unpinned
    ///   grip lock pin --dry-run    # preview changes without writing grip.toml
    Pin {
        /// Preview what would be pinned without modifying grip.toml
        #[arg(long = "dry-run")]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum CacheAction {
    /// Print the resolved cache directory path
    Dir,
    /// Show the number of cached archives and total disk usage
    Size,
    /// Remove all cached downloads and print how much was freed
    Clean,
    /// Remove stale or unreachable cache entries (currently equivalent to clean)
    Prune,
}
