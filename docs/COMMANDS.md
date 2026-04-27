# grip — CLI reference

> For copy-pasteable recipes see [EXAMPLES.md](EXAMPLES.md).

## Quick reference

| Command | What it does |
|---|---|
| [`grip init [PATH]`](#grip-init-path) | Create `grip.toml` and `.gitignore` entry; optionally import tools from a Dockerfile |
| [`grip add <name>...`](#grip-add-name) | Add one or more tools to `grip.toml` and install them immediately |
| [`grip remove <name>...`](#grip-remove-name) | Remove tools from `grip.toml`, `grip.lock`, and `.bin/` |
| [`grip lock`](#grip-lock) | Resolve versions and write `grip.lock` without installing anything |
| [`grip lock --check`](#grip-lock) | Assert the lockfile is up to date; exit `1` if a re-lock would change it |
| [`grip lock --upgrade`](#grip-lock) | Re-resolve all entries to their latest available version |
| [`grip lock --upgrade-package <n>`](#grip-lock) | Re-resolve a single entry to its latest available version |
| [`grip lock verify`](#grip-lock-verify) | Re-hash every `.bin/` binary and compare against `grip.lock` |
| [`grip lock pin`](#grip-lock-pin) | Write installed versions from `grip.lock` back into `grip.toml` |
| [`grip sync`](#grip-sync) | Install all missing tools from `grip.toml` into `.bin/` |
| [`grip sync --check`](#grip-sync) | Verify `.bin/` matches `grip.lock` without installing anything |
| [`grip sync --locked`](#grip-sync) | Install; fail if `grip.lock` would change (CI mode) |
| [`grip run [--] <cmd>`](#grip-run----cmd-args) | Run a command with `.bin/` on `PATH`; auto-syncs by default |
| [`grip tree`](#grip-tree) | List installed entries with versions, sources, and timestamps |
| [`grip cache <sub>`](#grip-cache) | Manage the local download cache (`dir`, `size`, `clean`, `prune`) |
| [`grip export`](#grip-export) | Export install commands or a CycloneDX/SPDX SBOM from `grip.lock` |
| [`grip audit`](#grip-audit) | Check installed tools against the OSV vulnerability database |
| [`grip suggest`](#grip-suggest) | Discover unmanaged tool references in your project |
| [`grip env`](#grip-env) | Print shell code to add `.bin/` to `PATH` |

**Global flags** available on every command: `--project <DIR>`, `--directory <DIR>`, `--offline`, `--no-cache`, `--cache-dir <DIR>`, `--no-progress`, `-q/--quiet`, `-v/--verbose`, `--color`. → [details](#global-flags)

---

## Global flags

These work with every command:

| Flag | Description |
|---|---|
| `-q, --quiet` | Suppress spinners and decorative output |
| `-v, --verbose` | More detail on errors |
| `--color auto\|always\|never` | ANSI color control; default `auto` (respects `NO_COLOR`) |
| `--project <DIR>` | Override the project root (skips `grip.toml` walk; useful in containers) |
| `--directory <DIR>` | Change to DIR before running any command (relative paths resolved from DIR) |
| `--offline` | Disable all network access; rely only on the local cache and installed state |
| `--no-cache` | Bypass the local download cache for this run |
| `--no-progress` | Hide all progress output (spinners, progress bars) |
| `--cache-dir <DIR>` | Override the cache directory; also settable via `GRIP_CACHE_DIR` |

---

## Commands

### `grip init [PATH]`

Creates `grip.toml` and adds `.bin/` to `.gitignore`. When a Dockerfile is detected (or
passed via `--from`), grip parses it for `RUN apt-get install` / `RUN dnf install` lines,
classifies each package, verifies the results against a curated tool list and the host
package manager, and offers to import the verified set into `grip.toml`.

If `PATH` is given, grip initialises the project there instead of the current directory.

```sh
grip init                          # auto-detect Dockerfile in cwd
grip init myproject/               # initialise in myproject/
grip init --from path/Dockerfile   # explicit Dockerfile path (repeatable)
grip init --yes                    # skip confirmation prompt
grip init --bare                   # blank template only; no Dockerfile scanning
grip init --offline                # skip GitHub repo-existence checks (Layer C)
```

#### Three-layer verification policy

Before suggesting a package for import, `grip init` runs three verification layers:

| Layer | What it does | Network? |
|---|---|---|
| A — curated list | Matches the package name against the built-in tool registry; confirms existence and fixes the on-PATH binary name (e.g. `ripgrep` → `rg`) | No |
| B — host package manager | Runs `apt-cache show <pkg>` or `dnf info <pkg>` to confirm the package exists and determine its section (lib vs utils) | No |
| C — GitHub repo existence | For tools identified in Layer A as having a GitHub source, sends a `HEAD` request to `api.github.com/repos/<owner>/<repo>` | Yes (opt-out with `--offline`) |

Packages that pass neither Layer A nor Layer B are listed as **Skipped (not verified)** and
are never written to `grip.toml`. Review them with `grip add <name>` if you need them.

| Flag | Description |
|---|---|
| `--from <PATH>`, `-f` | Explicit Dockerfile path to import from; may be repeated |
| `--yes`, `-y` | Skip the confirmation prompt (also default for non-TTY) |
| `--bare` | Never scan Dockerfiles; produce a blank template only |
| `--offline` | Disable Layer C (GitHub repo check); rely on curated list and host package manager only |

---

### `grip add <name>...`

Adds one or more binary or library entries to `grip.toml` and installs them immediately.
On Linux the default source is `apt` or `dnf` when available. Use `--source github` or the
`owner/repo` shorthand to force GitHub.

Pass `--no-sync` to write to `grip.toml` and `grip.lock` without installing.
Pass `--frozen` to write to `grip.toml` only, leaving `grip.lock` unchanged.

```sh
grip add ripgrep                                    # apt/dnf on Linux (default)
grip add BurntSushi/ripgrep                         # GitHub Releases shorthand
grip add jq@1.7.1 --repo jqlang/jq --source github  # pin a version
grip add jq ripgrep fd                              # add multiple tools at once
grip add libssl-dev --library                        # system library (no executable)
grip add mytool --source url --url https://example.com/mytool.tar.gz
grip add BurntSushi/ripgrep --no-sync               # add to manifest, skip install
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
| `--no-sync` | Write to `grip.toml` and `grip.lock` but skip installing into `.bin/` |
| `--frozen` | Write to `grip.toml` only; do not update `grip.lock` |
| `--gpg-fingerprint <FP>` | GPG key fingerprint to verify GitHub/URL release signatures |
| `--sig-asset-pattern <GLOB>` | Glob to find the detached signature asset in a GitHub release (e.g. `"*.asc"`); auto-detected if omitted |
| `--checksums-asset-pattern <GLOB>` | Glob to find a signed checksums file in a GitHub release (e.g. `"*SHA256SUMS"`); activates signed-checksums verification |
| `--sig-url <URL>` | URL of the detached GPG signature file (URL source only) |
| `--signed-checksums-url <URL>` | URL of a signed checksums file (URL source only); activates signed-checksums verification |
| `--checksums-sig-url <URL>` | URL of the GPG signature for the checksums file (URL source only; required with `--signed-checksums-url`) |

For a full explanation of the GPG verification modes, see [SECURITY.md](SECURITY.md).

---

### `grip remove <name>...`

Removes one or more entries from `grip.toml`, `grip.lock`, and `.bin/`.

```sh
grip remove ripgrep
grip remove jq ripgrep              # remove multiple tools at once
grip remove libssl-dev --library    # remove a library entry
grip remove ripgrep --no-sync       # remove from manifest/lock but keep .bin/ripgrep
```

| Flag | Description |
|---|---|
| `--library` | Remove from `[libraries]` instead of `[binaries]` |
| `--no-sync` | Remove from `grip.toml` and `grip.lock` but leave `.bin/` untouched |
| `--frozen` | Remove from `grip.toml` only; do not update `grip.lock` |

---

### `grip lock`

Manages `grip.lock`. Without flags, resolves every entry in `grip.toml` to its latest
matching version (respecting semver ranges and pins) and writes the result to `grip.lock`.
No binaries are installed or removed.

```sh
grip lock                                # update lock file from manifest
grip lock --check                        # assert lock file is up-to-date; exit 1 if not
grip lock --dry-run                      # show what would change; do not write
grip lock --upgrade                      # re-resolve all entries to latest available
grip lock --upgrade-package ripgrep      # re-resolve a single entry
grip lock --upgrade-package jq --upgrade-package rg   # re-resolve multiple entries
```

| Flag | Description |
|---|---|
| `--check` | Assert that `grip.lock` would not change; exit `1` if a re-lock would modify it |
| `--dry-run` | Print what would be written to `grip.lock` without modifying the file |
| `--upgrade` | Re-resolve all entries to the latest version available from their source |
| `--upgrade-package <name>` | Re-resolve only the named entry to the latest available version; repeatable |
| `--tag <tag>` | Only consider entries that carry this tag |


#### `grip lock verify`

Re-hashes every binary in `.bin/` and compares the result against the SHA-256 recorded in
`grip.lock`. Does not re-download anything or read `grip.toml` — purely a tamper-detection
command.

```sh
grip lock verify
```

Output:

```
  ✓  jq
  ✓  rg
  ⚠  fd  (no sha256 in lock — cannot verify)

  OK  (2 verified, 1 without sha256)
```

Exits `1` if any binary's hash does not match. For the recommended CI setup see
[SECURITY.md](SECURITY.md).

#### `grip lock pin`

Reads every binary and library in `grip.toml` that has no `version` field and writes the
exact version recorded in `grip.lock` back into `grip.toml`. Entries not yet in `grip.lock`
are skipped with a warning — run `grip sync` first, then re-run `grip lock pin`.

```sh
grip lock pin            # pin everything unpinned
grip lock pin --dry-run  # preview changes without modifying grip.toml
```

| Flag | Description |
|---|---|
| `--dry-run` | Print what would be pinned without writing `grip.toml` |


---

### `grip sync`

Downloads and installs any missing tools from `grip.toml` into `.bin/` concurrently.
Already-installed binaries are skipped. Download-based installs use a local cache so
archives are not re-downloaded on repeat runs.

The project is re-locked before syncing unless `--locked` or `--frozen` is provided.

```sh
grip sync
grip sync --locked                  # CI mode: fail if lock would change
grip sync --frozen                  # install exactly what is in grip.lock; never update it
grip sync --locked --require-pins   # also fail if any entry has no version pin
grip sync --tag dev                 # only entries tagged "dev"
grip sync --check                   # verify .bin/ matches grip.lock without installing
grip sync --dry-run                 # show what would be installed without doing it
grip sync --verify                  # re-verify SHA256 of already-installed binaries
```

| Flag | Description |
|---|---|
| `--locked` | Fail if `grip.lock` would change; enforces reproducibility in CI |
| `--frozen` | Do not update `grip.lock`; install exactly what is already recorded in it |
| `--check` | Verify `.bin/` against `grip.lock` without installing or modifying anything; exit `1` on any mismatch |
| `--dry-run` | Print what would be installed without writing anything to disk |
| `--verify` | Re-check SHA256 of on-disk binaries against `grip.lock` |
| `--tag <tag>` | Only install entries that carry this tag |
| `--require-pins` | Fail before touching the network if any entry has no version pin (prevents silent auto-upgrades in CI) |


---

### `grip run [--] <cmd> [args]`

Runs a command with `.bin/` prepended to `PATH`. All arguments after `--` are passed
directly to the command and never interpreted by grip.

```sh
grip run jq '.name' package.json
grip run rg --version
grip run -- fd --hidden .          # use -- to avoid ambiguity with grip flags
grip run --no-sync jq --version   # run without checking for missing tools
grip run --locked jq --version    # run; fail if lock would change
grip run --frozen jq --version    # run; never update grip.lock
```

| Flag | Description |
|---|---|
| `--no-sync` | Skip the pre-run sync check; run with whatever is already in `.bin/` |
| `--locked` | When syncing before run, fail if `grip.lock` would change |
| `--frozen` | When syncing before run, do not update `grip.lock` |

---

### `grip tree`

Prints installed entries from `grip.lock` with their versions, sources, and install
timestamps, in separate sections for binaries and libraries. Also shows entries declared
in `grip.toml` that have not yet been installed when `--all` is passed.

```sh
grip tree          # installed entries only (from grip.lock)
grip tree --all    # all declared entries; uninstalled ones are highlighted
```

| Flag | Description |
|---|---|
| `--all` | Also show entries declared in `grip.toml` that have not yet been installed |


---

### `grip cache`

Manages the local download cache (`~/.cache/grip/downloads/` by default). Override with
`--cache-dir` (global flag) or the `GRIP_CACHE_DIR` environment variable. Set
`GRIP_CACHE_DIR` to an empty string to disable caching entirely.

```sh
grip cache dir     # print the cache directory path
grip cache size    # show entry count and total disk usage
grip cache clean   # remove all cached downloads
grip cache prune   # remove stale or unreachable cache entries only
```

```sh
GRIP_CACHE_DIR=/tmp/my-cache grip sync   # custom cache location for one run
GRIP_CACHE_DIR= grip sync                # disable cache
```

| Subcommand | Description |
|---|---|
| `dir` | Print the resolved cache directory path |
| `size` | Show the number of cached archives and total disk usage |
| `clean` | Remove all cached downloads and print how much was freed |
| `prune` | Remove only stale or unreachable cache entries; keep entries referenced by `grip.lock` |


---

### `grip export`

Reads `grip.toml` and `grip.lock` and prints native install commands or a machine-readable
dependency artifact. Versions are taken from the lock file when available.

```sh
grip export                                    # shell script (default)
grip export --format dockerfile                # Dockerfile RUN lines
grip export --format makefile                  # Makefile target
grip export --format cyclonedx                 # CycloneDX 1.5 JSON SBOM to stdout
grip export --format spdx                      # SPDX 2.3 JSON SBOM to stdout
grip export --format cyclonedx -o sbom.json    # CycloneDX SBOM to file
grip export --format spdx -o sbom.spdx.json
```

| Flag | Description |
|---|---|
| `--format <fmt>` | `shell` (default), `dockerfile`, `makefile`, `cyclonedx`, `spdx` |
| `-o, --output <FILE>` | Write output to FILE instead of stdout |

**SBOM output** — no network access required:

- **CycloneDX 1.5** — each lock entry becomes a `component` with `type`, `name`,
  `version`, `purl`, `hashes` (SHA-256), and `externalReferences`.
- **SPDX 2.3** — each entry becomes a `package` with a `purl` external reference,
  `checksums` when available, and `NOASSERTION` download location for system packages.

**Example — `--format dockerfile`:**

```dockerfile
# Generated by grip export --format dockerfile
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    libssl-dev=3.0.2 \
    ripgrep=14.1.0 \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL -o /usr/local/bin/jq \
    "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64" \
    && chmod +x /usr/local/bin/jq
```


---

### `grip audit`

Queries the [OSV vulnerability database](https://osv.dev/) for known CVEs and advisories
affecting your installed tools. Sends a single batch request using the purl of each
`grip.lock` entry.

```sh
grip audit                 # exit 1 if any findings (default)
grip audit --no-fail       # report findings but always exit 0
```

| Flag | Description |
|---|---|
| `--no-fail` | Exit 0 even when vulnerabilities are found |

Exits `1` if any vulnerabilities are found (suitable for CI). Use `--no-fail` to report
without blocking. Run `grip lock --upgrade-package <name>` then `grip sync` to upgrade a
vulnerable tool.

---

### `grip suggest`

Scans the project and (optionally) source code for CLI tool references that are not yet
declared in `grip.toml`, then prints suggested `grip add` commands.

**Default scan sources** (no flags required):

- `Makefile` at the project root
- `scripts/` directory (shell scripts, Python, etc.)
- `.github/workflows/` CI YAML files
- `Dockerfile`, `Dockerfile.*`, and `*.dockerfile` — parses `RUN apt-get install` and `RUN dnf install` lines

**Optional source-code scan** — pass `--path` to also detect tools referenced via
subprocess/exec APIs in Rust, Python, JavaScript/TypeScript, Go, and Ruby source files,
as well as `/bin/<name>` path literals in any file type.

```sh
grip suggest                              # scan Makefile, scripts/, workflows/
grip suggest --history                    # also scan shell history
grip suggest --path src/                  # also scan source code under src/
grip suggest --path src/ --path scripts/  # multiple paths
grip suggest --check                      # exit 1 if any suggestions are found (CI)
```

| Flag | Description |
|---|---|
| `-p, --path <PATH>` | Source-code path to scan for binary invocations (repeatable) |
| `--history` | Also scan shell history files (`~/.bash_history`, `~/.zsh_history`, Fish history) |
| `--check` | Exit with status `1` if any unmanaged tools are found; useful in CI |

**Example output:**

```
  Suggested additions to grip.toml

  ✦  fd                grip add sharkdp/fd --source github
     ↳ found in: scripts/build.sh, .github/workflows/ci.yml

  ?  ffmpeg
     ↳ found in: src/encoder.py (2×)
```

Entries marked `✦` are in grip's curated tool list with a known GitHub source. Entries
marked `?` were detected but have no known source — add them manually.

Tools already declared in `grip.toml` are excluded. System builtins (`grep`, `sed`, `awk`,
`curl`, etc.) are always filtered out.

---

### `grip env`

Prints shell code that adds `.bin/` to `PATH`. Prefer `grip run` for one-off invocations;
use `grip env` when you want `.bin/` on `PATH` for an entire shell session.

```sh
eval "$(grip env)"               # bash / zsh
grip env --shell fish | source   # fish
```

| Flag | Description |
|---|---|
| `--shell <shell>` | `bash`, `zsh`, `fish`, `sh` (auto-detected from `$SHELL` if omitted) |

**Add `.bin/` to `PATH` permanently** — add to your shell profile:

```sh
# ~/.bashrc or ~/.zshrc
eval "$(grip env)"

# ~/.config/fish/config.fish
grip env --shell fish | source
```

---

## grip.toml reference

### GitHub Releases

Downloads a release asset for the current OS and architecture. Versions can be pinned
exactly or expressed as semver ranges.

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

**Semver ranges** (`^`, `~`, `>=`, `>`, `<`, `<=`, `*`) are resolved at lock time against
the GitHub releases list. The concrete version is written to `grip.lock`; `--locked` mode
pins to that exact version on subsequent syncs. If no `asset_pattern` is set, grip falls
back to a platform-aware heuristic.

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

When `extra_binaries` is set, grip symlinks each listed binary from its on-PATH location
into `.bin/` alongside the primary binary. The lock entry records all extra binary names so
`grip sync --check` can verify they are all present.

grip requires root or passwordless `sudo` to invoke `apt-get` / `dnf`. It checks privileges
once before any install and fails with a clear message rather than prompting for a password
mid-run.

#### Running in Docker

Running as a non-root user is incompatible with `apt` and `dnf` installs. The recommended
pattern is to run `grip sync` as root inside the container:

```dockerfile
# In your Dockerfile
RUN grip sync          # runs as root (default in most base images)
```

If your CI pipeline runs containers with `--user`, prefer `grip export --format dockerfile`
to generate a native `apt-get install` block instead.

---

### Custom apt/dnf sources and GPG keys

For packages outside the default repository:

```toml
[binaries.ttf-mscorefonts-installer]
source      = "apt"
package     = "ttf-mscorefonts-installer"
apt_sources = ["deb http://deb.debian.org/debian trixie contrib non-free"]

[binaries.ffmpeg]
source    = "dnf"
package   = "ffmpeg"
dnf_repos = ["https://download1.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm"]
```

#### debconf selections and GPG keys

```toml
[binaries.ttf-mscorefonts-installer]
source              = "apt"
package             = "ttf-mscorefonts-installer"
debconf_selections  = ["ttf-mscorefonts-installer msttcorefonts/accepted-mscorefonts-eula select true"]

[binaries.ffmpeg]
source    = "dnf"
package   = "ffmpeg"
gpg_keys  = ["https://rpmfusion.org/keys/RPM-GPG-KEY-rpmfusion-free-fedora-2020"]
```

`gpg_keys` on apt entries are downloaded, dearmored, and written to
`/usr/share/keyrings/grip-<name>.gpg`. For dnf entries, `rpm --import <url>` is used.

#### Flag passthrough

```toml
[binaries.supervisor]
source    = "apt"
package   = "supervisor"
apt_flags = ["--no-install-recommends"]
apt_env   = { DEBIAN_FRONTEND = "noninteractive" }

[binaries.ffmpeg]
source    = "dnf"
package   = "ffmpeg"
dnf_flags = ["--setopt=install_weak_deps=False"]
```

`DEBIAN_FRONTEND=noninteractive` is always set by default for apt; `apt_env` entries
supplement it.

---

### Libraries (no executable)

System packages that install headers or shared libraries but produce no binary belong in
`[libraries]`. Installed via the system package manager; no `.bin/` symlink is created.

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

After install, grip verifies the package is fully installed (`dpkg-query` / `rpm -q`) and
records a SHA-256 fingerprint of the installed `.so` files in `grip.lock`, enabling
`grip lock verify` to detect partial or corrupted library installs.

---

### Common optional fields

All entry types support these fields:

```toml
platforms    = ["linux", "darwin"]       # restrict to specific OSes (linux, darwin, windows)
tags         = ["dev", "ci"]             # selective installs: grip sync --tag dev
required     = false                     # warn instead of failing on error (default: true)
```

---

## Reproducibility and CI

`grip.lock` records the exact version, download URL, and SHA-256 checksum of every
installed binary. Commit it alongside `grip.toml`.

### Recommended CI setup

```sh
grip suggest --check                        # fail if any tool is used but not declared
grip lock --check                           # fail if grip.lock is not up-to-date
grip sync --locked --require-pins           # install; fail if lock would change or any version floats
grip lock verify                            # re-hash every .bin/ binary; catch tampering
```

Use `grip lock --upgrade` to see what has newer versions available (add `--dry-run` to
preview without writing), then `grip sync` to install. Use `grip export --format dockerfile`
to generate a Dockerfile snippet from the lock file.
