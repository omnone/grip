# grip — Binary Manager

**grip** is a per-project CLI tool dependency manager. Declare your tools in `grip.toml` — similar to `pyproject.toml` for Python — and `grip install` takes care of the rest.

Tools are installed into a local `.bin/` directory at the project root, isolated from your system. A `grip.lock` file records exact versions and checksums so every developer and CI run gets identical binaries.

**Supported sources:** GitHub Releases, direct URLs, APT, DNF, custom shell commands.

---

## Quick start

```sh
grip init                        # create grip.toml and add .bin/ to .gitignore
grip add BurntSushi/ripgrep      # add a tool from GitHub
grip install                     # install everything into .bin/
eval "$(grip env)"               # add .bin/ to PATH for this shell session
grip run rg --version            # or run a tool directly without touching PATH
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

Adds a binary entry to `grip.toml`.

```sh
grip add BurntSushi/ripgrep               # GitHub shorthand: name=ripgrep, repo=BurntSushi/ripgrep
grip add jq@1.7.1 --repo jqlang/jq        # pin a version
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

On Linux the default source is `apt` or `dnf` when available. Use `--source github` or the `owner/repo` shorthand to force GitHub.

---

### `grip install` / `grip sync`

Installs all declared binaries into `.bin/` concurrently. Already-installed binaries are skipped.

```sh
grip install
grip install --locked            # CI mode: fail if lock would change
grip install --tag dev           # only entries tagged "dev"
grip install --verify            # re-verify SHA256 of already-installed binaries
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

### `grip list`

Prints all entries from `grip.lock` with their versions, sources, and install timestamps. No flags.

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

Downloads a release asset for the current OS and architecture automatically.

```toml
[binaries.jq]
source = "github"
repo = "jqlang/jq"
version = "1.7.1"                 # omit for latest
asset_pattern = "jq-linux-amd64" # optional glob to pick the right asset
```

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
binary = "rg"         # optional: on-PATH command when it differs from the table name (Fedora/Debian ship `rg`, not `ripgrep`)
```

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
tags         = ["dev", "ci"]             # selective installs: grip install --tag dev
required     = false                     # warn instead of failing on error (default: true)
post_install = "chmod +x .bin/mytool"   # shell command to run after a successful install
```

---

## Reproducibility and CI

`grip.lock` records the exact version, download URL, and SHA-256 checksum of every installed binary. Commit it alongside `grip.toml`.

In CI, use `--locked` to enforce the lock file and fail if it would change:

```sh
grip install --locked
```

Use `grip outdated` to see what has newer versions available, then `grip update <name>` to upgrade and refresh the lock entry.

---

## Build from source

Requires [Rust](https://rustup.rs/) (stable toolchain).

```sh
git clone https://github.com/omnone/grip.git
cd grip
cargo install --path .
```
