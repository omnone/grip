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

    /// Returns `true` when running on macOS.
    pub fn is_macos(&self) -> bool {
        self.os == OS::MacOS
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
