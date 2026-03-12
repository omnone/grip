# grip — Binary Manager

**grip** is a per-project CLI tool dependency manager. Instead of relying on each developer to manually install the right versions of tools, you declare them in a `binaries.toml` file — similar to how `pyproject.toml` manages python dependencies — and `grip install` takes care of the rest.

Tools are installed into a local `.bin/` directory at the project root, keeping them isolated from your system. A `binaries.lock` file records exact versions and checksums so every developer and CI run gets identical binaries.

**Supported install sources:** GitHub Releases, direct URLs, APT, DNF, and custom shell commands.

---

## Usage

### 1. Initialize a project

```sh
grip init
```

Creates `binaries.toml` from a template and adds `.bin/` to `.gitignore`.

---

### 2. Declare your tools

Edit `binaries.toml` directly, or use `grip add`:

```sh
grip add jq --source github --repo jqlang/jq --version 1.7.1
grip add ripgrep --source github --repo BurntSushi/ripgrep
```

---

### 3. Install

```sh
grip install
```

Installs all declared binaries into `.bin/`, concurrently. Already-installed binaries are skipped.

```sh
grip install --locked   # CI mode: enforce lock file and verify checksums
grip install --tag dev  # only install entries with the "dev" tag
```

---

### 4. Run a tool

```sh
grip run jq '.name' package.json
```

Runs the command with `.bin/` prepended to `PATH` — no shell configuration needed.

---

### All commands

| Command | Description |
|---|---|
| `grip init` | Initialize `binaries.toml` and `.bin/` in the current directory |
| `grip add <name>` | Add an entry to `binaries.toml` |
| `grip install` | Install all binaries from `binaries.toml` |
| `grip run <cmd> [args]` | Run a command using binaries from `.bin/` |
| `grip list` | List all installed binaries and their versions |
| `grip update <name>` | Re-install a binary, fetching the latest version |

---

## binaries.toml reference

### GitHub Releases

Downloads a release asset for the current OS and architecture automatically.

```toml
[binaries.jq]
source = "github"
repo = "jqlang/jq"
version = "1.7.1"                 # omit for latest
asset_pattern = "jq-linux-amd64" # optional glob to disambiguate assets
binary = "jq-linux-amd64"        # optional name of binary inside archive
```

### Direct URL

```toml
[binaries.mytool]
source = "url"
url = "https://example.com/releases/mytool-linux-amd64.tar.gz"
binary = "mytool"     # name of binary inside archive (optional)
sha256 = "abc123..."  # optional, verified after download
```

### APT

```toml
[binaries.ripgrep]
source = "apt"
package = "ripgrep"  # defaults to binary name
version = "14.1.0"   # optional
```

### DNF

```toml
[binaries.tree]
source = "dnf"
package = "tree"
```

### Shell

Runs an arbitrary shell command. `$BM_BIN_DIR` points to the `.bin/` directory.

```toml
[binaries.mytool]
source = "shell"
install_cmd = "curl -fsSL https://example.com/install.sh | bash -s -- --dir $BM_BIN_DIR"
version = "1.0"
```

### Optional fields (all sources)

| Field | Type | Description |
|---|---|---|
| `tags` | `["tag1", "tag2"]` | Labels for selective installs (`--tag`) |
| `platforms` | `["linux", "darwin"]` | Only install on these platforms; omit for all |
| `required` | `true` / `false` | If `false`, failure is a warning not an error (default: `true`) |
| `post_install` | `"shell command"` | Run after a successful install |

Platform values: `linux`, `darwin`, `windows`.

---

## Lock file

`grip install` writes `binaries.lock` recording the exact version, URL, and SHA256 of every installed binary. Commit this file to ensure reproducible installs across machines and CI.

In CI, run `grip install --locked` to enforce the lock file and verify binary integrity.

---

## Build from source

**Prerequisites:** [Rust](https://rustup.rs/) (stable toolchain)

```sh
# Clone the repo
git clone https://github.com/omnone/grip.git
cd grip

# Build and install the binary to ~/.cargo/bin/
cargo install --path .
```

Or just build without installing:

```sh
cargo build --release
# Binary is at ./target/release/grip
```
