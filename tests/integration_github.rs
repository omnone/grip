//! Integration tests for the GitHub release adapter.
//!
//! These tests run inside the container built by `make test-integration-github`.
//! They require network access to api.github.com and GitHub's release CDN.
//! All binaries are pinned to specific versions so tests remain stable.
//!
//! Guard conditions (same as the apt/dnf suites):
//!   1. `#[ignore]` — skipped by plain `cargo test`; requires `--include-ignored`.
//!   2. `GRIP_INTEGRATION_TESTS=1` env var — set by the Dockerfile.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

// ── helpers ────────────────────────────────────────────────────────────────────

fn grip_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_grip"))
}

fn in_container() -> bool {
    std::env::var("GRIP_INTEGRATION_TESTS").as_deref() == Ok("1")
}

fn setup_project(toml: &str) -> TempDir {
    let tmp = TempDir::new().expect("TempDir");
    std::fs::write(tmp.path().join("grip.toml"), toml).expect("write grip.toml");
    tmp
}

/// Run `grip` with `--quiet --root <dir>` followed by `args`.
fn grip(dir: &Path, args: &[&str]) -> Output {
    Command::new(grip_bin())
        .arg("--quiet")
        .arg("--root")
        .arg(dir)
        .args(args)
        .output()
        .expect("grip invocation")
}

fn assert_success(out: &Output, context: &str) {
    if !out.status.success() {
        panic!(
            "{context} failed (exit {:?}):\nstdout: {}\nstderr: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// `grip sync` installs jq from a GitHub release and places it in `.bin/`.
#[test]
#[ignore]
fn github_sync_installs_binary() {
    if !in_container() { return; }

    let project = setup_project(r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1", asset_pattern = "jq-linux-amd64" }
"#);

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(project.path().join(".bin/jq").exists(), ".bin/jq not created");
    assert!(project.path().join("grip.lock").exists(), "grip.lock missing");
}

/// `grip check` passes after a successful `grip sync`.
#[test]
#[ignore]
fn github_check_passes_after_sync() {
    if !in_container() { return; }

    let project = setup_project(r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1", asset_pattern = "jq-linux-amd64" }
"#);

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["check"]), "grip check");
}

/// `grip list` prints the installed entry after sync.
#[test]
#[ignore]
fn github_list_shows_installed_entry() {
    if !in_container() { return; }

    let project = setup_project(r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1", asset_pattern = "jq-linux-amd64" }
"#);

    assert_success(&grip(project.path(), &["sync"]), "grip sync");

    let out = grip(project.path(), &["list"]);
    assert_success(&out, "grip list");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("jq"), "expected 'jq' in grip list output, got: {stdout}");
}

/// Running `grip sync` twice is idempotent: the second call skips the already-installed binary.
#[test]
#[ignore]
fn github_sync_is_idempotent() {
    if !in_container() { return; }

    let project = setup_project(r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1", asset_pattern = "jq-linux-amd64" }
"#);

    assert_success(&grip(project.path(), &["sync"]), "first grip sync");
    assert_success(&grip(project.path(), &["sync"]), "second grip sync (idempotent)");
    assert!(project.path().join(".bin/jq").exists());
}

/// `grip sync --verify` passes after a clean install (SHA-256 stored in lock file matches).
#[test]
#[ignore]
fn github_sync_verify_passes_on_clean_install() {
    if !in_container() { return; }

    let project = setup_project(r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1", asset_pattern = "jq-linux-amd64" }
"#);

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["sync", "--verify"]), "grip sync --verify");
}

/// `grip remove` deletes the binary from `.bin/` and its lock file entry.
#[test]
#[ignore]
fn github_remove_deletes_entry() {
    if !in_container() { return; }

    let project = setup_project(r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1", asset_pattern = "jq-linux-amd64" }
"#);

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(project.path().join(".bin/jq").exists());

    assert_success(&grip(project.path(), &["remove", "jq"]), "grip remove jq");

    assert!(!project.path().join(".bin/jq").exists(), ".bin/jq still exists after remove");
    let out = grip(project.path(), &["list"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("jq"), "jq still appears in grip list after remove");
}

/// Multiple GitHub binaries install without conflict.
#[test]
#[ignore]
fn github_sync_multiple_binaries() {
    if !in_container() { return; }

    let project = setup_project(r#"
[binaries]
jq = { source = "github", repo = "jqlang/jq", version = "1.7.1", asset_pattern = "jq-linux-amd64" }
yq = { source = "github", repo = "mikefarah/yq", version = "4.44.1", asset_pattern = "yq_linux_amd64" }
"#);

    assert_success(&grip(project.path(), &["sync"]), "grip sync (multiple)");
    assert!(project.path().join(".bin/jq").exists(), ".bin/jq missing");
    assert!(project.path().join(".bin/yq").exists(), ".bin/yq missing");
}

/// A missing GitHub repo that is marked `required = false` is a warning, not a hard failure.
#[test]
#[ignore]
fn github_optional_entry_failure_is_warning() {
    if !in_container() { return; }

    let project = setup_project(r#"
[binaries]
broken = { source = "github", repo = "grip-test/does-not-exist-xyz123", required = false }
"#);

    assert_success(&grip(project.path(), &["sync"]), "grip sync with optional broken entry");
    assert!(!project.path().join(".bin/broken").exists());
}
