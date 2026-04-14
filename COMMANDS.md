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
```

| Flag | Description |
|---|---|
| `--source <src>` | `github`, `url`, `apt`, `dnf` (default: OS-specific) |
| `--version <ver>` | Pin a specific version |
| `--repo <owner/repo>` | GitHub repo (required for `--source github` unless using `owner/repo` shorthand) |
| `--url <url>` | Download URL (required for `--source url`) |
| `--package <pkg>` | Package name for apt/dnf (defaults to binary name) |
| `--binary <cmd>` | On-PATH command for apt/dnf when it differs from NAME (e.g. `rg` for `ripgrep`) |
| `--library` | Add to `[libraries]` instead of `[binaries]` (apt/dnf only; no executable required) |
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
```

| Flag | Description |
|---|---|
| `--locked` | Fail if `grip.lock` would change; enforces reproducibility in CI |
| `--verify` | Re-check SHA256 of on-disk binaries against `grip.lock` |
| `--tag <tag>` | Only install entries that carry this tag |
| `--require-pins` | Fail before touching the network if any entry has no version pin (prevents silent auto-upgrades in CI) |

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
- **url** entries: compares the lock version against the manifest pin; no network query.

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

When `--all` is used, download-based entries (GitHub, URL) are updated concurrently; system packages (apt, dnf) are updated sequentially. A summary line is printed after all updates complete.

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
- Lock entries with no sha256 for sources that always record one (`github`, `url`) — may indicate the lock was hand-edited
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

Reads `grip.toml` and `grip.lock` and prints native install commands. Versions are taken from the lock file when available.

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

### `grip sbom`

Reads `grip.lock` and emits a machine-readable Software Bill of Materials. No network access required.

```sh
grip sbom                              # CycloneDX 1.5 JSON to stdout (default)
grip sbom --format spdx                # SPDX 2.3 JSON to stdout
grip sbom --output sbom.json           # CycloneDX to file
grip sbom --format spdx -o sbom.spdx.json
```

| Flag | Description |
|---|---|
| `--format <fmt>` | `cyclonedx` (default) or `spdx` |
| `-o, --output <FILE>` | Write to FILE instead of stdout |

**CycloneDX output** (spec version 1.5) — each lock entry becomes a `component` with:
- `type`, `name`, `version` (leading `v` stripped per purl spec)
- `purl` — e.g. `pkg:github/jqlang/jq@1.7.1`, `pkg:deb/debian/libssl-dev@3.0.2`
- `hashes` — SHA-256 from the lock file (GitHub and URL sources only)
- `externalReferences` — download URL (GitHub and URL sources only)

**SPDX output** (spec version 2.3) — each entry becomes a `package` with a `purl` external reference, `checksums` when available, and `NOASSERTION` download location for system packages (apt/dnf).

---

### `grip audit`

Queries the [OSV vulnerability database](https://osv.dev/) for known CVEs and advisories affecting your installed tools. Sends a single batch request using the purl of each `grip.lock` entry.

```sh
grip audit                 # exit 1 if any findings (default)
grip audit --no-fail       # report findings but always exit 0
```

| Flag | Description |
|---|---|
| `--no-fail` | Exit 0 even when vulnerabilities are found |

Exits `1` if any vulnerabilities are found (suitable for CI). Use `--no-fail` to report without blocking. Run `grip update <name>` to upgrade a vulnerable tool.

---

### `grip suggest`

Scans the project and (optionally) source code for CLI tool references that are not yet declared in `grip.toml`, then prints suggested `grip add` commands.

**Default scan sources** (no flags required):

- `Makefile` at the project root
- `scripts/` directory (shell scripts, Python, etc.)
- `.github/workflows/` CI YAML files

**Optional source-code scan** — pass `--path` to also detect tools referenced via subprocess/exec APIs in Rust, Python, JavaScript/TypeScript, Go, and Ruby source files, as well as `/bin/<name>` path literals in any file type.

```sh
grip suggest                              # scan Makefile, scripts/, workflows/
grip suggest --history                    # also scan shell history
grip suggest --path src/                  # also scan source code under src/
grip suggest --path src/ --path scripts/  # multiple paths
grip suggest --check                      # exit 1 if any suggestions are found (CI)
```

| Flag | Description |
|---|---|
| `-p, --path <PATH>` | Source-code path to scan for binary invocations (repeatable). Detects subprocess API calls and `/bin/<name>` path literals. |
| `--history` | Also scan shell history files (`~/.bash_history`, `~/.zsh_history`, Fish history). Off by default. |
| `--check` | Exit with status `1` if any unmanaged tools are found. Useful in CI to enforce that all tools are declared in `grip.toml`. |

**Example output:**

```
  Suggested additions to grip.toml

  ✦  fd                grip add sharkdp/fd --source github
     ↳ found in: scripts/build.sh, .github/workflows/ci.yml

  ?  ffmpeg
     ↳ found in: src/encoder.py (2×)
```

Entries marked `✦` are in grip's curated tool list with a known GitHub source. Entries marked `?` were detected but have no known source — you can still add them manually.

Tools already declared in `grip.toml` are excluded from the output. System builtins (`grep`, `sed`, `awk`, `curl`, etc.) are always filtered out.

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
extra_binaries = ["jqfmt"]        # optional: additional binaries to extract from the same archive
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
source         = "url"
url            = "https://example.com/releases/mytool-linux-amd64.tar.gz"
sha256         = "abc123..."   # optional hex digest; verified after download
binary         = "mytool"      # optional: name of the binary inside the archive
extra_binaries = ["mytoolfmt"] # optional: additional binaries to extract from the same archive

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

[binaries.ffmpeg]
source         = "apt"
package        = "ffmpeg"
extra_binaries = ["ffprobe", "ffplay"]  # additional binaries installed by the same package
```

When `extra_binaries` is set, grip symlinks each listed binary from its on-PATH location into `.bin/` alongside the primary binary. The lock entry records all extra binary names so `grip check` can verify they are all present.

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
