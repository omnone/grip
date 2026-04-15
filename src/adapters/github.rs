//! Adapter that downloads binaries from GitHub releases.

use async_trait::async_trait;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};

use std::sync::Arc;

use crate::adapters::SourceAdapter;
use crate::cache::Cache;
use crate::checksum::ChecksumWriter;
use crate::config::lockfile::LockEntry;
use crate::config::manifest::{BinaryEntry, GithubEntry};
use crate::error::GripError;
use crate::gpg::{verify_gpg_signature, verify_signed_checksums};
use crate::output;
use crate::platform::Platform;

/// Downloads and extracts a binary from a GitHub release asset.
/// Supported on all platforms.
pub struct GithubAdapter {
    pub platform: Platform,
    pub cache: Option<Arc<Cache>>,
}

/// Minimal GitHub releases API response.
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

/// A single asset attached to a GitHub release.
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[async_trait]
impl SourceAdapter for GithubAdapter {
    fn name(&self) -> &str {
        "github"
    }

    fn is_supported(&self) -> bool {
        true
    }

    async fn resolve_latest(
        &self,
        entry: &BinaryEntry,
        client: &Client,
    ) -> Result<String, GripError> {
        let BinaryEntry::Github(g) = entry else {
            return Err(GripError::Other("expected github entry".into()));
        };
        if let Some(v) = &g.version {
            if is_version_range(v) {
                let req = VersionReq::parse(v)
                    .map_err(|e| GripError::Other(format!("invalid semver range '{v}': {e}")))?;
                return resolve_range(&g.repo, &req, client).await;
            }
            return Ok(v.clone());
        }
        let url = format!("https://api.github.com/repos/{}/releases/latest", g.repo);
        let release: Release = client
            .get(&url)
            .header("User-Agent", "grip/0.1")
            .send()
            .await?
            .error_for_status()
            .map_err(|e| GripError::GitHubApi(e.to_string()))?
            .json()
            .await?;
        Ok(release.tag_name.trim_start_matches('v').to_string())
    }

    async fn install(
        &self,
        name: &str,
        entry: &BinaryEntry,
        bin_dir: &Path,
        client: &Client,
        pb: ProgressBar,
        colored: bool,
    ) -> Result<LockEntry, GripError> {
        let BinaryEntry::Github(g) = entry else {
            return Err(GripError::Other("expected github entry".into()));
        };

        pb.set_message(format!("{name}  resolving version..."));
        let version = self.resolve_latest(entry, client).await?;

        // Try common tag formats: v1.2.3, 1.2.3, name-1.2.3
        let candidate_tags = vec![
            format!("v{}", version),
            version.clone(),
            format!("{}-{}", name, version),
        ];

        let mut release: Option<Release> = None;
        for tag in &candidate_tags {
            let release_url = format!(
                "https://api.github.com/repos/{}/releases/tags/{}",
                g.repo, tag
            );
            let resp = client
                .get(&release_url)
                .header("User-Agent", "grip/0.1")
                .send()
                .await?;
            if resp.status().is_success() {
                release = Some(resp.json().await?);
                break;
            }
        }
        let release = release.ok_or_else(|| {
            GripError::GitHubApi(format!(
                "Could not find release for '{}' with tags: {}",
                version,
                candidate_tags.join(", ")
            ))
        })?;

        // Match asset
        let pattern = g.asset_pattern.clone().unwrap_or_else(|| name.to_string());

        let asset = match_asset(&release.assets, &pattern, &self.platform)
            .ok_or_else(|| GripError::NoMatchingAsset(pattern.clone()))?;

        let download_url = asset.browser_download_url.clone();
        let asset_size = asset.size;
        let asset_name = asset.name.clone();

        // Download (or use cached archive)
        let tmp = tempfile::NamedTempFile::new()?;
        let _archive_sha256 = if let Some(cache) = &self.cache {
            if let Some(cached) = cache.lookup(&download_url) {
                pb.set_message(format!("{name}  {asset_name} (cached)"));
                std::fs::copy(&cached, tmp.path())?;
                crate::checksum::sha256_file(tmp.path()).map_err(GripError::Io)?
            } else {
                pb.set_message(format!("{name}  fetching {asset_name}"));
                let sha = download_with_progress(
                    client,
                    &download_url,
                    tmp.path(),
                    name,
                    asset_size,
                    &pb,
                    colored,
                )
                .await?;
                cache.store(&download_url, tmp.path()).ok();
                sha
            }
        } else {
            pb.set_message(format!("{name}  fetching {asset_name}"));
            download_with_progress(
                client,
                &download_url,
                tmp.path(),
                name,
                asset_size,
                &pb,
                colored,
            )
            .await?
        };

        // GPG / signed-checksums verification (opt-in via gpg_fingerprint in grip.toml)
        if let Some(fingerprint) = &g.gpg_fingerprint {
            pb.set_message(format!("{name}  verifying signature..."));

            if let Some(checksums_pat) = &g.checksums_asset_pattern {
                // ── Mode 2: signed checksums file ────────────────────────────
                // Find the checksums asset, then find the signature of that checksums file.
                let checksums_asset = find_asset_by_pattern(&release.assets, checksums_pat)
                    .ok_or_else(|| GripError::GpgVerificationFailed {
                        name: name.to_string(),
                        detail: format!(
                            "no checksums asset matched pattern '{}'; \
                                 check checksums_asset_pattern in grip.toml",
                            checksums_pat
                        ),
                    })?;
                let checksums_name = checksums_asset.name.clone();
                let checksums_url = checksums_asset.browser_download_url.clone();

                let checksums_sig_asset = find_sig_asset(
                    &release.assets,
                    &checksums_name,
                    g.sig_asset_pattern.as_deref(),
                )
                .ok_or_else(|| GripError::GpgVerificationFailed {
                    name: name.to_string(),
                    detail: format!(
                        "no signature asset found for checksums file '{}'; \
                                 set sig_asset_pattern in grip.toml to locate it explicitly",
                        checksums_name
                    ),
                })?;

                let checksums_tmp = tempfile::NamedTempFile::new()?;
                let sig_tmp = tempfile::NamedTempFile::new()?;
                download_asset(client, &checksums_url, checksums_tmp.path()).await?;
                download_asset(
                    client,
                    &checksums_sig_asset.browser_download_url,
                    sig_tmp.path(),
                )
                .await?;

                verify_signed_checksums(
                    tmp.path(),
                    checksums_tmp.path(),
                    sig_tmp.path(),
                    fingerprint,
                    &asset_name,
                    name,
                )?;
            } else {
                // ── Mode 1: direct binary signature ──────────────────────────
                let sig_asset =
                    find_sig_asset(&release.assets, &asset_name, g.sig_asset_pattern.as_deref())
                        .ok_or_else(|| GripError::GpgVerificationFailed {
                            name: name.to_string(),
                            detail: format!(
                                "no signature asset found in release for '{}'; \
                                 set sig_asset_pattern in grip.toml to locate it explicitly",
                                asset_name
                            ),
                        })?;

                let sig_tmp = tempfile::NamedTempFile::new()?;
                download_asset(client, &sig_asset.browser_download_url, sig_tmp.path()).await?;
                verify_gpg_signature(tmp.path(), sig_tmp.path(), fingerprint, name)?;
            }
        }

        // Extract or copy
        pb.set_message(format!("{name}  extracting..."));
        let extra_installed = extract_binary(tmp.path(), &asset_name, g, name, bin_dir)?;

        // Hash the installed binary, not the archive, so `grip lock verify` can re-check it.
        let binary_sha256 = crate::checksum::sha256_file(&bin_dir.join(name))
            .map_err(GripError::Io)?;

        // Symlink into ~/.local/bin/ so the binary is on PATH without `grip env`.
        crate::bin_dir::link_to_user_path(bin_dir, name).ok();
        for extra in &extra_installed {
            crate::bin_dir::link_to_user_path(bin_dir, extra).ok();
        }

        pb.finish_with_message(format!(
            "{} {name}  {version}",
            output::success_checkmark(colored)
        ));
        Ok(LockEntry {
            name: name.to_string(),
            version,
            source: "github".to_string(),
            url: Some(download_url),
            sha256: Some(binary_sha256),
            installed_at: chrono::Utc::now(),
            extra_binaries: extra_installed,
            auto_binary: None,
            auto_extra_binaries: vec![],
        })
    }
}

/// Returns `true` if `v` looks like a semver range rather than a pinned version.
fn is_version_range(v: &str) -> bool {
    let t = v.trim();
    t.starts_with(['^', '~', '>', '<', '=', '*']) || t.contains(".*") || t.contains(" ")
}

/// Fetch up to 100 releases from GitHub and return the highest version matching `req`.
async fn resolve_range(repo: &str, req: &VersionReq, client: &Client) -> Result<String, GripError> {
    #[derive(Deserialize)]
    struct ReleaseTag {
        tag_name: String,
    }

    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=100");
    let releases: Vec<ReleaseTag> = client
        .get(&url)
        .header("User-Agent", "grip/0.1")
        .send()
        .await?
        .error_for_status()
        .map_err(|e| GripError::GitHubApi(e.to_string()))?
        .json()
        .await?;

    releases
        .iter()
        .filter_map(|r| {
            let stripped = r.tag_name.trim_start_matches('v');
            Version::parse(stripped)
                .ok()
                .map(|v| (v, stripped.to_string()))
        })
        .filter(|(v, _)| req.matches(v))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, s)| s)
        .ok_or_else(|| GripError::GitHubApi(format!("No release matching '{req}' found in {repo}")))
}

/// Find the best matching asset for the given `pattern` and `platform`.
/// Tries an exact glob match first, then falls back to a platform-aware heuristic.
fn match_asset<'a>(assets: &'a [Asset], pattern: &str, platform: &Platform) -> Option<&'a Asset> {
    // First try exact pattern match
    if let Ok(g) = glob::Pattern::new(pattern) {
        if let Some(a) = assets.iter().find(|a| g.matches(&a.name)) {
            return Some(a);
        }
    }

    // Try platform-aware heuristic
    let os = platform.os_str();
    let arch = platform.arch_str();
    assets.iter().find(|a| {
        let n = a.name.to_lowercase();
        n.contains(os) && (n.contains(arch) || n.contains("x86_64") || n.contains("amd64"))
    })
}

/// Stream `url` to `dest`, updating `pb` with download progress, and return the hex SHA-256.
async fn download_with_progress(
    client: &Client,
    url: &str,
    dest: &Path,
    label: &str,
    size: u64,
    pb: &ProgressBar,
    colored: bool,
) -> Result<String, GripError> {
    if size > 0 {
        pb.set_length(size);
        let tpl = if colored {
            "  {prefix:.bold.dim} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})"
        } else {
            "  {prefix} {msg} [{bar:40}] {bytes}/{total_bytes} ({eta})"
        };
        pb.set_style(
            ProgressStyle::default_bar()
                .template(tpl)
                .unwrap()
                .progress_chars("█▓░"),
        );
    } else {
        let tpl = if colored {
            "  {prefix:.bold.dim} {spinner:.cyan} {msg} {bytes}"
        } else {
            "  {prefix} {spinner} {msg} {bytes}"
        };
        pb.set_style(
            ProgressStyle::with_template(tpl)
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
    }

    let mut resp = client.get(url).send().await?.error_for_status()?;
    let file = std::fs::File::create(dest)?;
    let mut writer = ChecksumWriter::new(std::io::BufWriter::new(file));

    while let Some(chunk) = resp.chunk().await? {
        writer.write_all(&chunk)?;
        pb.inc(chunk.len() as u64);
    }

    // Restore spinner style for the extract step
    let tpl = if colored {
        "  {prefix:.bold.dim} {spinner:.cyan} {msg}"
    } else {
        "  {prefix} {spinner} {msg}"
    };
    pb.set_style(
        ProgressStyle::with_template(tpl)
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(format!("{label}  extracting..."));

    let (_, sha256) = writer.finalize();
    Ok(sha256)
}

/// Unpack `archive_path` (or copy it if it is a raw binary), locate the target binary (and any
/// extra binaries), copy them into `bin_dir`, and make them executable.
/// Returns the list of extra binary names that were successfully installed.
fn extract_binary(
    archive_path: &Path,
    asset_name: &str,
    g: &GithubEntry,
    binary_name: &str,
    bin_dir: &Path,
) -> Result<Vec<String>, GripError> {
    let tmp_dir = tempfile::tempdir()?;
    let lower = asset_name.to_lowercase();

    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        extract_tar_gz(archive_path, tmp_dir.path())?;
    } else if lower.ends_with(".tar.bz2") {
        extract_tar_bz2(archive_path, tmp_dir.path())?;
    } else if lower.ends_with(".zip") {
        extract_zip(archive_path, tmp_dir.path())?;
    } else {
        // Raw binary — extra_binaries are not extractable from a non-archive
        let final_dest = bin_dir.join(binary_name);
        std::fs::copy(archive_path, &final_dest)?;
        crate::bin_dir::make_executable(&final_dest)?;
        return Ok(vec![]);
    }

    // Find and install the primary binary
    let want = g.binary.as_deref().unwrap_or(binary_name);
    let found = find_binary_in_dir(tmp_dir.path(), want)?;
    let final_dest = bin_dir.join(binary_name);
    std::fs::copy(&found, &final_dest)?;
    crate::bin_dir::make_executable(&final_dest)?;

    // Find and install any extra binaries from the same archive
    let mut extra_installed: Vec<String> = Vec::new();
    for extra in g.extra_binaries.iter().flat_map(|v| v.iter()) {
        let extra_found = find_binary_in_dir(tmp_dir.path(), extra)?;
        let extra_dest = bin_dir.join(extra);
        std::fs::copy(&extra_found, &extra_dest)?;
        crate::bin_dir::make_executable(&extra_dest)?;
        extra_installed.push(extra.clone());
    }

    let _ = tmp_dir.keep();
    Ok(extra_installed)
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), GripError> {
    let file = std::fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest)?;
    Ok(())
}

fn extract_tar_bz2(archive: &Path, dest: &Path) -> Result<(), GripError> {
    let file = std::fs::File::open(archive)?;
    let bz = bzip2::read::BzDecoder::new(file);
    let mut tar = tar::Archive::new(bz);
    tar.unpack(dest)?;
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), GripError> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| GripError::Other(e.to_string()))?;
    zip.extract(dest)
        .map_err(|e| GripError::Other(e.to_string()))?;
    Ok(())
}

/// Recursively search `dir` for a file named `name`, returning an error if not found.
fn find_binary_in_dir(dir: &Path, name: &str) -> Result<PathBuf, GripError> {
    walkdir_find(dir, name).ok_or_else(|| GripError::BinaryNotFound(name.to_string()))
}

/// Depth-first recursive file search; returns the first path whose filename matches `name`.
fn walkdir_find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = walkdir_find(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

/// Find a release asset whose name matches `pattern` (glob).
fn find_asset_by_pattern<'a>(assets: &'a [Asset], pattern: &str) -> Option<&'a Asset> {
    let g = glob::Pattern::new(pattern).ok()?;
    assets.iter().find(|a| g.matches(&a.name))
}

/// Download `url` to `dest` without a progress bar (used for small ancillary files like
/// signature files and checksums).
async fn download_asset(
    client: &Client,
    url: &str,
    dest: &std::path::Path,
) -> Result<(), GripError> {
    let mut resp = client
        .get(url)
        .header("User-Agent", "grip/0.1")
        .send()
        .await?
        .error_for_status()
        .map_err(|e| GripError::GitHubApi(e.to_string()))?;
    let mut file = std::fs::File::create(dest)?;
    while let Some(chunk) = resp.chunk().await? {
        file.write_all(&chunk)?;
    }
    Ok(())
}

/// Find the detached signature asset for `asset_name` in the release asset list.
///
/// If `sig_pattern` is provided it is used as a glob; otherwise common naming conventions
/// are tried in order: `<asset>.sig`, `<asset>.asc`, `checksums.txt.sig`, `checksums.txt.asc`,
/// `SHA256SUMS.asc`, `SHA256SUMS.sig`.
fn find_sig_asset<'a>(
    assets: &'a [Asset],
    asset_name: &str,
    sig_pattern: Option<&str>,
) -> Option<&'a Asset> {
    if let Some(pat) = sig_pattern {
        if let Ok(g) = glob::Pattern::new(pat) {
            return assets.iter().find(|a| g.matches(&a.name));
        }
    }
    let candidates = [
        format!("{asset_name}.sig"),
        format!("{asset_name}.asc"),
        "checksums.txt.sig".to_string(),
        "checksums.txt.asc".to_string(),
        "SHA256SUMS.asc".to_string(),
        "SHA256SUMS.sig".to_string(),
        "checksums.sig".to_string(),
        "checksums.asc".to_string(),
    ];
    assets
        .iter()
        .find(|a| candidates.iter().any(|c| c == &a.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── find_sig_asset ────────────────────────────────────────────────────────

    fn make_assets(names: &[&str]) -> Vec<Asset> {
        names
            .iter()
            .map(|n| Asset {
                name: n.to_string(),
                browser_download_url: format!("https://example.com/{n}"),
                size: 0,
            })
            .collect()
    }

    #[test]
    fn find_sig_asset_prefers_explicit_pattern() {
        let assets = make_assets(&["tool.tar.gz", "tool.tar.gz.sig", "tool.tar.gz.asc"]);
        let found = find_sig_asset(&assets, "tool.tar.gz", Some("*.asc"));
        assert_eq!(found.map(|a| a.name.as_str()), Some("tool.tar.gz.asc"));
    }

    #[test]
    fn find_sig_asset_auto_detects_dot_sig() {
        let assets = make_assets(&["tool.tar.gz", "tool.tar.gz.sig"]);
        let found = find_sig_asset(&assets, "tool.tar.gz", None);
        assert_eq!(found.map(|a| a.name.as_str()), Some("tool.tar.gz.sig"));
    }

    #[test]
    fn find_sig_asset_auto_detects_dot_asc() {
        let assets = make_assets(&["tool.tar.gz", "tool.tar.gz.asc"]);
        let found = find_sig_asset(&assets, "tool.tar.gz", None);
        assert_eq!(found.map(|a| a.name.as_str()), Some("tool.tar.gz.asc"));
    }

    #[test]
    fn find_sig_asset_auto_detects_checksums_asc() {
        let assets = make_assets(&["tool.tar.gz", "checksums.txt.asc"]);
        let found = find_sig_asset(&assets, "tool.tar.gz", None);
        assert_eq!(found.map(|a| a.name.as_str()), Some("checksums.txt.asc"));
    }

    #[test]
    fn find_sig_asset_returns_none_when_absent() {
        let assets = make_assets(&["tool.tar.gz", "tool.tar.gz.sha256"]);
        let found = find_sig_asset(&assets, "tool.tar.gz", None);
        assert!(found.is_none());
    }
}
