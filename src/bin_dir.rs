//! Utilities for managing the project-local `.bin/` directory.

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::error::GripError;

/// Create `.bin/` inside `project_root` if it does not exist, and return its path.
pub fn ensure_bin_dir(project_root: &Path) -> Result<PathBuf, GripError> {
    let bin_dir = project_root.join(".bin");
    fs::create_dir_all(&bin_dir)?;
    Ok(bin_dir)
}

/// Copy a binary from `src` into `bin_dir` with the given `name` and make it executable.
pub fn copy_binary(src: &Path, bin_dir: &Path, name: &str) -> Result<PathBuf, GripError> {
    let dest = bin_dir.join(name);
    fs::copy(src, &dest)?;
    make_executable(&dest)?;
    Ok(dest)
}

/// Create a symlink at `bin_dir/name` pointing to `target`.
/// Any pre-existing file or symlink at that path is removed first.
#[cfg(unix)]
pub fn symlink_binary(target: &Path, bin_dir: &Path, name: &str) -> Result<PathBuf, GripError> {
    let link = bin_dir.join(name);
    if link.exists() || link.is_symlink() {
        fs::remove_file(&link)?;
    }
    std::os::unix::fs::symlink(target, &link)?;
    Ok(link)
}

/// Non-Unix fallback: copies the binary instead of symlinking.
#[cfg(not(unix))]
pub fn symlink_binary(target: &Path, bin_dir: &Path, name: &str) -> Result<PathBuf, GripError> {
    // Fallback: copy on non-unix
    copy_binary(target, bin_dir, name)
}

/// Compute SHA256 of the binary at `bin_dir/name`, following symlinks.
pub fn sha256_of_installed(bin_dir: &Path, name: &str) -> Option<String> {
    let path = bin_dir.join(name);
    // Follow symlink to the real file
    let real = fs::canonicalize(&path).ok()?;
    crate::checksum::sha256_file(&real).ok()
}

/// Symlink `~/.local/bin/<name>` → the absolute path of the installed binary in `bin_dir`.
/// Creates `~/.local/bin/` if it does not exist.
/// No-op when `$HOME` is unset or on non-Unix platforms.
pub fn link_to_user_path(bin_dir: &Path, name: &str) -> Result<(), GripError> {
    #[cfg(unix)]
    {
        let home = match std::env::var_os("HOME") {
            Some(h) => h,
            None => return Ok(()),
        };
        let local_bin = std::path::Path::new(&home).join(".local/bin");
        fs::create_dir_all(&local_bin)?;
        let target = fs::canonicalize(bin_dir.join(name))
            .unwrap_or_else(|_| bin_dir.join(name));
        let link = local_bin.join(name);
        if link.exists() || link.is_symlink() {
            fs::remove_file(&link)?;
        }
        std::os::unix::fs::symlink(&target, &link)?;
    }
    Ok(())
}

/// Set the executable bit (`0o755`) on a file. No-op on non-Unix platforms.
pub fn make_executable(path: &Path) -> Result<(), GripError> {
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── ensure_bin_dir ────────────────────────────────────────────────────────

    #[test]
    fn ensure_bin_dir_creates_directory() {
        let tmp = TempDir::new().unwrap();
        let bin = ensure_bin_dir(tmp.path()).unwrap();
        assert_eq!(bin, tmp.path().join(".bin"));
        assert!(bin.is_dir());
    }

    #[test]
    fn ensure_bin_dir_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        ensure_bin_dir(tmp.path()).unwrap();
        // Calling again must not fail
        let bin = ensure_bin_dir(tmp.path()).unwrap();
        assert!(bin.is_dir());
    }

    // ── copy_binary ───────────────────────────────────────────────────────────

    #[test]
    fn copy_binary_places_file_in_bin_dir() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("mybinary");
        std::fs::write(&src, b"ELF stub").unwrap();

        let bin_dir = ensure_bin_dir(tmp.path()).unwrap();
        let dest = copy_binary(&src, &bin_dir, "mybinary").unwrap();

        assert!(dest.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"ELF stub");
    }

    #[cfg(unix)]
    #[test]
    fn copy_binary_makes_file_executable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("mybinary");
        std::fs::write(&src, b"#!/bin/sh\necho hi").unwrap();

        let bin_dir = ensure_bin_dir(tmp.path()).unwrap();
        let dest = copy_binary(&src, &bin_dir, "mybinary").unwrap();

        let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "file should have execute bits set");
    }

    // ── sha256_of_installed ───────────────────────────────────────────────────

    #[test]
    fn sha256_of_installed_returns_hash_for_existing_file() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = ensure_bin_dir(tmp.path()).unwrap();
        std::fs::write(bin_dir.join("tool"), b"hello").unwrap();

        let hash = sha256_of_installed(&bin_dir, "tool");
        assert!(hash.is_some());
        // SHA256 of "hello"
        assert_eq!(
            hash.unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn sha256_of_installed_returns_none_for_missing_file() {
        let tmp = TempDir::new().unwrap();
        let bin_dir = ensure_bin_dir(tmp.path()).unwrap();
        let hash = sha256_of_installed(&bin_dir, "nonexistent");
        assert!(hash.is_none());
    }

    // ── make_executable ───────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn make_executable_sets_execute_bits() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("script.sh");
        std::fs::write(&file, b"#!/bin/sh").unwrap();
        // Remove execute bits first
        let mut perms = std::fs::metadata(&file).unwrap().permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&file, perms).unwrap();

        make_executable(&file).unwrap();

        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0);
    }

    #[test]
    fn make_executable_on_missing_file_returns_error() {
        let result = make_executable(Path::new("/nonexistent/file"));
        // On unix this is an Io error; on non-unix it's a no-op Ok.
        #[cfg(unix)]
        assert!(result.is_err());
        #[cfg(not(unix))]
        assert!(result.is_ok());
    }
}
