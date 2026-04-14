//! Package URL (purl) generation shared by `grip sbom` and `grip audit`.

use crate::config::lockfile::LockEntry;

/// Compute a purl for a lock entry based on its source.
///
/// - `github` → `pkg:github/{owner}/{repo}@{version}` (repo parsed from the download URL)
/// - `apt`    → `pkg:deb/debian/{name}@{version}`
/// - `dnf`    → `pkg:generic/{name}@{upstream-version}`
///             OSV does not index Fedora RPMs; `pkg:generic` with the upstream
///             version (dist-tag stripped) gives the best coverage.
/// - `url` / unknown → `pkg:generic/{name}@{version}`
///
/// Leading `v` is stripped from the version per the purl spec.
pub fn purl_for_entry(entry: &LockEntry) -> String {
    let version = entry.version.trim_start_matches('v');
    match entry.source.as_str() {
        "github" => {
            if let Some((owner, repo)) = entry.url.as_deref().and_then(parse_github_repo) {
                format!("pkg:github/{owner}/{repo}@{version}")
            } else {
                format!("pkg:generic/{}@{version}", entry.name)
            }
        }
        "apt" => format!("pkg:deb/debian/{}@{version}", entry.name),
        "dnf" => format!("pkg:generic/{}@{}", entry.name, strip_rpm_release(version)),
        _ => format!("pkg:generic/{}@{version}", entry.name),
    }
}

/// Strip the RPM release and dist suffix from a version string.
///
/// RPM versions have the form `VERSION-RELEASE[.DIST]` where RELEASE is
/// packaging metadata (e.g. `5.fc43`, `1.el9`).  OSV vulnerability records
/// use only the upstream VERSION, so `8.15.0-5.fc43` → `8.15.0`.
///
/// Only strips when the release segment starts with a digit (the normal RPM
/// convention), leaving upstream versions that legitimately contain `-` alone.
pub fn strip_rpm_release(version: &str) -> &str {
    if let Some(idx) = version.rfind('-') {
        let release = &version[idx + 1..];
        if release.starts_with(|c: char| c.is_ascii_digit()) {
            return &version[..idx];
        }
    }
    version
}

/// Parse `owner` and `repo` from a GitHub release download URL.
///
/// Expected format: `https://github.com/{owner}/{repo}/releases/download/...`
pub fn parse_github_repo(url: &str) -> Option<(String, String)> {
    let path = url.strip_prefix("https://github.com/")?;
    let mut parts = path.splitn(3, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::lockfile::LockEntry;
    use chrono::Utc;

    fn entry(name: &str, version: &str, source: &str, url: Option<&str>) -> LockEntry {
        LockEntry {
            name: name.to_string(),
            version: version.to_string(),
            source: source.to_string(),
            url: url.map(|s| s.to_string()),
            sha256: None,
            installed_at: Utc::now(),
            extra_binaries: vec![],
            auto_binary: None,
            auto_extra_binaries: vec![],
        }
    }

    #[test]
    fn github_purl_with_url() {
        let e = entry(
            "jq",
            "1.7.1",
            "github",
            Some("https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64"),
        );
        assert_eq!(purl_for_entry(&e), "pkg:github/jqlang/jq@1.7.1");
    }

    #[test]
    fn github_purl_strips_v_prefix() {
        let e = entry(
            "rg",
            "v14.1.1",
            "github",
            Some("https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/rg-linux"),
        );
        assert_eq!(purl_for_entry(&e), "pkg:github/BurntSushi/ripgrep@14.1.1");
    }

    #[test]
    fn github_purl_falls_back_to_generic_without_url() {
        let e = entry("mytool", "1.0.0", "github", None);
        assert_eq!(purl_for_entry(&e), "pkg:generic/mytool@1.0.0");
    }

    #[test]
    fn apt_purl() {
        let e = entry("libssl-dev", "3.0.2", "apt", None);
        assert_eq!(purl_for_entry(&e), "pkg:deb/debian/libssl-dev@3.0.2");
    }

    #[test]
    fn dnf_purl_uses_generic_and_strips_dist_tag() {
        let e = entry("curl", "8.15.0-5.fc43", "dnf", None);
        assert_eq!(purl_for_entry(&e), "pkg:generic/curl@8.15.0");
    }

    #[test]
    fn dnf_purl_bare_version_unchanged() {
        let e = entry("openssl-devel", "3.0.7", "dnf", None);
        assert_eq!(purl_for_entry(&e), "pkg:generic/openssl-devel@3.0.7");
    }

    #[test]
    fn strip_rpm_release_strips_dist_tag() {
        assert_eq!(strip_rpm_release("8.15.0-5.fc43"), "8.15.0");
        assert_eq!(strip_rpm_release("14.1.1-3.fc43"), "14.1.1");
        assert_eq!(strip_rpm_release("9.1.2114-1.fc43"), "9.1.2114");
        assert_eq!(strip_rpm_release("3.0.7-2.el9"), "3.0.7");
    }

    #[test]
    fn strip_rpm_release_leaves_bare_version_alone() {
        assert_eq!(strip_rpm_release("8.15.0"), "8.15.0");
        assert_eq!(strip_rpm_release("1.7.1"), "1.7.1");
    }

    #[test]
    fn strip_rpm_release_leaves_non_digit_release_alone() {
        // upstream versions like "1.0-beta" should not be stripped
        assert_eq!(strip_rpm_release("1.0-beta"), "1.0-beta");
    }

    #[test]
    fn url_source_uses_generic_purl() {
        let e = entry(
            "mytool",
            "2.0.0",
            "url",
            Some("https://example.com/mytool.tar.gz"),
        );
        assert_eq!(purl_for_entry(&e), "pkg:generic/mytool@2.0.0");
    }

    #[test]
    fn parse_github_repo_extracts_owner_and_repo() {
        let (owner, repo) = parse_github_repo(
            "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux-amd64",
        )
        .unwrap();
        assert_eq!(owner, "jqlang");
        assert_eq!(repo, "jq");
    }

    #[test]
    fn parse_github_repo_returns_none_for_non_github_url() {
        assert!(parse_github_repo("https://example.com/foo/bar").is_none());
    }

    #[test]
    fn parse_github_repo_returns_none_for_empty_segments() {
        assert!(parse_github_repo("https://github.com/").is_none());
    }
}
