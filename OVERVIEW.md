# grip — Codebase Overview

> **grip** is a Rust-based CLI tool that manages project-local binary and library dependencies. It solves the problem of coordinating tool versions across development teams, CI/CD pipelines, and Docker builds by declaring tools in a `grip.toml` manifest, installing them into an isolated `.bin/` directory, and locking exact versions + SHA-256 checksums in `grip.lock`.

---

## Directory Structure

```
binaries-manager/
├── src/
│   ├── main.rs                    # Entry point and all command implementations
│   ├── cli.rs                     # Clap CLI definitions
│   ├── installer.rs               # Core install orchestration
│   ├── checker.rs                 # Verification logic (grip check)
│   ├── lock_verify.rs             # Lock file tamper detection (grip lock verify)
│   ├── gpg.rs                     # GPG signature and signed-checksums verification
│   ├── error.rs                   # GripError enum and formatting
│   ├── output.rs                  # Terminal styling and ANSI colors
│   ├── platform.rs                # OS/arch detection
│   ├── privilege.rs               # Sudo/root detection
│   ├── bin_dir.rs                 # .bin/ directory management
│   ├── cache.rs                   # Download cache logic
│   ├── checksum.rs                # SHA-256 verification
│   ├── adapters/
│   │   ├── mod.rs                 # SourceAdapter trait definition
│   │   ├── github.rs              # GitHub Releases adapter
│   │   ├── url.rs                 # Direct URL downloader
│   │   ├── apt.rs                 # APT package manager adapter
│   │   ├── dnf.rs                 # DNF package manager adapter
│   │   └── shell.rs               # Shell command executor (allow_shell guard)
│   └── config/
│       ├── mod.rs                 # Config module root
│       ├── manifest.rs            # grip.toml TOML structs
│       └── lockfile.rs            # grip.lock structs and I/O
├── tests/
│   ├── integration_apt.rs         # APT adapter integration tests (272 lines)
│   ├── integration_dnf.rs         # DNF adapter integration tests (272 lines)
│   ├── integration_github.rs      # GitHub adapter integration tests (188 lines)
│   ├── integration_url.rs         # URL adapter integration tests (159 lines)
│   ├── integration_shell.rs       # Shell adapter integration tests (176 lines)
│   └── docker/
│       ├── Dockerfile.test-apt    # Debian Bookworm container for APT suite
│       ├── Dockerfile.test-dnf    # Fedora 40 container for DNF suite
│       ├── Dockerfile.test-github # Debian Bookworm container for GitHub suite
│       ├── Dockerfile.test-url    # Debian Bookworm container for URL suite
│       └── Dockerfile.test-shell  # Debian Bookworm container for Shell suite
├── Cargo.toml                     # Rust project metadata and dependencies
├── Cargo.lock                     # Locked Rust dependency versions
├── grip.toml                      # Example manifest
├── grip.lock                      # Example lock file
├── README.md                      # User documentation
├── OVERVIEW.md                    # This file — architecture reference
├── COMMANDS.md                    # Full CLI and grip.toml reference
├── SECURITY.md                    # Security guide: GPG, allow_shell, lock verify, CI setup
├── CONTRIBUTING.md                # Contributor guide
├── LICENSE                        # MIT
└── Makefile                       # Build + integration test targets
```

---

## Key Files and Their Roles

| File | Responsibility |
|------|---------------|
| `main.rs` | Routes 15 CLI commands; contains the implementation logic for each |
| `cli.rs` | Clap-derived structs for all flags and subcommands |
| `installer.rs` | Concurrent adapter execution, lock file updates, `--require-pins` guard, platform/tag filtering |
| `checker.rs` | Validates `.bin/` against `grip.lock` (version, SHA256, presence) |
| `lock_verify.rs` | Re-hashes `.bin/` against `grip.lock` without reading the manifest; backing logic for `grip lock verify` |
| `gpg.rs` | GPG signature verification (Mode 1: direct `.sig`/`.asc`; Mode 2: signed checksums file); shared by GitHub and URL adapters |
| `adapters/mod.rs` | `SourceAdapter` async trait that all 5 adapters implement |
| `adapters/github.rs` | Resolves semver ranges, downloads GitHub release assets, extracts archives, calls `gpg.rs` if configured |
| `adapters/apt.rs` | Invokes APT with privilege escalation checks, symlinks binary into `.bin/` |
| `adapters/dnf.rs` | Same as APT but for DNF/RPM systems; uses PATH-search instead of `which` |
| `adapters/url.rs` | HTTP downloads with optional SHA256 verification, caching, and GPG verification |
| `adapters/shell.rs` | Enforces `allow_shell` guard; executes `install_cmd` with `GRIP_BIN_DIR` set |
| `config/manifest.rs` | TOML deserialization for all entry types; `is_version_pinned()` and `source_label()` helpers |
| `config/lockfile.rs` | TOML serialization, atomic writes, entry lookups for `grip.lock` |
| `bin_dir.rs` | Creates `.bin/`, copies/symlinks binaries, sets executable bit |
| `cache.rs` | Stores/retrieves archives keyed by SHA256 of URL; configurable via env var |
| `checksum.rs` | Streams SHA-256 computation during/after download |
| `error.rs` | `GripError` enum with actionable `.hint()` messages per variant |
| `output.rs` | ANSI colors, respects `NO_COLOR` and TTY detection |
| `platform.rs` | Detects Linux/macOS/Windows and x86_64/aarch64 |
| `privilege.rs` | Detects root or passwordless sudo via `id -u` / `sudo -n true` |

---

## Architecture

### Entry Point

```rust
#[tokio::main]
async fn main() {
    let cli = Cli::parse();       // clap argument parsing
    let cfg = OutputCfg { ... };  // colors + verbosity config
    run_command(cli, cfg).await;  // dispatch to command impl
}

async fn run_command(cli, cfg) {
    match cli.command {
        Commands::Init       => cmd_init(...),
        Commands::Add { .. } => cmd_add(...),
        Commands::Sync { .. } => installer::run_install(...).await,
        Commands::Check { .. } => checker::run_check(...),
        // ... 10 more
    }
}
```

### Install Pipeline

```
read grip.toml
  → read grip.lock (or empty)
  → ensure .bin/ directory exists
  → for each entry: filter by platform + tag
  → skip already-installed entries (lock entry present AND file on disk)
  → split into two buckets:
      system PM (apt/dnf)  → run sequentially to avoid pkg manager lock contention
      downloads (github/url/shell) → run concurrently via FuturesUnordered
  → each adapter returns LockEntry { name, version, source, url, sha256, installed_at }
  → upsert entries into lock file
  → atomic write: tempfile + rename
  → print summary (installed / skipped / failed)
  → exit 1 if any required entry failed
```

### Adapter Trait

All 5 installation sources implement the same async trait:

```rust
pub trait SourceAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn is_supported(&self) -> bool;
    async fn install(...) -> Result<LockEntry, GripError>;
    async fn resolve_latest(...) -> Result<String, GripError>;
}
```

A factory function `get_adapter()` dispatches to the right implementation based on the manifest entry type.

### Data Flow Diagram

```
User runs: grip sync / grip add / etc.
         │
         ▼
   CLI parsing (clap)
   OutputCfg (colors, verbosity)
         │
         ▼
   find_manifest_dir() — walk up to find grip.toml
   Manifest::load()    — parse TOML
   LockFile::load()    — read or create empty lock
         │
         ▼
   For each entry:
     1. Platform filter (Linux/macOS/Windows)?
     2. Tag filter (--tag)?
     3. Already in lock + on disk? → skip
     4. Dispatch to adapter
         │
         ▼
   ┌─────────────────────────┬──────────────────────────────┐
   │  Sequential (pkg mgr)   │   Concurrent (downloads)     │
   │  APT / DNF              │   GitHub / URL / Shell        │
   │ ──────────────────────  │ ──────────────────────────── │
   │ Check privs             │ GitHub: resolve semver,       │
   │ Update pkg index        │   find asset, download,       │
   │ Run pkg manager         │   extract, place binary       │
   │ Symlink to .bin/        │ URL: download, verify SHA,    │
   │                         │   extract, place binary       │
   │                         │ Shell: exec install_cmd       │
   └─────────────────────────┴──────────────────────────────┘
         │
         ▼
   Collect LockEntry results
   Update grip.lock (atomic write)
   Print colored summary
```

---

## CLI Commands

| Command | Purpose | Key Flags |
|---------|---------|-----------|
| `grip init` | Create `grip.toml` template, add `.bin/` to `.gitignore` | — |
| `grip add <name>` | Add binary/library to manifest and install immediately | `--source`, `--version`, `--repo`, `--url`, `--package`, `--binary`, `--library`, `--cmd`, `--allow-shell`, `--gpg-fingerprint`, `--sig-asset-pattern`, `--checksums-asset-pattern`, `--sig-url`, `--signed-checksums-url`, `--checksums-sig-url` |
| `grip sync` | Install all missing binaries concurrently | `--locked`, `--verify`, `--tag`, `--require-pins`, `--yes` |
| `grip check` | Verify `.bin/` matches `grip.lock` | `--tag` |
| `grip lock verify` | Re-hash `.bin/` against `grip.lock`; tamper detection for CI | — |
| `grip list` | Print lock file entries; `--all` also shows uninstalled declarations | `--all` |
| `grip remove <name>` | Remove from manifest, lock, and `.bin/` | `--library` |
| `grip update <name \| --all>` | Re-install and refresh one entry or all entries | `--all` |
| `grip outdated` | Fetch latest versions and show comparison | `--tag` |
| `grip doctor` | Detect orphaned entries, missing binaries, SHA256 drift, unpinned versions | — |
| `grip cache info` | Show cache stats | — |
| `grip cache clean` | Clear all cached downloads | — |
| `grip export` | Generate install commands for Dockerfile/shell/Makefile | `--format {shell,dockerfile,makefile}` |
| `grip run <cmd>` | Execute a command with `.bin/` prepended to PATH | — |
| `grip env` | Output shell code to add `.bin/` to PATH (for `eval`) | `--shell {bash,zsh,fish,sh}` |

**Global flags** (all commands): `-q/--quiet`, `-v/--verbose`, `--color {auto,always,never}`, `--root <DIR>`

---

## Configuration Files

| File | Purpose | Committed? |
|------|---------|------------|
| `grip.toml` | Manifest of all binary/library dependencies | Yes |
| `grip.lock` | Exact versions and SHA-256 checksums | Yes |
| `Cargo.toml` | grip's own Rust dependencies | Yes |
| `.env` vars | `GRIP_CACHE_DIR`, `NO_COLOR`, `HOME`, `SHELL` | No |

### grip.toml structure

```toml
[binaries.ripgrep]
source = "apt"           # or github, url, dnf, shell
package = "ripgrep"
version = "14.1.0"
platforms = ["linux"]
tags = ["search"]
required = true

[binaries.jq]
source = "github"
repo = "jqlang/jq"
version = "^1.7"         # semver range, pinned on first install

[libraries.openssl-devel]
source = "dnf"
package = "openssl-devel"
```

### grip.lock structure

```toml
[[binary]]
name = "ripgrep"
version = "14.1.0"
source = "apt"
url = null
sha256 = null
installed_at = "2024-03-21T16:18:00Z"

[[binary]]
name = "jq"
version = "1.7.1"
source = "github"
url = "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64"
sha256 = "e0165c9afcd6e81e86bf4f9... (sha256 hex)"
installed_at = "2024-03-21T16:20:00Z"
```

---

## Makefile Targets

| Target | Description |
|--------|-------------|
| `make build` | `cargo build --release` |
| `make test` | `cargo test` (unit tests, no Docker) |
| `make test-integration-shell` | Shell adapter suite (no network) |
| `make test-integration-apt` | APT adapter suite (Debian Bookworm container) |
| `make test-integration-dnf` | DNF adapter suite (Fedora 40 container) |
| `make test-integration-url` | URL adapter suite (network required) |
| `make test-integration-github` | GitHub adapter suite (network required) |
| `make test-integration` | All five suites sequentially |

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` 4 | CLI argument parsing with derive macros |
| `serde` 1 | Serialization framework |
| `toml` 0.8 | TOML parsing/serialization |
| `tokio` 1 | Async runtime |
| `reqwest` 0.12 | HTTP client for downloads |
| `flate2` / `bzip2` / `tar` / `zip` | Archive extraction |
| `sha2` + `hex` | SHA-256 hashing |
| `thiserror` / `anyhow` | Error handling |
| `indicatif` | Progress bars and spinners |
| `dialoguer` | User prompts |
| `chrono` | Timestamps in lock file |
| `glob` | Asset pattern matching |
| `async-trait` | Async methods in traits |
| `indexmap` | Ordered maps (preserves TOML key order) |
| `tempfile` | Atomic lock file writes |
| `futures` | `FuturesUnordered` for concurrent tasks |
| `semver` | Version range resolution for GitHub adapter |

---

## Notable Design Decisions

### Adapter Pattern
All install sources implement a single `SourceAdapter` async trait. Adding a new source (e.g., Homebrew) only requires implementing the trait — no changes to the dispatch logic.

### Sequential vs. Concurrent Installs
System package manager installs (APT, DNF) run **sequentially** to avoid concurrent writes to the system package database lock (`/var/lib/dpkg/lock`, `rpm.lock`). Download-based installs (GitHub, URL, Shell) run **concurrently** via `FuturesUnordered`.

### Fail-Fast Privilege Checks
Checks for root/passwordless sudo upfront before any install, rather than silently hanging or prompting mid-run.

### PATH-Based Binary Detection
The DNF adapter and `installer.rs` discover binaries via direct PATH directory search rather than shelling out to `which`, which is not available in all container environments (e.g., minimal Fedora images).

### Atomicity
Lock file writes use a tempfile + rename pattern to prevent corruption if the process crashes mid-write.

### Reproducibility
Every installed binary's SHA-256 is recorded. `grip sync --locked` (CI mode) fails if the lock would change, catching any version drift.

### TOML Preservation
Uses `indexmap::IndexMap` instead of `HashMap` to preserve the user's key ordering in `grip.toml` when re-serializing.

### Docker Workflow
`grip export --format dockerfile` generates native `RUN apt-get install` / `RUN curl` commands from the lock file, so Docker images don't need grip installed at build time.

### Shell Adapter SHA-256
After a shell `install_cmd` succeeds, grip computes the SHA-256 of the binary placed in `.bin/` (if any) and records it in `grip.lock`, enabling `grip check` to verify shell-installed binaries just like download-based ones.

### Supply Chain Attack Protections
Four layered controls are implemented in `src/`:

1. **`allow_shell` guard** (`adapters/shell.rs`) — shell entries are blocked unless `allow_shell = true` is explicitly set in `grip.toml`. Even then, an interactive TTY prompt shows the command and asks for confirmation. Protects against a malicious PR adding an `install_cmd`.

2. **GPG signature verification** (`gpg.rs`) — two modes: direct binary signature (Mode 1) and signed checksums file (Mode 2, used by HashiCorp, Go, jq, etc.). Shared by `adapters/github.rs` and `adapters/url.rs`. Both modes use `verify_gpg_signature_with_cmd` / `verify_signed_checksums_with_cmd` internally, which accept a `gpg_cmd` parameter so tests can pass a non-existent binary name instead of mutating `PATH`.

3. **`grip lock verify`** (`lock_verify.rs`) — reads `grip.lock` directly (no manifest, no network), re-hashes every `.bin/` binary, and reports mismatches. Separates "is my setup complete?" (`grip check`) from "was anything tampered with after install?" (`grip lock verify`).

4. **`--require-pins`** (`installer.rs`) — checked at the top of `run_install` before any network call. Uses `BinaryEntry::is_version_pinned()` from `config/manifest.rs`. `url` entries are always considered pinned (the URL is the artifact identifier); all other sources require an explicit `version` field.

### Real apt/dnf Version Resolution
`grip outdated` queries `apt-cache policy` (APT) and `dnf info` (DNF) to retrieve the actual repository candidate version rather than reporting a static `"latest"` string. Both fall back gracefully when the package manager is unavailable.

### Cache Strategy
Downloads are keyed by SHA-256 of the URL string. Configurable via `$GRIP_CACHE_DIR`; setting it to empty disables caching entirely.

### Error UX
`GripError` enum has a `.hint()` method on every variant that provides actionable guidance (e.g., "Run `grip init` in your project directory").

---

## Test Suite

grip has comprehensive integration tests covering all 5 adapters:

| Suite | Container | Network | Tests |
|-------|-----------|---------|-------|
| `integration_shell` | Debian Bookworm | No | 7 |
| `integration_apt` | Debian Bookworm | No (apt cache) | 11 |
| `integration_dnf` | Fedora 40 | No (dnf cache) | 11 |
| `integration_url` | Debian Bookworm | Yes | 6 |
| `integration_github` | Debian Bookworm | Yes | 8 |

Each suite runs inside a dedicated Docker container to ensure clean, reproducible test environments. Tests are gated by `GRIP_INTEGRATION_TESTS=1` and `#[ignore]` to prevent accidental execution on the host.

---

## Example Usage

```bash
# Initialize a project
grip init

# Add a tool from GitHub (auto-installs)
grip add BurntSushi/ripgrep

# Add a system package
grip add ripgrep --source apt

# Install all declared tools (safe to run repeatedly)
grip sync

# CI mode: fail if lock would change
grip sync --locked

# Verify installed binaries match the lock
grip check

# Use an installed binary
eval "$(grip env)"
rg --version
# OR
grip run rg --version

# Check for updates
grip outdated

# Generate Dockerfile install commands
grip export --format dockerfile

# Remove a tool
grip remove ripgrep
```
