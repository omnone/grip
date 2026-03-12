//! CLI argument definitions parsed by clap.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "grip", about = "Binary dependency manager", version)]
pub struct Cli {
    /// Override the project root directory (skips the binaries.toml walk).
    /// Useful inside containers where the project root is known.
    #[arg(long, global = true, value_name = "DIR")]
    pub root: Option<std::path::PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize binaries.toml in current directory
    Init,
    /// Add a binary entry to binaries.toml
    Add {
        name: String,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        package: Option<String>,
    },
    /// Install all binaries from binaries.toml
    Install {
        #[arg(long, help = "Fail if lock file would change (CI mode)")]
        locked: bool,
        #[arg(long, help = "Verify SHA256 of each binary against the lock file")]
        verify: bool,
        #[arg(long, help = "Only install binaries with this tag")]
        tag: Option<String>,
    },
    /// Run a command with .bin/ prepended to PATH
    Run {
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// List installed binaries
    List,
    /// Update a binary to its latest version
    Update {
        name: String,
    },
}
