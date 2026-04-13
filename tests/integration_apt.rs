//! Integration tests for the APT adapter.
//!
//! These tests only run inside the dedicated Docker container built by
//! `make test-integration-apt`.  They are guarded by two conditions:
//!
//!   1. `#[ignore]`  — skipped by plain `cargo test`; requires `--include-ignored`.
//!   2. `GRIP_INTEGRATION_TESTS=1` env var — must be set explicitly (the Dockerfile
//!      sets it).  If absent the test returns immediately so nothing executes on the
//!      developer's host even when `--include-ignored` is passed.
//!
//! Every test creates an isolated temporary project directory and invokes the
//! compiled `grip` binary via `std::process::Command`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

// ── helpers ────────────────────────────────────────────────────────────────────

/// Path to the compiled `grip` binary produced by `cargo test`.
fn grip_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_grip"))
}

/// Returns `true` only when the `GRIP_INTEGRATION_TESTS` env var is set to `1`.
/// This is the single gate that keeps every test from running on the host.
fn in_container() -> bool {
    std::env::var("GRIP_INTEGRATION_TESTS").as_deref() == Ok("1")
}

/// Create a temporary project directory pre-populated with the given `grip.toml` content.
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

/// Assert the command succeeded; print stdout/stderr on failure.
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

/// `grip sync` installs `jq` via apt and places a symlink in `.bin/`.
#[test]
#[ignore]
fn apt_sync_installs_binary() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "apt", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(project.path().join(".bin/jq").exists(), ".bin/jq not created");
    assert!(project.path().join("grip.lock").exists(), "grip.lock missing");
}

/// `grip check` passes after a successful `grip sync`.
#[test]
#[ignore]
fn apt_check_passes_after_sync() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "apt", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["check"]), "grip check");
}

/// `grip list` prints the installed entry after sync.
#[test]
#[ignore]
fn apt_list_shows_installed_entry() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "apt", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");

    let out = grip(project.path(), &["list"]);
    assert_success(&out, "grip list");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("jq"), "expected 'jq' in grip list output, got: {stdout}");
}

/// Running `grip sync` twice is idempotent: the second call skips the already-installed binary.
#[test]
#[ignore]
fn apt_sync_is_idempotent() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "apt", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "first grip sync");
    assert_success(&grip(project.path(), &["sync"]), "second grip sync (idempotent)");
    assert!(project.path().join(".bin/jq").exists());
}

/// `grip remove` deletes the `.bin/` symlink and the lock file entry.
#[test]
#[ignore]
fn apt_remove_deletes_entry() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "apt", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(project.path().join(".bin/jq").exists());

    assert_success(&grip(project.path(), &["remove", "jq"]), "grip remove jq");

    assert!(!project.path().join(".bin/jq").exists(), ".bin/jq still exists after remove");

    let out = grip(project.path(), &["list"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("jq"), "jq still appears in grip list after remove");
}

/// Packages with a different on-PATH binary name work via the `binary` field.
/// `fd-find` installs as `fdfind` on Debian; the `binary` field maps it to `fd`.
#[test]
#[ignore]
fn apt_binary_field_remaps_executable_name() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
fd = { source = "apt", package = "fd-find", binary = "fdfind" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(project.path().join(".bin/fd").exists(), ".bin/fd not created");
}

/// An optional (`required = false`) entry that fails does not fail `grip sync`.
#[test]
#[ignore]
fn apt_optional_entry_failure_is_warning() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
nonexistent-grip-test-pkg = { source = "apt", package = "nonexistent-grip-test-pkg-xyz", required = false }
"#,
    );

    let out = grip(project.path(), &["sync"]);
    assert!(
        out.status.success(),
        "grip sync should exit 0 when only optional entries fail; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `grip sync` installs a library entry (no binary) and records it in the lock file.
#[test]
#[ignore]
fn apt_library_install() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[libraries]
zlib = { source = "apt", package = "zlib1g-dev" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync (library)");

    let lock = std::fs::read_to_string(project.path().join("grip.lock"))
        .expect("grip.lock missing after library install");
    assert!(lock.contains("zlib"), "expected 'zlib' in grip.lock, got:\n{lock}");
}

/// `grip sync --verify` re-checks SHA-256 of an already-installed binary.
#[test]
#[ignore]
fn apt_sync_verify_passes_on_clean_install() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "apt", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["sync", "--verify"]), "grip sync --verify");
}

/// `grip doctor` reports no issues for a clean project.
#[test]
#[ignore]
fn apt_doctor_clean_project() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "apt", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["doctor"]), "grip doctor");
}

/// Multiple binaries install concurrently without conflict.
#[test]
#[ignore]
fn apt_sync_multiple_binaries() {
    if !in_container() { return; }

    let project = setup_project(
        r#"
[binaries]
jq  = { source = "apt", package = "jq" }
git = { source = "apt", package = "git" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync (multiple)");
    assert!(project.path().join(".bin/jq").exists(), ".bin/jq missing");
    assert!(project.path().join(".bin/git").exists(), ".bin/git missing");
}
