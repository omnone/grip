# grip — Security Guide

grip is designed to protect projects from supply chain attacks: compromised upstream releases and tampered binaries. This document explains every security control, when to use each one, and the recommended CI setup.

---

## Threat model

grip defends against the following scenarios:

| Threat | Control |
|--------|---------|
| Compromised upstream release (tampered binary) | GPG signature verification, SHA-256 checksums |
| Binary swapped on disk after install | `grip lock verify`, `grip check` SHA256 drift check |
| Silent auto-upgrade to a compromised release | `--require-pins`, `grip check` unpinned entry warning |
| Lock file hand-edited to remove hash checks | `grip check` missing-sha256 warning |
| Cleartext MITM on download | HTTPS enforced by reqwest; `sha256` field in `grip.toml` |

---

## 1. GPG signature verification

grip can verify GPG signatures on GitHub release assets and direct-URL downloads before installing the binary.

### Prerequisites

```sh
# Install gpg
apt install gnupg        # Debian/Ubuntu
brew install gnupg       # macOS
dnf install gnupg2       # Fedora

# Import the maintainer's public key
gpg --recv-keys <FINGERPRINT>
# or
gpg --import maintainer.asc
```

### Mode 1 — detached binary signature

The release ships a `.sig` or `.asc` file alongside the binary.

**GitHub:**

```toml
[binaries.jq]
source         = "github"
repo           = "jqlang/jq"
version        = "1.7.1"
gpg_fingerprint = "634365D9472D7468F"   # maintainer key fingerprint
# sig_asset_pattern = "*.asc"          # optional; auto-detected if omitted
```

grip downloads the asset and its detached signature, then runs:

```sh
gpg --status-fd 1 --verify <sig> <asset>
```

It parses the `[GNUPG:] VALIDSIG` line and checks that the signing key's fingerprint ends with the value you supplied (so you can give a short key ID or a full 40-character fingerprint).

**URL:**

```toml
[binaries.mytool]
source          = "url"
url             = "https://example.com/releases/mytool-1.0-linux-amd64.tar.gz"
gpg_fingerprint = "AF436C3B58B2E3B2"
sig_url         = "https://example.com/releases/mytool-1.0-linux-amd64.tar.gz.sig"
```

### Mode 2 — signed checksums file

The release ships a `SHA256SUMS` (or similar) file signed with GPG. This is the pattern used by HashiCorp, Go, jq, and many other projects.

**GitHub:**

```toml
[binaries.terraform]
source                  = "github"
repo                    = "hashicorp/terraform"
version                 = "1.7.0"
gpg_fingerprint         = "34365D9472D7468F"
checksums_asset_pattern = "*SHA256SUMS"     # glob to find the checksums file
sig_asset_pattern       = "*SHA256SUMS.sig" # glob to find the checksums file's signature
```

Verification steps:
1. Find the checksums asset via `checksums_asset_pattern`
2. Find its detached signature via `sig_asset_pattern` (auto-detected if omitted)
3. Verify the GPG signature of the checksums file
4. Look up the downloaded asset's filename in the checksums file
5. Compare with the actual SHA-256 of the downloaded file

**URL:**

```toml
[binaries.mytool]
source               = "url"
url                  = "https://example.com/mytool-1.0-linux-amd64.tar.gz"
gpg_fingerprint      = "AF436C3B58B2E3B2"
signed_checksums_url = "https://example.com/SHA256SUMS"
checksums_sig_url    = "https://example.com/SHA256SUMS.sig"
```

### Fingerprint format

`gpg_fingerprint` accepts:
- A full 40-character fingerprint: `"34365D9472D7468F41E9F5E69A2E76EC4FCB0E5C"`
- A long key ID (16 hex characters): `"34365D9472D7468F"`

The check is a suffix match: the `VALIDSIG` line's fingerprint must end with the value you supply.

### Finding a maintainer's fingerprint

```sh
# From a keyserver
gpg --search-keys <maintainer@email.com>

# From a release page (common patterns)
gpg --recv-keys <fingerprint shown on release page>
gpg --fingerprint <keyid>
```

---

## 2. Lock file integrity (`grip lock verify`)

After install, grip records the SHA-256 of every downloaded binary in `grip.lock`. `grip lock verify` re-hashes every `.bin/` binary and compares it against the lock — without re-downloading anything.

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

On any mismatch:

```
  ✗  jq: checksum mismatch — lock: abc123...  disk: deadbeef...

  1 mismatch(es) detected — possible tampering!
```

Exits `1` if any binary fails. Suitable for CI.

### When to run it

| Scenario | Command |
|----------|---------|
| After `git pull` (check nothing changed) | `grip lock verify` |
| Before a deploy | `grip lock verify` |
| In CI after `grip sync` | `grip lock verify` |
| Periodic audit | `grip lock verify` |

### Difference from `grip check`

| | `grip check` | `grip lock verify` |
|---|---|---|
| Reads | manifest + lock | lock file only |
| Fails on | not-yet-installed entries | tampered/replaced binaries |
| Network | none | none |
| Use case | "is my setup complete?" | "was anything modified after install?" |

---

## 3. Version pins and `--require-pins`

An entry with no `version` pin silently upgrades to whatever is latest on every `grip sync`. If an upstream release is ever compromised, an unpinned entry picks it up automatically.

### Pinning an entry

```sh
grip add jq@1.7.1 --repo jqlang/jq --source github
# or edit grip.toml:
# version = "1.7.1"
```

### Enforcing pins in CI

```sh
grip sync --require-pins
```

If any entry lacks a version pin, grip exits before touching the network:

```
error: the following entries have no version pin: jq, rg
       Run `grip sync` without `--require-pins` to install the latest versions,
       then pin them with `grip add <name>@<version>`.
hint:  Pin each entry by adding a version: `grip add <name>@<version>`, ...
```

### What counts as pinned

| Source | Pinned when |
|--------|-------------|
| `github` | `version = "x.y.z"` is set |
| `apt` | `version = "x.y"` is set |
| `dnf` | `version = "x.y"` is set |
| `url` | always — the URL itself is the pin |

---

## 4. `grip check` security checks

`grip check` reports the following security-relevant issues in addition to per-entry verification:

| Check | What it detects |
|-------|----------------|
| SHA256 drift (check 6) | Binary on disk doesn't match the hash in `grip.lock` — may indicate post-install tampering |
| Missing sha256 in lock (check 8) | A `github` or `url` entry has no sha256 recorded — lock may have been hand-edited |
| Unpinned entries (check 10) | An entry has no version pin — silent auto-upgrade risk in CI |

Run after `git pull` or as part of a pre-commit hook:

```sh
grip pin    # pin any unpinned entries first
grip check
```

---

## Recommended CI configuration

### Minimal (reproducibility only)

```yaml
- run: grip sync --locked
```

Fails the build if `grip.lock` would change — prevents version drift.

### Standard (reproducibility + tamper detection)

```yaml
- run: grip sync --locked --require-pins
- run: grip lock verify
```

`--require-pins` catches floating versions before install.  
`grip lock verify` re-hashes every `.bin/` binary after install.

### Maximum (+ GPG verification)

Add `gpg_fingerprint` (and optionally `checksums_asset_pattern`) to each `github` or `url` entry in `grip.toml`. Requires the maintainer's public key to be in the CI keyring:

```yaml
- run: gpg --recv-keys <fingerprint>
- run: grip sync --locked --require-pins
- run: grip lock verify
```

Or import from a file you commit to the repo:

```yaml
- run: gpg --import ci/maintainer-keys.asc
- run: grip sync --locked --require-pins
- run: grip lock verify
```

---

## What grip cannot protect against

- **Signed git commits on `grip.toml`** — grip doesn't sign the manifest file itself. Use `git commit --gpg-sign` and enforce it via branch protection rules. If a PR can silently change `repo = "evil/fork"`, GPG verification doesn't help until the key is also compromised.
- **Compromised GPG key** — if the maintainer's signing key is compromised, grip trusts the signed release. Keep key imports auditable (store them in the repo under `ci/`).
- **Post-install execution** — grip does not sandbox installed binaries. A tampered binary that passes signature verification can still do damage when executed.
