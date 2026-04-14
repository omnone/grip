//! Source adapters — one per installation method (apt, dnf, github, url).

use async_trait::async_trait;
use indicatif::ProgressBar;
use reqwest::Client;
use std::path::Path;
use std::sync::Arc;

use crate::cache::Cache;
use crate::config::lockfile::LockEntry;
use crate::config::manifest::BinaryEntry;
use crate::error::GripError;
use crate::platform::Platform;

pub mod apt;
pub mod dnf;
pub mod github;
pub mod url;

/// Common interface implemented by every source adapter.
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    /// Short identifier for the adapter (e.g. `"github"`, `"apt"`).
    fn name(&self) -> &str;
    /// Returns `true` if this adapter can run on the current platform.
    fn is_supported(&self) -> bool;
    /// Install the binary described by `entry` into `bin_dir` and return a lock entry.
    /// `pb` is a spinner progress bar pre-configured by the caller; adapters should update
    /// its message and style as work progresses, and call `finish_with_message` on success.
    async fn install(
        &self,
        name: &str,
        entry: &BinaryEntry,
        bin_dir: &Path,
        client: &Client,
        pb: ProgressBar,
        colored: bool,
    ) -> Result<LockEntry, GripError>;
    /// Resolve the latest available version string without installing.
    async fn resolve_latest(
        &self,
        entry: &BinaryEntry,
        client: &Client,
    ) -> Result<String, GripError>;
}

/// Construct the appropriate [`SourceAdapter`] for the given manifest entry.
pub fn get_adapter(entry: &BinaryEntry, cache: Option<Arc<Cache>>) -> Box<dyn SourceAdapter> {
    let platform = Platform::current();
    match entry {
        BinaryEntry::Apt(_) => Box::new(apt::AptAdapter { platform }),
        BinaryEntry::Dnf(_) => Box::new(dnf::DnfAdapter { platform }),
        BinaryEntry::Github(_) => Box::new(github::GithubAdapter { platform, cache }),
        BinaryEntry::Url(_) => Box::new(url::UrlAdapter { cache }),
    }
}
