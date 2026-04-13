//! Platform detection utilities.

/// The detected operating system and CPU architecture of the current host.
#[derive(Debug, Clone, PartialEq)]
pub struct Platform {
    pub os: OS,
    pub arch: Arch,
}

/// Operating system variants recognised by grip.
#[derive(Debug, Clone, PartialEq)]
pub enum OS {
    Linux,
    MacOS,
    Windows,
    Other(String),
}

/// CPU architecture variants recognised by grip.
#[derive(Debug, Clone, PartialEq)]
pub enum Arch {
    X86_64,
    Aarch64,
    Other(String),
}

impl Platform {
    /// Detect the platform of the running process.
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "linux" => OS::Linux,
            "macos" => OS::MacOS,
            "windows" => OS::Windows,
            other => OS::Other(other.to_string()),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            other => Arch::Other(other.to_string()),
        };
        Platform { os, arch }
    }

    /// Returns `true` when running on Linux.
    pub fn is_linux(&self) -> bool {
        self.os == OS::Linux
    }

    /// Returns a lowercase OS identifier string compatible with GitHub release asset names
    /// (e.g. `"linux"`, `"darwin"`, `"windows"`).
    pub fn os_str(&self) -> &str {
        match &self.os {
            OS::Linux => "linux",
            OS::MacOS => "darwin",
            OS::Windows => "windows",
            OS::Other(s) => s.as_str(),
        }
    }

    /// Returns a lowercase architecture identifier string compatible with GitHub release asset names
    /// (e.g. `"amd64"`, `"arm64"`).
    pub fn arch_str(&self) -> &str {
        match &self.arch {
            Arch::X86_64 => "amd64",
            Arch::Aarch64 => "arm64",
            Arch::Other(s) => s.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(os: OS, arch: Arch) -> Platform {
        Platform { os, arch }
    }

    // ── os_str ────────────────────────────────────────────────────────────────

    #[test]
    fn os_str_linux() {
        assert_eq!(make(OS::Linux, Arch::X86_64).os_str(), "linux");
    }

    #[test]
    fn os_str_macos() {
        assert_eq!(make(OS::MacOS, Arch::X86_64).os_str(), "darwin");
    }

    #[test]
    fn os_str_windows() {
        assert_eq!(make(OS::Windows, Arch::X86_64).os_str(), "windows");
    }

    #[test]
    fn os_str_other() {
        assert_eq!(make(OS::Other("freebsd".into()), Arch::X86_64).os_str(), "freebsd");
    }

    // ── arch_str ──────────────────────────────────────────────────────────────

    #[test]
    fn arch_str_x86_64() {
        assert_eq!(make(OS::Linux, Arch::X86_64).arch_str(), "amd64");
    }

    #[test]
    fn arch_str_aarch64() {
        assert_eq!(make(OS::Linux, Arch::Aarch64).arch_str(), "arm64");
    }

    #[test]
    fn arch_str_other() {
        assert_eq!(make(OS::Linux, Arch::Other("riscv64".into())).arch_str(), "riscv64");
    }

    // ── is_linux ──────────────────────────────────────────────────────────────

    #[test]
    fn is_linux_true_for_linux() {
        assert!(make(OS::Linux, Arch::X86_64).is_linux());
    }

    #[test]
    fn is_linux_false_for_other() {
        assert!(!make(OS::MacOS, Arch::X86_64).is_linux());
        assert!(!make(OS::Windows, Arch::X86_64).is_linux());
    }

    // ── Platform::current ─────────────────────────────────────────────────────

    #[test]
    fn current_returns_valid_platform() {
        let p = Platform::current();
        // os_str must be non-empty and arch_str must be non-empty
        assert!(!p.os_str().is_empty());
        assert!(!p.arch_str().is_empty());
    }
}
