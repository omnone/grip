# grip — Codebase Overview

> **grip** is a Rust-based CLI tool that manages project-local binary and library dependencies. It solves the problem of coordinating tool versions across development teams, CI/CD pipelines, and Docker builds by declaring tools in a `grip.toml` manifest, installing them into an isolated `.bin/` directory, and locking exact versions + SHA-256 checksums in `grip.lock`.

---

## Directory Structure

```
binaries-manager/
├── src/
│   ├── main.rs                    # Entry point and all command implementations (~1,396 lines)
│   ├── cli.rs                     # Clap CLI definitions (158 lines)
│   ├── installer.rs               # Core install orchestration (455 lines)
│   ├── checker.rs                 # Verification logic (272 lines)
│   ├── error.rs                   # GripError enum and formatting (113 lines)
│   ├── output.rs                  # Terminal styling and ANSI colors (106 lines)
│   ├── platform.rs                # OS/arch detection (69 lines)
│   ├── privilege.rs               # Sudo/root detection (48 lines)
│   ├── bin_dir.rs                 # .bin/ directory management (62 lines)
│   ├── cache.rs                   # Download cache logic (188 lines)
│   ├── checksum.rs                # SHA-256 verification (55 lines)
│   ├── adapters/
│   │   ├── mod.rs                 # SourceAdapter trait definition (58 lines)
│   │   ├── github.rs              # GitHub Releases adapter (382 lines)
│   │   ├── url.rs                 # Direct URL downloader (191 lines)
│   │   ├── apt.rs                 # APT package manager adapter (205 lines)
│   │   ├── dnf.rs                 # DNF package manager adapter (205 lines)
│   │   └── shell.rs               # Shell command executor (76 lines)
│   └── config/
│       ├── mod.rs                 # Config module root
│       ├── manifest.rs            # grip.toml TOML structs (241 lines)
│       └── lockfile.rs            # grip.lock structs and I/O (92 lines)
├── Cargo.toml                     # Rust project metadata and dependencies
├── Cargo.lock                     # Locked Rust dependency versions
├── grip.toml                      # Example manifest
├── grip.lock                      # Example lock file
├── README.md                      # User documentation
└── Makefile                       # Single `build` target (cargo build --release)
```

**Total: ~2,922 lines of Rust across 20 source files.**

---

## Key Files and Their Roles

| File | Responsibility |
|------|---------------|
| `main.rs` | Routes 14 CLI commands; contains the implementation logic for each |
| `cli.rs` | Clap-derived structs for all flags and subcommands |
| `installer.rs` | Concurrent adapter execution, lock file updates, platform/tag filtering |
| `checker.rs` | Validates `.bin/` against `grip.lock` (version, SHA256, presence) |
| `adapters/mod.rs` | `SourceAdapter` async trait that all 5 adapters implement |
| `adapters/github.rs` | Resolves semver ranges, downloads GitHub release assets, extracts archives |
| `adapters/apt.rs` | Invokes APT with privilege escalation checks, symlinks binary into `.bin/` |
| `adapters/dnf.rs` | Same as APT but for DNF/RPM systems |
| `adapters/url.rs` | HTTP downloads with optional SHA256 verification and caching |
| `adapters/shell.rs` | Executes user-supplied shell commands (`install_cmd`) with `GRIP_BIN_DIR` set |
| `config/manifest.rs` | TOML deserialization for all entry types (Github, Apt, Dnf, Url, Shell) |
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
  → skip already-installed entries
  → spawn async adapter tasks (FuturesUnordered)
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
   ┌────────────┬──────────────┬──────────────┬───────────┐
   │  GitHub    │   APT/DNF    │     URL      │   Shell   │
   │ ─────────  │ ──────────── │ ──────────── │ ───────── │
   │ Resolve    │ Check privs  │ Download     │ Execute   │
   │ version    │ Update index │ Verify SHA   │ install   │
   │ Find asset │ Run pkg mgr  │ Extract      │ command   │
   │ Download   │ Symlink      │ Place binary │           │
   │ Extract    │              │              │           │
   │ Place      │              │              │           │
   └────────────┴──────────────┴──────────────┴───────────┘
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
| `grip add <name>` | Add binary/library to manifest and install immediately | `--source`, `--version`, `--repo`, `--url`, `--package`, `--binary`, `--library` |
| `grip sync` | Install all missing binaries concurrently | `--locked` (CI mode), `--verify`, `--tag` |
| `grip check` | Verify `.bin/` matches `grip.lock` | `--tag` |
| `grip list` | Print all lock file entries with metadata | — |
| `grip remove <name>` | Remove from manifest, lock, and `.bin/` | `--library` |
| `grip update <name>` | Re-install and refresh a single entry | — |
| `grip outdated` | Fetch latest versions and show comparison | `--tag` |
| `grip doctor` | Detect orphaned entries, missing binaries, SHA256 drift | — |
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
url = "https://..."
sha256 = "abc123..."
installed_at = "2024-03-21T16:18:00Z"
```

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

### Concurrent Installs
Uses `FuturesUnordered` with `tokio` to run all adapter tasks concurrently, with `indicatif` multi-progress for live status without visual chaos.

### Fail-Fast Privilege Checks
Checks for root/passwordless sudo upfront before any install, rather than silently hanging or prompting mid-run.

### Atomicity
Lock file writes use a tempfile + rename pattern to prevent corruption if the process crashes mid-write.

### Reproducibility
Every installed binary's SHA-256 is recorded. `grip sync --locked` (CI mode) fails if the lock would change, catching any version drift.

### TOML Preservation
Uses `indexmap::IndexMap` instead of `HashMap` to preserve the user's key ordering in `grip.toml` when re-serializing.

### Docker Workflow
`grip export --format dockerfile` generates native `RUN apt-get install` / `RUN curl` commands from the lock file, so Docker images don't need grip installed at build time.

### Cache Strategy
Downloads are keyed by SHA-256 of the URL string. Configurable via `$GRIP_CACHE_DIR`; setting it to empty disables caching entirely.

### Error UX
`GripError` enum has a `.hint()` method on every variant that provides actionable guidance (e.g., "Run `grip init` in your project directory").

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

---

## Code Quality Notes

- **Type safety**: `Result<T, GripError>` throughout; minimal panicking
- **Async**: Proper `async/await` with `tokio`; no blocking calls on the async executor
- **Error messages**: Rich, actionable hints per error variant
- **No tests**: Validation is done via CLI usage (no unit/integration test files)
- **Largest file**: `main.rs` at ~1,396 lines — mixes command routing with implementation; a candidate for future refactoring into per-command modules
