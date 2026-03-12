//! Adapter that downloads binaries directly from an arbitrary URL.

use async_trait::async_trait;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use std::io::Write;
use std::path::Path;

use crate::adapters::SourceAdapter;
use crate::bin_dir::copy_binary;
use crate::checksum::ChecksumWriter;
use crate::config::lockfile::LockEntry;
use crate::config::manifest::BinaryEntry;
use crate::error::GripError;

/// Downloads a binary (or archive) from a URL, optionally verifies its SHA-256, and installs it.
/// Supported on all platforms.
pub struct UrlAdapter;

#[async_trait]
impl SourceAdapter for UrlAdapter {
    fn name(&self) -> &str {
        "url"
    }

    fn is_supported(&self) -> bool {
        true
    }

    async fn resolve_latest(&self, entry: &BinaryEntry, _client: &Client) -> Result<String, GripError> {
        let BinaryEntry::Url(u) = entry else {
            return Err(GripError::Other("expected url entry".into()));
        };
        Ok(u.url.clone())
    }

    async fn install(
        &self,
        name: &str,
        entry: &BinaryEntry,
        bin_dir: &Path,
        client: &Client,
        pb: ProgressBar,
    ) -> Result<LockEntry, GripError> {
        let BinaryEntry::Url(u) = entry else {
            return Err(GripError::Other("expected url entry".into()));
        };

        pb.set_message(format!("{name}  connecting..."));
        let resp = client.get(&u.url).send().await?.error_for_status()?;
        let total = resp.content_length().unwrap_or(0);

        if total > 0 {
            pb.set_length(total);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("  {prefix:.bold.dim} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                    .unwrap()
                    .progress_chars("█▓░"),
            );
        }
        pb.set_message(format!("{name}  downloading"));

        let tmp = tempfile::NamedTempFile::new()?;
        let file = std::fs::File::create(tmp.path())?;
        let mut writer = ChecksumWriter::new(std::io::BufWriter::new(file));

        // Drop first resp (it was just for content-length), make a fresh request
        drop(resp);
        let mut resp2 = client.get(&u.url).send().await?.error_for_status()?;
        while let Some(chunk) = resp2.chunk().await? {
            writer.write_all(&chunk)?;
            pb.inc(chunk.len() as u64);
        }

        let (_, sha256) = writer.finalize();

        // Verify expected checksum if provided
        if let Some(expected) = &u.sha256 {
            if expected != &sha256 {
                return Err(GripError::ChecksumMismatch {
                    expected: expected.clone(),
                    got: sha256,
                });
            }
        }

        // Extract if archive
        pb.set_message(format!("{name}  extracting..."));
        let asset_name = u.url.split('/').last().unwrap_or("download");
        let lower = asset_name.to_lowercase();
        let binary_name = u.binary.as_deref().unwrap_or(name);

        let tmp_dir = tempfile::tempdir()?;
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            extract_tar_gz(tmp.path(), tmp_dir.path())?;
            let found = find_in_dir(tmp_dir.path(), binary_name)?;
            copy_binary(&found, bin_dir, name)?;
        } else if lower.ends_with(".zip") {
            extract_zip(tmp.path(), tmp_dir.path())?;
            let found = find_in_dir(tmp_dir.path(), binary_name)?;
            copy_binary(&found, bin_dir, name)?;
        } else {
            copy_binary(tmp.path(), bin_dir, name)?;
        }

        let version: String = sha256.chars().take(12).collect();
        pb.finish_with_message(format!("\x1b[32m✓\x1b[0m {name}  {version}"));
        Ok(LockEntry {
            name: name.to_string(),
            version,
            source: "url".to_string(),
            url: Some(u.url.clone()),
            sha256: Some(sha256),
            installed_at: chrono::Utc::now(),
        })
    }
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), GripError> {
    let file = std::fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest)?;
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), GripError> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| GripError::Other(e.to_string()))?;
    zip.extract(dest).map_err(|e| GripError::Other(e.to_string()))?;
    Ok(())
}

/// Recursively search `dir` for a file named `name`, returning an error if not found.
fn find_in_dir(dir: &Path, name: &str) -> Result<std::path::PathBuf, GripError> {
    find_recursive(dir, name).ok_or_else(|| GripError::BinaryNotFound(name.to_string()))
}

/// Depth-first recursive file search; returns the first path whose filename matches `name`.
fn find_recursive(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_recursive(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}
