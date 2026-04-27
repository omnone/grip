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
  grip sync
  grip check
  grip lock verify
  grip outdated
  grip suggest --path src/
  grip sbom --output sbom.json
  grip audit
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
    #[arg(long, global = true, value_name = "WHEN", default_value_t = ColorWhen::Auto)]
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
    ///
    /// When a Dockerfile is detected (or passed via --from), grip parses it for
    /// `RUN apt-get install` / `RUN dnf install` lines, classifies each package as
    /// a binary tool or a library, verifies the findings against a curated list and
    /// the host package manager, and offers to import the verified set into grip.toml.
    Init {
        /// Explicit Dockerfile path(s) to import from; may be repeated.
        /// When omitted, grip auto-detects Dockerfile / Dockerfile.* / *.dockerfile in cwd.
        #[arg(long = "from", short = 'f', value_name = "PATH")]
        from: Vec<std::path::PathBuf>,
        /// Accept all verified entries without prompting (also the default for non-TTY)
        #[arg(long, short = 'y')]
        yes: bool,
        /// Skip Dockerfile scanning; create a blank grip.toml template only
        #[arg(long)]
        no_import: bool,
        /// Skip GitHub repo-existence checks; rely only on the curated list and host package manager
        #[arg(long)]
        offline: bool,
    },
    /// Add a binary or library entry to grip.toml
    ///
    /// For GitHub, you can pass `owner/repo` as NAME (binary name becomes the last segment),
    /// or `name@version` to pin a version. On Linux, the default source is often apt/dnf unless
    /// you pass `--source github`. Use `--library` to add to [libraries] instead of [binaries].
    Add {
        /// Binary name, or `owner/repo` for GitHub, optionally `name@version`
        name: String,
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
        /// URL of the GPG signature for the checksums file (URL source only); required with --signed-checksums-url
        #[arg(long, value_name = "URL")]
        checksums_sig_url: Option<String>,
    },
    /// Download and install any missing binaries from grip.toml into .bin/
    Sync {
        /// Fail if the lock file would change (for CI)
        #[arg(long)]
        locked: bool,
        /// Re-verify SHA256 of on-disk binaries against the lock file
        #[arg(long)]
        verify: bool,
        /// Only install entries that include this tag
        #[arg(long)]
        tag: Option<String>,
        /// Fail if any entry in grip.toml has no version pin (for CI; prevents silent auto-upgrades)
        #[arg(long)]
        require_pins: bool,
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
    List {
        /// Also show entries declared in grip.toml that are not yet installed
        #[arg(long)]
        all: bool,
    },
    /// Remove a binary or library entry from grip.toml, grip.lock, and .bin/
    Remove {
        /// Name of the entry to remove (must match the key in grip.toml)
        name: String,
        /// Remove from [libraries] instead of [binaries]
        #[arg(long)]
        library: bool,
    },
    /// Re-install one or all binaries from the manifest and refresh their lock entries
    Update {
        /// Name of the binary or library to update (omit when using --all)
        name: Option<String>,
        /// Update every binary and library declared in grip.toml
        #[arg(long)]
        all: bool,
    },
    /// Check whether newer versions of installed binaries are available
    Outdated {
        /// Only check entries that include this tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// Pin all unpinned entries in grip.toml to their currently installed versions (from grip.lock)
    ///
    /// Reads each binary and library that has no `version` field and writes the exact version
    /// recorded in grip.lock back into grip.toml. Entries that are not yet installed are skipped
    /// with a warning — run `grip sync` first, then re-run `grip pin`.
    ///
    /// Examples:
    ///   grip pin              # pin everything unpinned
    ///   grip pin --dry-run    # preview changes without writing grip.toml
    Pin {
        /// Preview what would be pinned without modifying grip.toml
        #[arg(long)]
        dry_run: bool,
    },
    /// Manage the local download cache (~/.cache/grip/downloads/)
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Inspect and verify grip.lock
    ///
    /// Sub-commands operate on the lock file directly — no network, no manifest walk.
    Lock {
        #[command(subcommand)]
        action: LockAction,
    },
    /// Suggest CLI tools to add based on shell history, project scripts, and source code
    ///
    /// Scans ~/.bash_history, ~/.zsh_history, ~/.local/share/fish/fish_history,
    /// Makefile, scripts/, .github/workflows/, and any source paths you pass with --path.
    /// Cross-references findings against a curated list of known tools and your
    /// existing grip.toml, then prints suggested `grip add` commands.
    Suggest {
        /// Source-code paths to scan for binary invocations (Rust, Python, JS, Go, Ruby, shell,
        /// Dockerfile, YAML, TOML …). Detects subprocess API calls and /bin/<name> path literals.
        #[arg(long = "path", short = 'p', value_name = "PATH")]
        paths: Vec<std::path::PathBuf>,
        /// Also scan shell history files (~/.bash_history, ~/.zsh_history, fish history)
        #[arg(long)]
        history: bool,
        /// Exit with a non-zero status if any suggestions are found (for CI)
        #[arg(long)]
        check: bool,
    },
    /// Export install commands for use in Dockerfiles or CI scripts
    Export {
        /// Output format: dockerfile | shell | makefile
        #[arg(long, default_value = "shell")]
        format: String,
    },
    /// Generate a Software Bill of Materials from grip.lock
    ///
    /// Reads grip.lock and emits a machine-readable SBOM in CycloneDX 1.5 JSON
    /// (default) or SPDX 2.3 JSON.  No network access required.
    ///
    /// Examples:
    ///   grip sbom                              # CycloneDX to stdout
    ///   grip sbom --format spdx                # SPDX to stdout
    ///   grip sbom --output sbom.json           # CycloneDX to file
    ///   grip sbom --format spdx -o sbom.spdx.json
    Sbom {
        /// Output format: cyclonedx (default) or spdx
        #[arg(long, default_value = "cyclonedx", value_name = "FORMAT")]
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
pub enum CacheAction {
    /// Remove all cached downloads and print how much was freed
    Clean,
    /// Show the number of cached files and total disk usage
    Info,
}

#[derive(Subcommand)]
pub enum LockAction {
    /// Re-hash every binary in .bin/ and compare against grip.lock; exits non-zero on any mismatch
    ///
    /// Designed for CI: reads only the lock file (no network, no manifest walk) and
    /// rehashes every .bin/ binary that has a recorded sha256.  Detects binaries that
    /// were swapped or modified after installation.
    Verify,
}
