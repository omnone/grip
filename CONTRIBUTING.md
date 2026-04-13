# Contributing to grip

Thanks for your interest! Contributions of all kinds are welcome — bug reports, documentation improvements, new features, and adapter additions.

---

## Prerequisites

- **Rust stable** — install via [rustup.rs](https://rustup.rs/)
- **Docker** — required for integration tests (not for unit tests or building)

---

## Build

```sh
cargo build            # debug build
cargo build --release  # release build; binary at target/release/grip
```

---

## Tests

### Unit tests

Run without Docker — fast, no network required:

```sh
cargo test
```

### Integration tests (requires Docker)

Each adapter has its own isolated test suite that runs inside a dedicated container. They are ordered roughly from fastest to slowest:

```sh
make test-integration-shell    # Shell adapter — no network, ~5 s
make test-integration-apt      # APT adapter  — Debian Bookworm container
make test-integration-dnf      # DNF adapter  — Fedora 40 container
make test-integration-url      # URL adapter  — downloads from GitHub CDN
make test-integration-github   # GitHub adapter — GitHub API + CDN
```

Run all suites sequentially:

```sh
make test-integration
```

> Tests are guarded by `GRIP_INTEGRATION_TESTS=1` (set inside the container) and `#[ignore]`, so they never run accidentally on the host with plain `cargo test`.

---

## Project structure

```
src/
├── main.rs          # CLI routing and command implementations
├── cli.rs           # Clap argument definitions
├── installer.rs     # Concurrent adapter orchestration
├── checker.rs       # Verification logic
├── adapters/        # One file per source adapter
│   ├── mod.rs       # SourceAdapter trait + factory
│   ├── github.rs
│   ├── apt.rs
│   ├── dnf.rs
│   ├── url.rs
│   └── shell.rs
└── config/
    ├── manifest.rs  # grip.toml types
    └── lockfile.rs  # grip.lock types + I/O
tests/
├── integration_apt.rs
├── integration_dnf.rs
├── integration_github.rs
├── integration_url.rs
├── integration_shell.rs
└── docker/          # One Dockerfile per integration suite
```

---

## Adding a new adapter

1. Create `src/adapters/<name>.rs` implementing the `SourceAdapter` async trait:
   ```rust
   pub trait SourceAdapter: Send + Sync {
       fn name(&self) -> &str;
       fn is_supported(&self) -> bool;
       async fn install(...) -> Result<LockEntry, GripError>;
       async fn resolve_latest(...) -> Result<String, GripError>;
   }
   ```

2. Add a new variant to `BinaryEntry` in `src/config/manifest.rs` and a corresponding entry struct with `source = "<name>"` TOML tag.

3. Wire it in `get_adapter()` in `src/adapters/mod.rs`.

4. Add an integration test file (`tests/integration_<name>.rs`), a Dockerfile (`tests/docker/Dockerfile.test-<name>`), and a Makefile target (`test-integration-<name>`).

---

## PR guidelines

- **One logical change per PR** — easier to review and revert.
- **Integration tests must pass** — run at least the relevant suite before opening a PR.
- **Keep `main.rs` thin** — dispatch only; implementation logic belongs in modules.
- **Preserve TOML key order** — always use `IndexMap`, never `HashMap`, for manifest/lock structures.
- **No panics in the hot path** — propagate errors via `GripError`; reserve `expect`/`unwrap` for truly impossible states.
