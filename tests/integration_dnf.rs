//! Integration tests for the DNF adapter.
//!
//! These tests only run inside the dedicated Docker container built by
//! `make test-integration-dnf`.  They are guarded by two conditions:
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
        .arg("--project")
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

/// `grip sync` installs `jq` via dnf and places a symlink in `.bin/`.
#[test]
#[ignore]
fn dnf_sync_installs_binary() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "dnf", package = "jq" }
"#,
    );

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

/// `grip sync --check` passes after a successful `grip sync` and `grip lock pin`.
#[test]
#[ignore]
fn dnf_check_passes_after_sync() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "dnf", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["lock", "pin"]), "grip lock pin");
    assert_success(&grip(project.path(), &["sync", "--check"]), "grip sync --check");
}

/// `grip tree` prints the installed entry after sync.
#[test]
#[ignore]
fn dnf_list_shows_installed_entry() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "dnf", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");

    let out = grip(project.path(), &["tree"]);
    assert_success(&out, "grip tree");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("jq"),
        "expected 'jq' in grip tree output, got: {stdout}"
    );
}

/// Running `grip sync` twice is idempotent: the second call skips already-installed binaries.
#[test]
#[ignore]
fn dnf_sync_is_idempotent() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "dnf", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "first grip sync");
    assert_success(
        &grip(project.path(), &["sync"]),
        "second grip sync (idempotent)",
    );
    assert!(project.path().join(".bin/jq").exists());
}

/// `grip remove` deletes the `.bin/` symlink and the lock file entry.
#[test]
#[ignore]
fn dnf_remove_deletes_entry() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "dnf", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(project.path().join(".bin/jq").exists());

    assert_success(&grip(project.path(), &["remove", "jq"]), "grip remove jq");

    assert!(
        !project.path().join(".bin/jq").exists(),
        ".bin/jq still exists after remove"
    );

    let out = grip(project.path(), &["tree"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("jq"),
        "jq still appears in grip tree after remove"
    );
}

/// Packages with a different on-PATH binary name work via the `binary` field.
/// `ripgrep` installs the `rg` binary — a canonical remap case on Fedora.
#[test]
#[ignore]
fn dnf_binary_field_remaps_executable_name() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
rg = { source = "dnf", package = "ripgrep", binary = "rg" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert!(
        project.path().join(".bin/rg").exists(),
        ".bin/rg not created"
    );
}

/// An optional (`required = false`) entry that fails does not fail `grip sync`.
#[test]
#[ignore]
fn dnf_optional_entry_failure_is_warning() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
nonexistent-grip-test-pkg = { source = "dnf", package = "nonexistent-grip-test-pkg-xyz", required = false }
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
fn dnf_library_install() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[libraries]
zlib = { source = "dnf", package = "zlib-devel" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync (library)");

    let lock = std::fs::read_to_string(project.path().join("grip.lock"))
        .expect("grip.lock missing after library install");
    assert!(
        lock.contains("zlib"),
        "expected 'zlib' in grip.lock, got:\n{lock}"
    );
}

/// `grip sync --verify` re-checks SHA-256 of an already-installed binary.
#[test]
#[ignore]
fn dnf_sync_verify_passes_on_clean_install() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "dnf", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(
        &grip(project.path(), &["sync", "--verify"]),
        "grip sync --verify",
    );
}

/// `grip sync --check` reports no issues for a clean project after pinning.
#[test]
#[ignore]
fn dnf_check_clean_project() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
jq = { source = "dnf", package = "jq" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync");
    assert_success(&grip(project.path(), &["lock", "pin"]), "grip lock pin");
    assert_success(&grip(project.path(), &["sync", "--check"]), "grip sync --check");
}

/// Multiple binaries install concurrently without conflict.
#[test]
#[ignore]
fn dnf_sync_multiple_binaries() {
    if !in_container() {
        return;
    }

    let project = setup_project(
        r#"
[binaries]
jq  = { source = "dnf", package = "jq" }
git = { source = "dnf", package = "git" }
"#,
    );

    assert_success(&grip(project.path(), &["sync"]), "grip sync (multiple)");
    assert!(project.path().join(".bin/jq").exists(), ".bin/jq missing");
    assert!(project.path().join(".bin/git").exists(), ".bin/git missing");
}
