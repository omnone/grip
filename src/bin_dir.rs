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
