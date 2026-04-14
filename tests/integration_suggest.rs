//! Integration tests for `grip suggest`.
//!
//! Unlike the adapter suites (apt, dnf, github, url) these tests do NOT require a
//! Docker container or network access — `grip suggest` is purely local file analysis.
//! They run with plain `cargo test` on any developer machine.
//!
//! Each test creates an isolated temporary directory, optionally writes source files
//! into it, then invokes the compiled `grip` binary as a subprocess and asserts on
//! the exit code.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

// ── helpers ────────────────────────────────────────────────────────────────────

fn grip_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_grip"))
}

/// Run `grip --quiet --root <dir> suggest [args]`.
fn suggest(dir: &Path, extra: &[&str]) -> Output {
    Command::new(grip_bin())
        .arg("--quiet")
        .arg("--root")
        .arg(dir)
        .arg("suggest")
        .args(extra)
        .output()
        .expect("grip invocation failed")
}

fn exit_code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

// ── tests ──────────────────────────────────────────────────────────────────────

/// With no scan sources and no shell history, `grip suggest` exits 0.
#[test]
fn suggest_exits_zero_when_nothing_to_scan() {
    let dir = TempDir::new().expect("TempDir");
    let out = suggest(dir.path(), &[]);
    assert_eq!(exit_code(&out), 0, "expected exit 0, got:\n{}", stderr(&out));
}

/// `--check` on a clean project (no suggestions) still exits 0.
#[test]
fn suggest_check_exits_zero_on_clean_project() {
    let dir = TempDir::new().expect("TempDir");
    let out = suggest(dir.path(), &["--check"]);
    assert_eq!(
        exit_code(&out),
        0,
        "--check on clean project must exit 0, got:\n{}",
        stderr(&out)
    );
}

/// A Python file referencing a known tool via `subprocess.run` triggers a suggestion.
/// Without `--check`, the command exits 0 regardless of findings.
#[test]
fn suggest_exits_zero_without_check_flag_even_with_findings() {
    let dir = TempDir::new().expect("TempDir");
    let src = dir.path().join("script.py");
    std::fs::write(&src, "import subprocess\nsubprocess.run([\"jq\", \".\"])\n")
        .expect("write script.py");

    let out = suggest(dir.path(), &["--path", src.to_str().unwrap()]);
    assert_eq!(
        exit_code(&out),
        0,
        "without --check, grip suggest should always exit 0; got:\n{}",
        stderr(&out)
    );
}

/// `--check` exits 1 when an unmanaged tool is found in a scanned source file.
#[test]
fn suggest_check_exits_one_when_findings_exist() {
    let dir = TempDir::new().expect("TempDir");
    let src = dir.path().join("script.py");
    std::fs::write(&src, "import subprocess\nsubprocess.run([\"jq\", \".\"])\n")
        .expect("write script.py");

    let out = suggest(
        dir.path(),
        &["--check", "--path", src.to_str().unwrap()],
    );
    assert_eq!(
        exit_code(&out),
        1,
        "--check should exit 1 when unmanaged tools are found; got:\n{}",
        stderr(&out)
    );
}

/// When the detected tool is already declared in `grip.toml`, `--check` exits 0.
#[test]
fn suggest_check_exits_zero_when_tool_already_declared() {
    let dir = TempDir::new().expect("TempDir");

    std::fs::write(
        dir.path().join("grip.toml"),
        "[binaries.jq]\nsource = \"github\"\nrepo = \"jqlang/jq\"\n",
    )
    .expect("write grip.toml");

    let src = dir.path().join("script.py");
    std::fs::write(&src, "import subprocess\nsubprocess.run([\"jq\", \".\"])\n")
        .expect("write script.py");

    let out = suggest(
        dir.path(),
        &["--check", "--path", src.to_str().unwrap()],
    );
    assert_eq!(
        exit_code(&out),
        0,
        "jq is declared in grip.toml — --check should exit 0; got:\n{}",
        stderr(&out)
    );
}

/// `/bin/<name>` path literals in source files are detected by the universal scanner.
#[test]
fn suggest_check_detects_bin_path_literals() {
    let dir = TempDir::new().expect("TempDir");
    let src = dir.path().join("deploy.py");
    // Uses a /usr/bin/ path literal — caught by scan_binary_paths, not a language pattern.
    std::fs::write(&src, "os.execv(\"/usr/bin/jq\", [\"/usr/bin/jq\", \".\"])\n")
        .expect("write deploy.py");

    let out = suggest(
        dir.path(),
        &["--check", "--path", src.to_str().unwrap()],
    );
    assert_eq!(
        exit_code(&out),
        1,
        "/usr/bin/jq path literal should be detected; got:\n{}",
        stderr(&out)
    );
}

/// A Go file using `exec.Command` is picked up by the Go language scanner.
#[test]
fn suggest_check_detects_go_exec_command() {
    let dir = TempDir::new().expect("TempDir");
    let src = dir.path().join("main.go");
    std::fs::write(
        &src,
        "package main\nimport \"os/exec\"\nfunc main() { exec.Command(\"jq\", \".\") }\n",
    )
    .expect("write main.go");

    let out = suggest(
        dir.path(),
        &["--check", "--path", src.to_str().unwrap()],
    );
    assert_eq!(
        exit_code(&out),
        1,
        "exec.Command(\"jq\") in Go should be detected; got:\n{}",
        stderr(&out)
    );
}

// ── util ───────────────────────────────────────────────────────────────────────

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}
