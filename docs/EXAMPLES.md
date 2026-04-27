# grip — recipes and examples

Copy-pasteable recipes for common grip workflows.
For flag documentation see [COMMANDS.md](COMMANDS.md).

---

## 1. Bootstrap from a Dockerfile

For projects that already declare dependencies in a `Dockerfile`, `grip init` can import
them in one step. It classifies packages as binary tools or libraries, verifies each package
against a curated list and the host package manager, and writes only the verified set into
`grip.toml`.

### Basic import

```sh
# Run from the project root — Dockerfile is auto-detected
grip init

# Explicit path (repeatable for multi-stage projects)
grip init --from images/builder/Dockerfile --from images/runtime/Dockerfile

# Skip the prompt and accept all verified entries
grip init --yes

# Skip GitHub repo checks (useful on air-gapped machines)
grip init --offline
```

### What gets imported

Given this Dockerfile:

```dockerfile
FROM debian:bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    jq=1.6-2.1ubuntu3 \
    ripgrep \
    libssl-dev \
    pkg-config \
    build-essential \
    ca-certificates \
 && rm -rf /var/lib/apt/lists/*
```

`grip init` produces this `grip.toml`:

```toml
[binaries.jq]
source = "apt"
package = "jq"
version = "1.6-2.1ubuntu3"

[binaries.ripgrep]
source = "apt"
package = "ripgrep"
binary = "rg"

[libraries.libssl-dev]
source = "apt"
package = "libssl-dev"

[libraries.pkg-config]
source = "apt"
package = "pkg-config"

[libraries.build-essential]
source = "apt"
package = "build-essential"

[libraries.ca-certificates]
source = "apt"
package = "ca-certificates"
```

### Generate grip.lock inside the base image

When the project builds on a Linux base image but you are on macOS (or a different Linux
distro), generate the lock file by running grip inside a matching container:

```sh
docker run --rm -v "$PWD":/work -w /work debian:bookworm \
  sh -c '
    apt-get update && apt-get install -y curl ca-certificates
    curl -fsSL https://github.com/omnone/grip/releases/latest/download/grip-x86_64-linux \
         -o /usr/local/bin/grip && chmod +x /usr/local/bin/grip
    grip sync --locked
  '
```

### Replace the Dockerfile apt-get block

After committing `grip.toml` and `grip.lock`, replace the manual `apt-get install` block:

```sh
grip export --format dockerfile >> Dockerfile.new
# Review, then replace the old RUN block
```

### Full workflow

```sh
grip init --yes                              # import from Dockerfile
docker run ... grip sync --locked            # generate grip.lock
grip export --format dockerfile              # regenerate Dockerfile snippet
git add grip.toml grip.lock Dockerfile
git commit -m "migrate deps to grip"
```

See also: [`grip init` flags](COMMANDS.md#grip-init), [SECURITY.md](SECURITY.md)

---

## 2. Source: GitHub releases

GitHub releases are the recommended source for binary tools distributed as pre-built
archives. grip resolves semver ranges, verifies SHA-256, and optionally checks GPG
signatures.

### Basic add

```sh
# owner/repo shorthand — binary name is the last segment
grip add BurntSushi/ripgrep

# Explicit flags
grip add rg --source github --repo BurntSushi/ripgrep

# Pin to a specific tag
grip add rg --source github --repo BurntSushi/ripgrep --version 14.1.0
```

### Semver range

```toml
[binaries.kubectl]
source  = "github"
repo    = "kubernetes/kubectl"
version = "^1.30"   # resolves to highest 1.30.x; locked in grip.lock
```

### Custom asset name template

When the release asset name doesn't match grip's auto-detection:

```toml
[binaries.protoc]
source        = "github"
repo          = "protocolbuffers/protobuf"
version       = "26.1"
asset_pattern = "protoc-*-linux-x86_64.zip"
binary        = "bin/protoc"
```

### GPG verification (fingerprint pin)

```toml
[binaries.age]
source          = "github"
repo            = "FiloSottile/age"
version         = "1.2.0"
gpg_fingerprint = "FBE0D0E4B1E715668D8D60D89E8C9D94557AADC7"
```

See also: [SECURITY.md — GPG verification](SECURITY.md)

---

## 3. Source: apt

Use `apt` for packages that need system-level integration or are only distributed through
the OS package manager. grip uses `apt-get` with privilege escalation and records the
installed version in `grip.lock`.

### Binary tool from apt

```toml
[binaries.ripgrep]
source  = "apt"
package = "ripgrep"
binary  = "rg"        # on-PATH command differs from package name
version = "14.1.0"    # optional: pin to a specific version
```

### Version-pinned install

```toml
[binaries.jq]
source  = "apt"
package = "jq"
version = "1.6-2.1ubuntu3"   # exact apt version string from apt-cache show
```

### Custom apt source and GPG key

```toml
[libraries.docker-ce]
source      = "apt"
package     = "docker-ce"
apt_sources = ["deb [arch=amd64] https://download.docker.com/linux/debian bookworm stable"]
gpg_keys    = ["https://download.docker.com/linux/debian/gpg"]
```

### Debconf preseed

```toml
[libraries.mysql-server]
source              = "apt"
package             = "mysql-server"
debconf_selections  = ["mysql-server mysql-server/root_password password secret"]
```

See also: [`grip add --library`](COMMANDS.md#grip-add-name)

---

## 4. Source: dnf

Use `dnf` for Fedora / RHEL / Rocky / AlmaLinux environments. The workflow mirrors `apt`
but uses `dnf install` with RPM-style version strings.

### Binary tool from dnf

```toml
[binaries.jq]
source  = "dnf"
package = "jq"
version = "1.6"
```

### Custom dnf repo and GPG key

```toml
[libraries.docker-ce]
source    = "dnf"
package   = "docker-ce"
dnf_repos = ["https://download.docker.com/linux/fedora/docker-ce.repo"]
gpg_keys  = ["https://download.docker.com/linux/fedora/gpg"]
```

### Extra dnf flags

```toml
[binaries.kubectl]
source    = "dnf"
package   = "kubectl"
dnf_flags = ["--setopt=install_weak_deps=False"]
```

---

## 5. Pinning and reproducibility

Pinning ensures every developer and CI run installs exactly the same version of every tool.

### Pin all unpinned entries at once

```sh
grip sync                  # install (resolves floating "latest" versions)
grip lock pin              # write the resolved versions back into grip.toml
git add grip.toml grip.lock
git commit -m "pin all tool versions"
```

### Preview what would be pinned

```sh
grip lock pin --dry-run
```

### Check whether the lockfile is current

```sh
grip lock --check          # exit 1 if a re-lock would change grip.lock
```

### See what has newer versions available

```sh
grip lock --upgrade --dry-run   # show outdated tools without writing anything
```

### Upgrade a single tool

```sh
grip lock --upgrade-package jq   # re-resolve jq to latest and update grip.lock
grip sync                         # install the upgraded version
```

### Upgrade all tools

```sh
grip lock --upgrade    # re-resolve everything to latest and update grip.lock
grip sync              # install upgraded versions
```

### Lock file commands

```sh
grip lock                # resolve versions from grip.toml → write grip.lock (no install)
grip lock --check        # assert lock is up to date; exit 1 if stale
grip lock pin            # write installed versions from grip.lock into grip.toml
grip lock pin --dry-run  # preview what would be pinned
grip lock verify         # re-hash every .bin/ binary against grip.lock; exits 1 on mismatch
```

### CI enforcement

```sh
grip suggest --check                  # fail if any tool is referenced but not declared
grip lock --check                     # fail if grip.lock is stale
grip sync --locked --require-pins     # install; fail if lock would change or any entry floats
grip lock verify                      # detect tampering between sync and execution
grip sync --check                     # verify installed binaries match version + SHA-256
```

See also: [SECURITY.md — CI setup](SECURITY.md)

---

## 6. Troubleshooting

### GPG verification failed

```
error: GPG signature verification failed for 'mytool': fingerprint mismatch
hint: The release asset may have been tampered with, or the key is not in your keyring.
      Import the maintainer's key with `gpg --recv-keys <fingerprint>` and re-run.
```

Import the key first, then re-run `grip sync`. If the fingerprint does not match the key
you trust, do not install the binary and report the issue to the upstream project.

### Insufficient privileges (sudo / rootless)

```
error: Insufficient privileges: apt-get requires root
hint: Run grip as root, or configure passwordless sudo for apt-get/dnf.
      Alternatively, switch the entry to `source = "github"` or `source = "url"` for a
      sudo-free install into .bin/.
```

Change the entry source to `github` or `url` to install without root:

```toml
# Before (requires sudo)
[binaries.jq]
source = "apt"

# After (sudo-free)
[binaries.jq]
source = "github"
repo   = "jqlang/jq"
```

### Lock mismatch in CI (`--locked` fails)

```
error: Checksum mismatch: expected abc123…, got def456…
hint: Re-run `grip sync` to re-download, or update the expected hash in your manifest or lock file.
```

This usually means `grip.lock` was committed without running `grip sync` afterwards, or a
developer ran `grip sync` without `--locked` and the lock file diverged. Fix: run `grip sync`
locally, commit the updated `grip.lock`, and re-run CI.

### owner/repo without `--source` now works

Before this was fixed, `grip add BurntSushi/ripgrep` required `--source github`. It no longer
does — the `owner/repo` format now implies `--source github` automatically:

```sh
grip add BurntSushi/ripgrep           # works, implies --source github
grip add BurntSushi/ripgrep --source github  # also works
grip add BurntSushi/ripgrep --source apt     # error: conflicting source
```

### Unsupported platform

```
error: Unsupported platform for adapter 'apt'
hint: This adapter does not support the current OS; use a different source or add a
      platform-specific entry.
```

Use `platforms` to restrict an entry to the OS that supports it:

```toml
[binaries.ripgrep]
source    = "apt"
package   = "ripgrep"
platforms = ["linux"]

[binaries.ripgrep-mac]
source  = "github"
repo    = "BurntSushi/ripgrep"
platforms = ["darwin"]
```
