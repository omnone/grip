# grip

[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust stable](https://img.shields.io/badge/rust-stable-orange.svg)](https://rustup.rs/)
[![Build](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/omnone/grip/actions)

**Your code has a lockfile. Your tools don't.**

grip gives CLI tools the same treatment that npm gives packages and Cargo
gives crates — declare them, lock them, sync them.

Declare your tools in `grip.toml`. Run `grip sync`. Every developer, CI job,
and Docker build gets the exact same binaries — same version, same SHA-256,
no setup docs, no surprises.

---

## The problem

Every project depends on CLI tools but there is no standard way to manage them:

- README says "install `jq >= 1.6`" — everyone installs something slightly different
- CI pulls the *latest* release — until a breaking change breaks the build
- Dockerfiles have versions copy-pasted from a wiki that nobody updates
- A new teammate spends an afternoon installing things manually before they can run the project

This is a dependency problem, and it deserves a real solution.

---

## The solution

Three pieces, one workflow:

- A **manifest** (`grip.toml`) — declare which tools the project needs and where to get them
- A **lockfile** (`grip.lock`) — records the exact version, download URL, and SHA-256 of every tool
- One command (`grip sync`) — installs everything reproducibly from the lockfile

Commit both files. Every developer runs `grip sync` after pulling. That is the entire workflow.

---

## Why grip?

Tools like asdf, mise, and Homebrew are version managers for your machine — they answer "what version do I have installed?" grip answers a different question: "what does this project require?"

| | asdf / mise | Homebrew | **grip** |
|---|---|---|---|
| Scope | machine-wide | machine-wide | **per-project** |
| Lockfile checked into repo | no | no | **yes** |
| Works without root | yes | no | **yes** |
| Mixed sources in one file | no | no | **yes** |
| SHA-256 verified install | no | no | **yes** |

The lockfile is the difference. Commit `grip.lock` alongside `grip.toml` and there's nothing to configure per-machine — every install gets identical bytes, not "the latest 1.x" or "whatever was on the wiki."

---

## Quickstart

```sh
# 1. Set up grip in your project (run once, from the project root)
$ grip init
Created grip.toml
Created .gitignore with .bin/

# 2. Discover tools your project already uses but hasn't declared
$ grip suggest
  jq          referenced in .github/workflows/release.yml  →  grip add jq
  shellcheck  referenced in Makefile                       →  grip add shellcheck

# 3. Add a tool — grip downloads and installs it immediately
$ grip add jq@1.7.1 --repo jqlang/jq --source github
Added 'jq' to grip.toml

  1 installed  (1.2s)

# 4. See what is installed
$ grip list

  Installed binaries (from grip.lock)

  NAME    VERSION   SOURCE   INSTALLED AT
  ──────────────────────────────────────────────
  jq      1.7.1     github   2025-03-14 09:41

# 5. Commit the manifest and lockfile so everyone gets the same thing
$ git add grip.toml grip.lock
$ git commit -m "add dev tools"
```

Generate Dockerfile instructions from the lockfile — no grip required in the image:

```sh
$ grip export --format dockerfile
```

```dockerfile
RUN curl -fsSL -o /usr/local/bin/jq \
    "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64" \
    && chmod +x /usr/local/bin/jq
```

Paste the output into your `Dockerfile`. The URL and version come straight from `grip.lock`, so they stay in sync as you update tools.

**On a teammate's machine** — after `git pull`, a single command installs everything:

```sh
$ grip sync --locked
  All up to date  (1 skipped, 0.0s)
```

`--locked` means the build fails if `grip.lock` would change — no silent version drift in CI.

---

## Integrating grip into a Dockerfile project

If your project already has a `Dockerfile` with `RUN apt-get install` or `RUN dnf install` lines,
`grip init` can import those packages automatically:

```sh
# 1. Run grip init in the project root — auto-detects Dockerfile
$ grip init

Created grip.toml
Added .bin/ to .gitignore

Found Dockerfile — 14 apt packages.
Verifying against curated tools and host package manager…

  Verified — will import (11):
    binary   jq            1.6-2.1          via apt  (curated)
    binary   ripgrep                        via apt  (curated)  → cmd `rg`
    library  libssl-dev    1.1.1n-0+deb11u5 via apt
    library  pkg-config                     via apt  (curated)
    library  build-essential                via apt  (curated)
    ...

  Skipped (not verified) — review manually (3):
    some-internal-tool   not found in curated list or host package manager

  Import the 11 verified entries into grip.toml? [Y/n]
```

```sh
# 2. Generate grip.lock from inside the same Docker base image
$ docker run --rm -v "$PWD":/work -w /work debian:bookworm \
    sh -c 'apt-get update && apt-get install -y curl ca-certificates && \
           curl -fsSL https://github.com/omnone/grip/releases/latest/download/grip-x86_64-linux \
                -o /usr/local/bin/grip && chmod +x /usr/local/bin/grip && \
           grip sync --locked'

# 3. Replace the apt-get install block in your Dockerfile
$ grip export --format dockerfile

# 4. Commit
$ git add grip.toml grip.lock Dockerfile
```

After this, every developer and CI run gets the same packages via `grip sync`.

See [EXAMPLES.md](docs/EXAMPLES.md) for the full walkthrough including multi-Dockerfile projects,
`--offline` mode, and the CI setup.

---

## Day-to-day workflow

### Run a tool without changing your shell PATH

```sh
$ grip run jq --version
jq-1.7.1
```

### Add your project's `.bin/` to PATH for the current shell session

```sh
$ eval "$(grip env)"
$ jq --version
jq-1.7.1
```

### Check that what is installed matches the lockfile

`grip check` verifies installed binaries against the lockfile and also reports consistency issues — orphaned lock entries, unpinned versions, and missing SHA-256 hashes.

```sh
$ grip check

  Checking installed binaries…

  ✓  jq

  All 1 checks passed
```

If any consistency issues are found, they appear in a separate section and `grip check` exits non-zero:

```sh
$ grip check

  Checking installed binaries…

  ✓  jq

  Consistency issues

  ⚠  binary 'kubectl' (github) has no version pin — run `grip pin` to fix

  1 check passed, 1 consistency issue
```

### Pin unpinned tools to their installed versions

```sh
$ grip pin              # write exact versions from grip.lock into grip.toml
$ grip pin --dry-run    # preview what would be pinned without modifying grip.toml
```

Entries that are not yet installed (not in `grip.lock`) are skipped with a warning — run `grip sync` first, then re-run `grip pin`.

### See if newer versions are available

```sh
$ grip outdated

  BINARY    INSTALLED   LATEST    STATUS
  ───────────────────────────────────────
  jq        1.7.1       1.7.1     up to date
  kubectl   1.30.2      1.31.0    outdated
  terraform 1.6.6       1.8.1     outdated
```

Run `grip add kubectl@1.31.0` to update, then `git commit grip.toml grip.lock`.

### Remove a tool

```sh
$ grip remove jq
  ✓  removed .bin/jq
  ✓  removed 'jq' from [binaries] in grip.toml
```

---

## Example `grip.toml`

```toml
# Pin an exact version from GitHub Releases
[binaries.jq]
source  = "github"
repo    = "jqlang/jq"
version = "1.7.1"

# Use a semver range — grip resolves to the highest matching release
# and pins the resolved version in grip.lock
[binaries.kubectl]
source  = "github"
repo    = "kubernetes/kubectl"
version = "^1.30"

# Install from the system package manager (apt or dnf)
[binaries.ripgrep]
source  = "apt"
package = "ripgrep"
binary  = "rg"        # the on-PATH command differs from the package name

# Install from a direct URL (SHA-256 is recorded in grip.lock)
[binaries.protoc]
source  = "url"
url     = "https://github.com/protocolbuffers/protobuf/releases/download/v26.1/protoc-26.1-linux-x86_64.zip"
binary  = "bin/protoc"

# Declare a system library (no binary symlink, just ensures the package is present)
[libraries.libssl-dev]
source  = "apt"
package = "libssl-dev"
```

You never edit `grip.lock` by hand — grip maintains it automatically.

---

## CI

### Recommended setup

```sh
grip suggest --check                # fail if any tool is used but not declared
grip sync --locked --require-pins   # fails if lock would change, or any version floats
grip lock verify                    # re-hashes every .bin/ binary; catches tampering
```

`grip suggest --check` fails the job if any tool referenced in scripts or CI YAML isn't declared in `grip.toml`. `grip sync --locked` ensures the lockfile is respected exactly — the job fails if `grip.lock` would need to change. `grip lock verify` re-hashes every binary in `.bin/` against the recorded SHA-256 without re-downloading anything, catching tampering introduced between sync and execution.

For generating lock-accurate Dockerfile instructions, see [Quickstart](#quickstart).

---

## Features

- **Byte-for-byte reproducibility** — `grip.lock` records the exact version, download URL, and SHA-256 of every installed binary.
- **No global pollution** — tools land in `.bin/` at the project root; nothing touches system directories.
- **Mixed sources** — four source types are currently supported: `github`, `url`, `apt`, and `dnf`, all declared in a single `grip.toml`.
- **Fast installs** — a local download cache avoids re-fetching; multiple tools install concurrently.
- **Semver ranges** — `version = "^1.30"` resolves to the highest matching release and locks the result.
- **Supply chain protection** — optional GPG verification, tamper detection, and floating-version enforcement. See [SECURITY.md](docs/SECURITY.md).

### SBOM generation

`grip sbom` exports a Software Bill of Materials directly from `grip.lock` — no network access needed:

```sh
$ grip sbom --format cyclonedx > sbom.json
$ grip sbom --format spdx > sbom.spdx
```

CycloneDX 1.5 and SPDX 2.3 are both supported.

### Vulnerability scanning

`grip audit` cross-references every installed tool against the [OSV database](https://osv.dev/) and exits non-zero on findings:

```sh
$ grip audit

  Checking 3 binaries against OSV…

  jq        1.7.1   ✓  no known vulnerabilities
  kubectl   1.30.2  ✓  no known vulnerabilities
  terraform 1.6.6   ✗  1 vulnerability  (run grip audit --json for details)

  1 vulnerability found — exit code 1
```

Add `grip audit` to CI to block deploys when new CVEs are published against your declared tools.

---

## Installation

grip runs on Linux and macOS. Windows is not currently supported.

```sh
curl -fsSL https://github.com/omnone/grip/releases/latest/download/grip-$(uname -m)-linux -o ~/.local/bin/grip && chmod +x ~/.local/bin/grip
```

Or build from source with the Rust stable toolchain. Install Rust if you don't have it:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Clone and install:

```sh
git clone https://github.com/omnone/grip.git
cd grip
cargo install --path .    # installs grip into ~/.cargo/bin (already on your PATH)
```

Or build a release binary and place it wherever you like:

```sh
cargo build --release
sudo cp target/release/grip /usr/local/bin/grip
```

Verify:

```sh
grip --version
```

---

## Security

grip supports layered supply chain protections:

- **GPG signature verification** — add `gpg_fingerprint` to any `github` or `url` entry to verify the release asset signature before installing.
- **Post-install tamper detection** — `grip lock verify` re-hashes every `.bin/` binary against `grip.lock` without re-downloading anything.
- **Version pin enforcement** — `grip sync --require-pins` fails before touching the network if any entry floats to "latest".
- **Health checks** — `grip check` detects orphaned lock entries, missing SHA-256 hashes, and unpinned entries in addition to verifying installed binaries.

See [SECURITY.md](docs/SECURITY.md) for the full guide.

---

## CLI reference

For the full command reference — all flags, source types, shell integration, and CI guidance — see [COMMANDS.md](docs/COMMANDS.md).

---

## Recipes

For copy-pasteable examples covering Dockerfile import, GitHub releases, apt/dnf, pinning, and CI setup see [EXAMPLES.md](docs/EXAMPLES.md).

---

## Contributing

See [CONTRIBUTING.md](docs/CONTRIBUTING.md) for how to build, run the test suites, and add new adapters.

---

## License

[MIT](LICENSE)
