//! Upfront privilege detection for system package manager operations.
//!
//! Rather than silently retrying commands with `sudo`, we check once whether the
//! current process can run the package manager, and fail fast with a clear message if not.

use std::process::Command;

use crate::error::GripError;

/// How to invoke a privileged command.
#[derive(Debug, Clone, Copy)]
pub enum PrivilegeMode {
    /// Running as root — invoke the command directly.
    Root,
    /// Non-root with passwordless sudo — prefix the command with `sudo`.
    Sudo,
}

/// Detect whether the current process can run privileged commands.
///
/// Returns [`PrivilegeMode::Root`] if UID is 0, [`PrivilegeMode::Sudo`] if
/// `sudo -n true` succeeds (passwordless sudo), or an
/// [`GripError::InsufficientPrivileges`] error otherwise.
pub fn check_privileges() -> Result<PrivilegeMode, GripError> {
    let is_root = Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false);

    if is_root {
        return Ok(PrivilegeMode::Root);
    }

    let has_passwordless_sudo = Command::new("sudo")
        .args(["-n", "true"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_passwordless_sudo {
        return Ok(PrivilegeMode::Sudo);
    }

    Err(GripError::InsufficientPrivileges {
        hint: "re-run as root or ensure passwordless sudo is configured".into(),
    })
}
