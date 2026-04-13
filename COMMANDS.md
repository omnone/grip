# grip — CLI reference

## Global flags

These work with every command:

| Flag | Description |
|---|---|
| `-q, --quiet` | Suppress spinners and decorative output |
| `-v, --verbose` | More detail on errors |
| `--color auto\|always\|never` | ANSI color control; default `always` (`NO_COLOR` still disables) |
| `--root <DIR>` | Override project root (skips `grip.toml` walk; useful in containers) |

---

## Commands

### `grip init`

Creates `grip.toml` from a template and adds `.bin/` to `.gitignore`. No flags.

---

### `grip add <name>`

Adds a binary entry to `grip.toml` and installs it immediately. On Linux the default source is `apt` or `dnf` when available. Use `--source github` or the `owner/repo` shorthand to force GitHub.

```sh
grip add ripgrep                                    # apt/dnf on Linux (default)
grip add BurntSushi/ripgrep                         # GitHub Releases shorthand
grip add jq@1.7.1 --repo jqlang/jq --source github  # pin a version
grip add libssl-dev --library                        # system library (no executable)
grip add mytool --source url --url https://example.com/mytool.tar.gz
grip add mytool --source shell --allow-shell \
  --cmd 'curl -fsSL https://example.com/install.sh | sh -s -- --dir $GRIP_BIN_DIR'
```

| Flag | Description |
|---|---|
| `--source <src>` | `github`, `url`, `apt`, `dnf`, `shell` (default: OS-specific) |
| `--version <ver>` | Pin a specific version |
| `--repo <owner/repo>` | GitHub repo (required for `--source github` unless using `owner/repo` shorthand) |
| `--url <url>` | Download URL (required for `--source url`) |
| `--package <pkg>` | Package name for apt/dnf (defaults to binary name) |
| `--binary <cmd>` | On-PATH command for apt/dnf when it differs from NAME (e.g. `rg` for `ripgrep`) |
| `--library` | Add to `[libraries]` instead of `[binaries]` (apt/dnf only; no executable required) |
| `--cmd <CMD>` | Shell command to run for `--source shell` (required for that source; `$GRIP_BIN_DIR` is set) |
| `--allow-shell` | Set `allow_shell = true` on the new shell entry (required to run shell installs) |
| `--gpg-fingerprint <FP>` | GPG key fingerprint to verify GitHub/URL release signatures |
| `--sig-asset-pattern <GLOB>` | Glob to find the detached signature asset in a GitHub release (e.g. `"*.asc"`); auto-detected if omitted |
| `--checksums-asset-pattern <GLOB>` | Glob to find a signed checksums file in a GitHub release (e.g. `"*SHA256SUMS"`); activates signed-checksums verification |
| `--sig-url <URL>` | URL of the detached GPG signature file (URL source only) |
| `--signed-checksums-url <URL>` | URL of a signed checksums file (URL source only); activates signed-checksums verification |
| `--checksums-sig-url <URL>` | URL of the GPG signature for the checksums file (URL source only; required with `--signed-checksums-url`) |

For a full explanation of the GPG verification modes, see [SECURITY.md](SECURITY.md).

---

### `grip sync`

Downloads and installs any missing binaries from `grip.toml` into `.bin/` concurrently. Already-installed binaries are skipped. Download-based installs use a local cache so archives are not re-downloaded on repeat runs.

```sh
grip sync
grip sync --locked                  # CI mode: fail if lock would change
grip sync --locked --require-pins   # also fail if any entry has no version pin
grip sync --tag dev                 # only entries tagged "dev"
grip sync --verify                  # re-verify SHA256 of already-installed binaries
grip sync --yes                     # skip interactive confirmation for shell installs
```

| Flag | Description |
|---|---|
| `--locked` | Fail if `grip.lock` would change; enforces reproducibility in CI |
| `--verify` | Re-check SHA256 of on-disk binaries against `grip.lock` |
| `--tag <tag>` | Only install entries that carry this tag |
| `--require-pins` | Fail before touching the network if any entry has no version pin (prevents silent auto-upgrades in CI) |
| `--yes` | Skip the interactive confirmation prompt for shell installs |

---

### `grip check`

Verifies `.bin/` against `grip.lock` without installing or modifying anything. Checks binary existence, version pins, and SHA256 checksums.

```sh
grip check
grip check --tag ci
```

| Flag | Description |
|---|---|
| `--tag <tag>` | Only check entries that carry this tag |

Exits `0` if all required entries pass; `1` if any required entry fails.

---

### `grip outdated`

Fetches the latest available version for every declared binary and shows a comparison table.

- **GitHub** entries: queries the GitHub Releases API.
- **apt** entries: queries `apt-cache policy` for the repository candidate version.
- **dnf** entries: queries `dnf info` for the latest available version.
- **url / shell** entries: compares the lock version against the manifest pin; no network query.

```sh
grip outdated
grip outdated --tag dev
```

| Flag | Description |
|---|---|
| `--tag <tag>` | Only check entries that carry this tag |

---

### `grip update <name | --all>`

Re-installs one or all binaries and libraries from the manifest, fetching the latest version, and refreshes their lock entries.

```sh
grip update ripgrep          # update a single binary
grip update libssl-dev       # update a single library
grip update --all            # update every entry in grip.toml concurrently
```

| Flag | Description |
|---|---|
| `--all` | Update every binary and library declared in `grip.toml` |

When `--all` is used, download-based entries (GitHub, URL, shell) are updated concurrently; system packages (apt, dnf) are updated sequentially. A summary line is printed after all updates complete.

---

### `grip remove <name>`

Removes an entry from `grip.toml`, `grip.lock`, and `.bin/`.

```sh
grip remove ripgrep
grip remove libssl-dev --library   # remove a library entry
```

| Flag | Description |
|---|---|
| `--library` | Remove from `[libraries]` instead of `[binaries]` |

---

### `grip list`

Prints entries from `grip.lock` with their versions, sources, and install timestamps, in separate sections for binaries and libraries.

```sh
grip list          # installed entries only (from grip.lock)
grip list --all    # all declared entries; uninstalled ones are highlighted
```

| Flag | Description |
|---|---|
| `--all` | Also show entries declared in `grip.toml` that have not yet been installed, with a `not installed` status column |

---

### `grip doctor`

Checks consistency between `grip.toml`, `grip.lock`, and `.bin/`. No flags.

Detects:
- Orphaned lock entries (in lock but not in manifest)
- Binaries declared but not yet installed
- Binary on disk missing from `.bin/`
- SHA256 drift — binary on disk no longer matches `grip.lock` (possible post-install tampering)
- Lock entries with no sha256 for sources that always record one (`github`, `url`, `shell`) — may indicate the lock was hand-edited
- Unpinned entries — entries with no version pin that could silently auto-upgrade
- Libraries in the lock but not found on the system

---

### `grip cache`

Manages the local download cache (`~/.cache/grip/downloads/` by default).

```sh
grip cache info    # show entry count and total disk usage
grip cache clean   # remove all cached downloads
```

Set `GRIP_CACHE_DIR` to override the cache location. Set it to an empty string to disable caching entirely:

```sh
GRIP_CACHE_DIR=/tmp/my-cache grip sync   # custom cache location
GRIP_CACHE_DIR= grip sync                # disable cache
```

---

### `grip lock verify`

Re-hashes every binary in `.bin/` and compares the result against the sha256 recorded in `grip.lock`. Does not re-download anything or read `grip.toml` — purely a tamper-detection command.

```sh
grip lock verify
```

Output:

```
  grip lock verify

  ✓  jq
  ✓  rg
  ⚠  fd  (no sha256 in lock — cannot verify)

  OK  (2 verified, 1 without sha256)
```

Exits `1` if any binary's hash does not match. Suitable for CI pipelines. For the recommended CI setup combining `--locked`, `--require-pins`, and `grip lock verify`, see [SECURITY.md](SECURITY.md).

---

### `grip export`

Reads `grip.toml` and `grip.lock` and prints native install commands. Versions are taken from the lock file when available. Shell entries are emitted as comments — they cannot be auto-exported.

```sh
grip export                         # shell script (default)
grip export --format dockerfile     # Dockerfile RUN lines
grip export --format makefile       # Makefile target
```

| Flag | Description |
|---|---|
| `--format <fmt>` | `shell` (default), `dockerfile`, `makefile` |

**Example output — `--format dockerfile`:**

```dockerfile
# Generated by grip export --format dockerfile
RUN apt-get update -y && apt-get install -y --no-install-recommends \
    ripgrep=14.1.0 \
    libssl-dev=3.0.2 \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL -o /usr/local/bin/jq \
    "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64" \
    && chmod +x /usr/local/bin/jq
```

---

### `grip run <cmd> [args]`

Runs a command with `.bin/` prepended to `PATH`. Useful without shell integration.

```sh
grip run jq '.name' package.json
grip run rg --version
```

---

### `grip env`

Prints shell code that adds `.bin/` to `PATH`. Designed to be captured by `eval`.

```sh
eval "$(grip env)"               # bash / zsh
grip env --shell fish | source   # fish
```

| Flag | Description |
|---|---|
| `--shell <shell>` | `bash`, `zsh`, `fish`, `sh` (auto-detected from `$SHELL` if omitted) |

---

## Shell integration

**Add `.bin/` to `PATH` permanently** — add to your shell profile:

```sh
# ~/.bashrc or ~/.zshrc
eval "$(grip env)"

# ~/.config/fish/config.fish
grip env --shell fish | source
```

**Or run tools without touching `PATH`:**

```sh
grip run jq '.name' package.json
grip run rg --version
```

---

## grip.toml reference

### GitHub Releases

Downloads a release asset for the current OS and architecture. Versions can be pinned exactly or expressed as semver ranges.

```toml
[binaries.jq]
source        = "github"
repo          = "jqlang/jq"
version       = "1.7.1"           # exact pin
# version     = "^1.7"            # semver range: resolves to latest 1.x
asset_pattern = "jq-linux-amd64"  # optional glob to select the right asset
binary        = "jq"              # optional: name of the binary inside the archive
```

**Semver ranges** (`^`, `~`, `>=`, `>`, `<`, `<=`, `*`) are resolved at install time against the GitHub releases list. The concrete version is written to `grip.lock`; `--locked` mode pins to that exact version on subsequent installs. If no `asset_pattern` is set, grip falls back to a platform-aware heuristic (matches on OS + architecture strings in the asset filename).

**Optional GPG verification fields:**

```toml
[binaries.terraform]
source                  = "github"
repo                    = "hashicorp/terraform"
version                 = "1.7.0"
gpg_fingerprint         = "34365D9472D7468F"   # maintainer's key fingerprint
checksums_asset_pattern = "*SHA256SUMS"         # signed checksums file (Mode 2)
sig_asset_pattern       = "*SHA256SUMS.sig"     # signature of the checksums file
# For direct binary signature (Mode 1), omit checksums_asset_pattern:
# sig_asset_pattern = "*.asc"
```

See [SECURITY.md](SECURITY.md) for a full explanation of Mode 1 vs Mode 2 verification.

---

### Direct URL

```toml
[binaries.mytool]
source = "url"
url    = "https://example.com/releases/mytool-linux-amd64.tar.gz"
sha256 = "abc123..."  # optional hex digest; verified after download
binary = "mytool"     # optional: name of the binary inside the archive

# Optional GPG verification:
gpg_fingerprint      = "AF436C3B58B2E3B2"
# Mode 1 — direct binary signature:
sig_url              = "https://example.com/releases/mytool-linux-amd64.tar.gz.sig"
# Mode 2 — signed checksums file (takes precedence over sig_url):
# signed_checksums_url = "https://example.com/SHA256SUMS"
# checksums_sig_url    = "https://example.com/SHA256SUMS.sig"
```

---

### APT / DNF

```toml
[binaries.ripgrep]
source  = "apt"       # or "dnf"
package = "ripgrep"   # defaults to the entry name
binary  = "rg"        # optional: on-PATH command when it differs from the table key
version = "14.1.0"    # optional: exact package version
```

grip requires root or passwordless `sudo` to invoke `apt-get` / `dnf`. It checks privileges once before any install and fails with a clear message rather than prompting for a password mid-run.

---

### Libraries (no executable)

System packages that install headers or shared libraries but produce no binary belong in `[libraries]`. Installed via the system package manager; no `.bin/` symlink is created.

```toml
[libraries.libssl-dev]
source  = "apt"
package = "libssl-dev"
version = "3.0.2"      # optional

[libraries.openssl-devel]
source  = "dnf"
package = "openssl-devel"
```

Add with: `grip add libssl-dev --library`

---

### Shell

Runs an arbitrary shell command. `$GRIP_BIN_DIR` is set to the project's `.bin/` directory so the command can place the binary there.

**Shell installs are blocked by default.** You must explicitly set `allow_shell = true` in `grip.toml` to permit execution. This protects against arbitrary code execution if `grip.toml` is compromised (e.g., a malicious PR that adds a shell entry).

```toml
[binaries.mytool]
source      = "shell"
install_cmd = "curl -fsSL https://example.com/install.sh | bash -s -- --dir $GRIP_BIN_DIR"
version     = "1.0"    # metadata only; not enforced by grip
allow_shell = true     # must be explicitly set; false or absent blocks execution
```

Add from the CLI with `--cmd` and `--allow-shell`:

```sh
grip add mytool --source shell \
  --cmd 'curl -fsSL https://example.com/install.sh | bash -s -- --dir $GRIP_BIN_DIR' \
  --version 1.0 \
  --allow-shell
```

Even with `allow_shell = true`, grip shows the `install_cmd` and prompts for confirmation before running it (when a TTY is attached). Use `grip sync --yes` to suppress the prompt in automation.

After installation, grip computes the SHA-256 of the binary placed in `.bin/` (if any) and records it in `grip.lock`. This allows `grip check` to verify the binary has not been tampered with on subsequent runs.

---

### Common optional fields

All entry types support these fields:

```toml
platforms    = ["linux", "darwin"]       # restrict to specific OSes (linux, darwin, windows)
tags         = ["dev", "ci"]             # selective installs: grip sync --tag dev
required     = false                     # warn instead of failing on error (default: true)
post_install = "chmod +x .bin/mytool"   # shell command to run after a successful install
```

---

## Reproducibility and CI

`grip.lock` records the exact version, download URL, and SHA-256 checksum of every installed binary. Commit it alongside `grip.toml`.

In CI, use `--locked` to enforce the lock file and fail if it would change:

```sh
grip sync --locked
```

Use `grip outdated` to see what has newer versions available, then `grip update <name>` to upgrade one entry or `grip update --all` to upgrade everything at once and refresh all lock entries.

Use `grip export --format dockerfile` to generate a Dockerfile snippet from the lock file without requiring grip in the image.
