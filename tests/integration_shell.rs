//! Integration tests for the Shell adapter.
//!
//! These tests run inside the container built by `make test-integration-shell`.
//! They do not require network access — the install commands are pure shell.
//! `GRIP_BIN_DIR` is set by grip so the install command can place the binary
//! in the correct location.
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

// ── tests ─────────────────────────────────────────────────────────────────────

/// `grip sync` runs the install command and the binary appears in `.bin/`.
/// The install command copies `/bin/true` (a tiny always-succeeding executable)
/// into `$GRIP_BIN_DIR` under the name `hello`.
#[test]
#[ignore]
fn shell_sync_installs_binary() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
hello = { source = "shell", install_cmd = "cp /bin/true $GRIP_BIN_DIR/hello", version = "1.0.0" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(
        project.path().join(".bin/hello").exists(),
        ".bin/hello not created"
    );
    assert!(
        project.path().join("grip.lock").exists(),
        "grip.lock missing"
    );
}

/// `grip check` passes after a successful `grip sync`.
#[test]
#[ignore]
fn shell_check_passes_after_sync() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
hello = { source = "shell", install_cmd = "cp /bin/true $GRIP_BIN_DIR/hello", version = "1.0.0" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["check"]), "grip check");
}

/// `grip list` shows the installed shell entry after sync.
#[test]
#[ignore]
fn shell_list_shows_installed_entry() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
hello = { source = "shell", install_cmd = "cp /bin/true $GRIP_BIN_DIR/hello", version = "1.0.0" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");

    let out = grip(project.path(), &["list"]);
    assert_success(&out, "grip list");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("hello"),
        "expected 'hello' in grip list output, got: {stdout}"
    );
}

/// Running `grip sync` twice is idempotent: the second call skips the already-installed entry.
#[test]
#[ignore]
fn shell_sync_is_idempotent() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
hello = { source = "shell", install_cmd = "cp /bin/true $GRIP_BIN_DIR/hello", version = "1.0.0" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "first grip sync");
    assert_success(
        &grip(project.path(), &["sync"]),
        "second grip sync (idempotent)",
    );
    assert!(project.path().join(".bin/hello").exists());
}

/// `grip remove` deletes the binary from `.bin/` and its lock file entry.
#[test]
#[ignore]
fn shell_remove_deletes_entry() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
hello = { source = "shell", install_cmd = "cp /bin/true $GRIP_BIN_DIR/hello", version = "1.0.0" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(project.path().join(".bin/hello").exists());

    assert_success(
        &grip(project.path(), &["remove", "hello"]),
        "grip remove hello",
    );

    assert!(
        !project.path().join(".bin/hello").exists(),
        ".bin/hello still exists after remove"
    );
    let out = grip(project.path(), &["list"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("hello"),
        "hello still appears in grip list after remove"
    );
}

/// The `version` field recorded in the lock file matches what the manifest declares.
#[test]
#[ignore]
fn shell_version_recorded_in_lockfile() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
hello = { source = "shell", install_cmd = "cp /bin/true $GRIP_BIN_DIR/hello", version = "2.5.0" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");

    let lock =
        std::fs::read_to_string(project.path().join("grip.lock")).expect("grip.lock missing");
    assert!(
        lock.contains("2.5.0"),
        "expected version '2.5.0' in grip.lock, got:\n{lock}"
    );
}

/// A failing shell command that is marked `required = false` produces a warning, not a failure.
#[test]
#[ignore]
fn shell_optional_failure_is_warning() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
broken = { source = "shell", install_cmd = "exit 1", required = false }
"#,
    );

    assert_success(
        &grip(project.path(), &["sync"]),
        "grip sync with optional broken entry",
    );
    assert!(!project.path().join(".bin/broken").exists());
}
