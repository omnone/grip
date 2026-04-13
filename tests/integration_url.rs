//! Integration tests for the URL adapter.
//!
//! These tests run inside the container built by `make test-integration-url`.
//! They require network access to download binaries from direct URLs.
//! The pinned URL points to a stable release of jq so the tests remain reproducible.
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

// Pinned jq 1.7.1 raw Linux amd64 binary.
const JQ_URL: &str = "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64";

// ── tests ─────────────────────────────────────────────────────────────────────

/// `grip sync` downloads jq from a direct URL and places it in `.bin/`.
#[test]
#[ignore]
fn url_sync_installs_binary() {
    if !in_container() {
        return;
    }

    let project = setup_project(&format!(
        r#"
[binaries]
jq = {{ source = "url", url = "{JQ_URL}" }}
"#
    ));

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(
        project.path().join(".bin/jq").exists(),
        ".bin/jq not created"
    );
    assert!(
        project.path().join("grip.lock").exists(),
        "grip.lock missing"
    );
}

/// `grip check` passes after a successful `grip sync`.
#[test]
#[ignore]
fn url_check_passes_after_sync() {
    if !in_container() {
        return;
    }

    let project = setup_project(&format!(
        r#"
[binaries]
jq = {{ source = "url", url = "{JQ_URL}" }}
"#
    ));

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["check"]), "grip check");
}

/// `grip list` shows the installed entry after sync.
#[test]
#[ignore]
fn url_list_shows_installed_entry() {
    if !in_container() {
        return;
    }

    let project = setup_project(&format!(
        r#"
[binaries]
jq = {{ source = "url", url = "{JQ_URL}" }}
"#
    ));

    assert_success(&grip(project.path(), &["sync"]), "grip sync");

    let out = grip(project.path(), &["list"]);
    assert_success(&out, "grip list");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("jq"),
        "expected 'jq' in grip list output, got: {stdout}"
    );
}

/// Running `grip sync` twice is idempotent.
#[test]
#[ignore]
fn url_sync_is_idempotent() {
    if !in_container() {
        return;
    }

    let project = setup_project(&format!(
        r#"
[binaries]
jq = {{ source = "url", url = "{JQ_URL}" }}
"#
    ));

    assert_success(&grip(project.path(), &["sync"]), "first grip sync");
    assert_success(
        &grip(project.path(), &["sync"]),
        "second grip sync (idempotent)",
    );
    assert!(project.path().join(".bin/jq").exists());
}

/// A wrong `sha256` value causes `grip sync` to fail.
#[test]
#[ignore]
fn url_sync_bad_checksum_fails() {
    if !in_container() {
        return;
    }

    let project = setup_project(&format!(
        r#"
[binaries]
jq = {{ source = "url", url = "{JQ_URL}", sha256 = "0000000000000000000000000000000000000000000000000000000000000000" }}
"#
    ));

    let out = grip(project.path(), &["sync"]);
    assert!(
        !out.status.success(),
        "expected sync to fail with wrong sha256, but it succeeded"
    );
}

/// `grip remove` deletes the binary and the lock file entry.
#[test]
#[ignore]
fn url_remove_deletes_entry() {
    if !in_container() {
        return;
    }

    let project = setup_project(&format!(
        r#"
[binaries]
jq = {{ source = "url", url = "{JQ_URL}" }}
"#
    ));

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(project.path().join(".bin/jq").exists());

    assert_success(&grip(project.path(), &["remove", "jq"]), "grip remove jq");

    assert!(
        !project.path().join(".bin/jq").exists(),
        ".bin/jq still exists after remove"
    );
    let out = grip(project.path(), &["list"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("jq"),
        "jq still appears in grip list after remove"
    );
}
