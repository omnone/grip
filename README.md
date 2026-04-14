# grip

[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust stable](https://img.shields.io/badge/rust-stable-orange.svg)](https://rustup.rs/)
[![Build](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/omnone/grip/actions)

**A per-project tool dependency manager** — like `package.json`, but for the binaries your project actually depends on.

Declare your tools in `grip.toml`. Run `grip sync`.  
That's it.

Every developer, CI job, and Docker build gets **byte-for-byte identical binaries**:

- no version drift  
- no manual setup docs  
- no global installs  
- no surprises  

---

## Why grip exists

If your project depends on CLI tools, you already have a dependency problem — just without a proper solution.

It usually looks like this:

- A Makefile full of `curl | sh` scripts that *kind of* install things  
- A README that says "install `jq >= 1.6`" (so everyone runs something different)  
- CI pipelines pulling the *latest* version — until a breaking release ruins your day or triggers a security incident  
- Dockerfiles with pinned versions that slowly rot out of sync  

This is dependency management — just inconsistent, fragile, and invisible.

---

## The solution

**grip brings real dependency management to your tooling stack.**

Like npm, Cargo, and pip did for code, grip gives you:

- A **manifest** (`grip.toml`) to declare tools  
- A **lockfile** to pin exact versions  
- A single command (`grip sync`) to install everything reproducibly  

No guesswork. No drift. No "works on my machine."

---

```
$ grip init
  ✓  created grip.toml
  ✓  added .bin/ to .gitignore

$ grip add BurntSushi/ripgrep
  [1/1] ✓  rg  14.1.1

$ grip add jq --source github --repo jqlang/jq
  [1/1] ✓  jq  1.7.1

$ grip list

  Installed binaries (from grip.lock)

  NAME               VERSION        SOURCE     INSTALLED AT
  ──────────────────────────────────────────────────────────────────
  rg                 14.1.1         github     2024-03-21 16:18
  jq                 1.7.1          github     2024-03-21 16:19

# On a teammate's machine after git pull:
$ grip sync --locked
  ✓  all 2 binaries up to date

$ grip check
  ✓  rg   14.1.1   github  sha256 verified
  ✓  jq   1.7.1    github  sha256 verified
```

---

## Features

- **Byte-for-byte reproducibility** — `grip.lock` records the exact version, download URL, and SHA-256 of every installed binary. `grip sync --locked` fails CI if the lock would change.
- **No global pollution** — tools land in `.bin/` at the project root; nothing touches `/usr/local/bin` or any system directory.
- **Fast, cached installs** — a local download cache avoids re-fetching archives on every run. Concurrent installs for download-based sources.
- **Mixed sources in one file** — GitHub Releases, direct URLs, APT, and DNF all declared in a single `grip.toml`.
- **Docker-native export** — `grip export --format dockerfile` generates lock-file-accurate `RUN` instructions, so your images don't need grip installed at build time.
- **Library support** — declare `apt`/`dnf` packages that produce no binary (headers, shared libs) alongside your tools in the same manifest.
- **Supply chain attack protection** — GPG signature verification for GitHub and URL sources, `grip lock verify` for post-install tamper detection, and `--require-pins` to block silent auto-upgrades in CI. See [SECURITY.md](SECURITY.md).
- **Tool discovery** — `grip suggest` scans your Makefile, CI YAML, shell history, and source code (Python, Rust, JS, Go, Ruby) to find CLI tools you already use but haven't declared in `grip.toml`. Use `--check` in CI to enforce that nothing is left unmanaged.
- **SBOM generation** — `grip sbom` exports a CycloneDX 1.5 or SPDX 2.3 Software Bill of Materials from `grip.lock`. Required for US federal procurement (EO 14028) and enterprise compliance. No network access needed.
- **Vulnerability scanning** — `grip audit` cross-references every installed tool against the [OSV database](https://osv.dev/) and exits non-zero on findings, making it drop-in ready for CI pipelines.

---

## How it compares

| Feature | grip | Makefile + curl | brew | asdf / mise |
|---------|:----:|:---------------:|:----:|:-----------:|
| Per-project isolation | ✓ | ✓ | ✗ global | ✓ |
| Lockfile with SHA-256 | ✓ | ✗ | partial | ✗ |
| GitHub, URL, APT, DNF | ✓ | manual | ✗ | via plugins |
| Docker export (no grip in image) | ✓ | ✗ | ✗ | ✗ |
| System library packages | ✓ | ✗ | ✗ | ✗ |
| Semver ranges | ✓ | ✗ | ✓ | ✗ |
| Works on Linux + macOS | ✓ | ✓ | macOS-first | ✓ |
| CI mode — fail on lock drift | ✓ `--locked` | ✗ | ✗ | ✗ |
| CI mode — fail on unpinned versions | ✓ `--require-pins` | ✗ | ✗ | ✗ |
| GPG signature verification | ✓ | ✗ | ✗ | ✗ |
| Post-install tamper detection | ✓ `grip lock verify` | ✗ | ✗ | ✗ |
| SBOM export (CycloneDX / SPDX) | ✓ `grip sbom` | ✗ | ✗ | ✗ |
| CVE / advisory scanning | ✓ `grip audit` | ✗ | ✗ | ✗ |
| Zero setup for consumers | ✓ `grip sync` | ✓ | requires brew | requires asdf |

---

## Quick start

```sh
grip init                          # create grip.toml, add .bin/ to .gitignore
grip add BurntSushi/ripgrep        # add from GitHub Releases (installs immediately)
grip add jq --source apt           # or from the system package manager
eval "$(grip env)"                 # add .bin/ to PATH for this shell session
grip run rg --version              # or run a tool without touching PATH
```

`grip add` installs the binary immediately and writes both `grip.toml` (the manifest) and `grip.lock` (the lockfile with the exact version, download URL, and SHA-256). You never edit `grip.lock` by hand — grip maintains it automatically. `grip sync` also updates the lockfile for any entries not yet recorded there.

Commit both files so teammates and CI get identical binaries:

```sh
git add grip.toml grip.lock
git commit -m "add dev tools"
```

On any other machine, after cloning:

```sh
grip sync --locked     # installs exactly what's in grip.lock; fails if it would change
```

### Example `grip.toml`

```toml
[binaries.jq]
source = "github"
repo   = "jqlang/jq"
version = "^1.7"          # semver range; resolved version is pinned in grip.lock

[binaries.ripgrep]
source  = "apt"           # or "dnf" on RPM-based systems
package = "ripgrep"
binary  = "rg"            # on-PATH command differs from the package name

[binaries.ffmpeg]
source          = "apt"
package         = "ffmpeg"
extra_binaries  = ["ffprobe", "ffplay"]  # additional binaries installed by the same package

[libraries.libssl-dev]
source  = "apt"
package = "libssl-dev"
version = "3.0.2"         # system library — no .bin/ symlink created
```

### Docker / CI

Use `grip export` to generate native install instructions from the lock file — no grip required in the image:

```sh
grip export --format dockerfile
```

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

In CI without Docker, enforce the lock file directly:

```sh
grip sync --locked    # fails the build if grip.lock would change
```

---

## Installation

Pre-built binaries for Linux and macOS are planned for a future release. For now, build from source using the Rust stable toolchain.

**Prerequisites:** install [rustup](https://rustup.rs/) if you don't have Rust yet:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Clone the repository:

```sh
git clone https://github.com/omnone/grip.git
cd grip
```

**Option A — install system-wide** (copies `grip` into `~/.cargo/bin`, which is already on your `PATH`):

```sh
cargo install --path .
```

**Option B — build a release binary** and place it wherever you like:

```sh
cargo build --release
# binary is at target/release/grip
sudo cp target/release/grip /usr/local/bin/grip   # or any directory on your PATH
```

Verify the install:

```sh
grip --version
```

---

## Security

grip includes layered supply chain attack protections:

- **GPG signature verification** — add `gpg_fingerprint` to any `github` or `url` entry; grip verifies the release asset signature before installing (direct `.sig`/`.asc` or signed `SHA256SUMS` file).
- **Post-install tamper detection** — `grip lock verify` re-hashes every `.bin/` binary against `grip.lock` without re-downloading.
- **Version pin enforcement** — `grip sync --require-pins` fails before touching the network if any entry floats to "latest".
- **`grip doctor`** detects SHA256 drift, missing hashes in the lock, and unpinned entries.

Recommended CI setup:

```sh
grip sync --locked --require-pins
grip lock verify
```

See [SECURITY.md](SECURITY.md) for the full security guide including GPG setup, threat model, and CI configuration examples.

---

## CLI reference

For the full command reference — all flags, `grip.toml` source types, shell integration, and CI guidance — see [COMMANDS.md](COMMANDS.md).

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, run the test suites, and add new adapters.

---

## License

[MIT](LICENSE)
