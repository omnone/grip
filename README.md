# grip — Binary Manager

**grip** is a per-project CLI tool dependency manager. Declare the tools your project needs in `grip.toml` — similar to how `pyproject.toml` manages Python dependencies — and `grip sync` handles the rest.

Tools land in a local `.bin/` directory at the project root, completely isolated from your system install. A `grip.lock` file records exact versions, download URLs, and SHA-256 checksums, so every developer, CI job, and Docker build gets byte-for-byte identical binaries without manual version coordination.

**Why grip?**

- **Reproducibility** — lock file guarantees identical tool versions across machines and over time
- **No global pollution** — tools are scoped to the project; nothing touches `/usr/local/bin`
- **Fast CI** — a built-in download cache avoids re-downloading the same archives on every run
- **Mixed sources** — GitHub Releases, direct URLs, APT, DNF, and shell scripts all live in one manifest
- **Docker-native** — `grip export` generates ready-to-paste Dockerfile `RUN` instructions from the lock file, so you never have to install grip in your images
- **Library support** — declare `apt`/`dnf` packages that produce no binary (headers, shared libs) alongside your tools in the same file

**Supported sources:** GitHub Releases, direct URLs, APT, DNF, custom shell commands.

---

## Docker workflow

The typical pattern is: use grip locally and in CI, and generate a lock-file-accurate Dockerfile snippet for production images.

**1. Declare your tools:**

```toml
# grip.toml
[binaries.jq]
source = "github"
repo = "jqlang/jq"
version = "^1.7"          # semver range — resolved to a concrete version in grip.lock

[binaries.ripgrep]
source = "apt"
package = "ripgrep"

[libraries.libssl-dev]
source = "apt"
package = "libssl-dev"
version = "3.0.2"
```

**2. Commit the lock file:**

`grip add` already installed each tool. Just commit what was written:

```sh
git add grip.toml grip.lock
git commit -m "add dev tools"
```

**3. Install from the lock file in Docker or CI:**

Add grip to your image and install with `--locked` to pin to exactly what is in `grip.lock`:

```dockerfile
RUN grip sync --locked
```

grip will not re-download archives that are already cached, and `--locked` fails the build if the lock file would change — catching any version drift.

> **Alternative — no grip in the image:** run `grip export --format dockerfile` locally to generate native `RUN` instructions from your lock file and paste them directly into your `Dockerfile`. grip is not required at image build time in this case.
>
> ```dockerfile
> # example output for the grip.toml above
> RUN apt-get update -y && apt-get install -y --no-install-recommends \
>     ripgrep=14.1.0 \
>     libssl-dev=3.0.2 \
>     && rm -rf /var/lib/apt/lists/*
> RUN curl -fsSL -o /usr/local/bin/jq \
>     "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64" \
>     && chmod +x /usr/local/bin/jq
> ```


## Quick start

```sh
grip init                        # create grip.toml and add .bin/ to .gitignore
grip add BurntSushi/ripgrep      # add a tool from GitHub (installs it immediately)
eval "$(grip env)"               # add .bin/ to PATH for this shell session
grip run rg --version            # or run a tool directly without touching PATH
grip sync                        # sync missing binaries (e.g. after cloning the repo)
```

---

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

Adds a binary entry to `grip.toml`. On Linux the default source is `apt` or `dnf` when available. Use `--source github` or the `owner/repo` shorthand to force GitHub.

```sh
grip add ripgrep                                   # apt/dnf on Linux (default)
grip add BurntSushi/ripgrep --source github        # GitHub Releases
grip add jq@1.7.1 --repo jqlang/jq --source github # pin a version
grip add libssl-dev --library                       # system library (no executable)
grip add mytool --source url --url https://example.com/mytool.tar.gz
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

---

### `grip sync`

Downloads and installs any missing binaries from `grip.toml` into `.bin/` concurrently. Already-installed binaries are skipped. Download-based installs use a local cache so archives are not re-downloaded on repeat runs.

```sh
grip sync
grip sync --locked            # CI mode: fail if lock would change
grip sync --tag dev           # only entries tagged "dev"
grip sync --verify            # re-verify SHA256 of already-installed binaries
```

| Flag | Description |
|---|---|
| `--locked` | Fail if `grip.lock` would change; enforces reproducibility in CI |
| `--verify` | Re-check SHA256 of on-disk binaries against `grip.lock` |
| `--tag <tag>` | Only install entries that carry this tag |

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

---

### `grip outdated`

Fetches the latest available version for every installed binary and shows a comparison table.

```sh
grip outdated
grip outdated --tag dev
```

| Flag | Description |
|---|---|
| `--tag <tag>` | Only check entries that carry this tag |

---

### `grip update <name>`

Re-installs a single binary from the manifest, fetching the latest version, and refreshes its lock entry.

```sh
grip update ripgrep
```

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

Prints all entries from `grip.lock` with their versions, sources, and install timestamps, in separate sections for binaries and libraries. No flags.

---

### `grip doctor`

Checks consistency between `grip.toml`, `grip.lock`, and `.bin/`: detects orphaned lock entries, missing binaries, SHA256 drift, and libraries not present on the system. No flags.

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

### `grip export`

Reads `grip.toml` and `grip.lock` and prints native install commands for use in Dockerfiles, CI scripts, or Makefiles. Versions are taken from the lock file when available.

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

Shell entries are emitted as comments — they cannot be auto-exported.

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

**Permanent setup** — add to your shell profile:

```sh
# ~/.bashrc or ~/.zshrc
eval "$(grip env)"

# ~/.config/fish/config.fish
grip env --shell fish | source
```

---

## grip.toml reference

### GitHub Releases

Downloads a release asset for the current OS and architecture automatically. Versions can be pinned exactly or expressed as semver ranges.

```toml
[binaries.jq]
source = "github"
repo = "jqlang/jq"
version = "1.7.1"                 # exact pin
# version = "^1.7"               # semver range: resolves to latest 1.x
asset_pattern = "jq-linux-amd64" # optional glob to pick the right asset
```

**Semver ranges** (`^`, `~`, `>=`, `>`, `<`, `<=`, `*`) are resolved at install time against the GitHub releases list. The concrete version is written to `grip.lock`; `--locked` mode pins to that exact version on subsequent installs.

### Direct URL

```toml
[binaries.mytool]
source = "url"
url = "https://example.com/releases/mytool-linux-amd64.tar.gz"
sha256 = "abc123..."  # optional, verified after download
```

### APT / DNF

```toml
[binaries.ripgrep]
source = "apt"        # or "dnf"
package = "ripgrep"   # defaults to binary name
binary = "rg"         # optional: on-PATH command when it differs from the table name
```

grip requires root or passwordless `sudo` to invoke `apt-get` / `dnf`. It checks privileges once before any install and fails with a clear message rather than prompting for a password mid-run.

### Libraries (no executable)

System packages that install headers or shared libraries but produce no command-line binary belong in `[libraries]`. They are installed via `apt-get` or `dnf` but grip does not create a `.bin/` symlink for them.

```toml
[libraries.libssl-dev]
source = "apt"
package = "libssl-dev"
version = "3.0.2"    # optional

[libraries.openssl-devel]
source = "dnf"
package = "openssl-devel"
```

Add with: `grip add libssl-dev --library`

### Shell

Runs an arbitrary shell command. `$GRIP_BIN_DIR` points to the project's `.bin/` directory.

```toml
[binaries.mytool]
source = "shell"
install_cmd = "curl -fsSL https://example.com/install.sh | bash -s -- --dir $GRIP_BIN_DIR"
version = "1.0"
```

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

Use `grip outdated` to see what has newer versions available, then `grip update <name>` to upgrade and refresh the lock entry.

Use `grip export --format dockerfile` to generate a Dockerfile snippet from the lock file without requiring grip to be installed in the image.

---

## Build from source

Requires [Rust](https://rustup.rs/) (stable toolchain).

```sh
git clone https://github.com/omnone/grip.git
cd grip
cargo install --path .
```
