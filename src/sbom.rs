//! `grip sbom` — generate a Software Bill of Materials from `grip.lock`.
//!
//! Supports CycloneDX 1.5 JSON (default) and SPDX 2.3 JSON.
//! No network access required — reads only `grip.lock`.

use std::io::BufWriter;
use std::path::PathBuf;

use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::config::lockfile::{LockEntry, LockFile};
use crate::config::manifest::find_manifest_dir;
use crate::error::GripError;
use crate::purl;

// ── Public API ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbomFormat {
    CycloneDx,
    Spdx,
}

pub struct SbomOptions {
    pub format: SbomFormat,
    /// `None` → write to stdout.
    pub output: Option<PathBuf>,
}

pub fn run_sbom(root: Option<PathBuf>, opts: SbomOptions) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).unwrap_or(cwd)
        }
    };

    let lock = LockFile::load(&project_root.join("grip.lock"))?;

    let all_entries: Vec<&LockEntry> = lock
        .entries
        .iter()
        .chain(lock.library_entries.iter())
        .collect();

    let doc = match opts.format {
        SbomFormat::CycloneDx => build_cyclonedx(&all_entries),
        SbomFormat::Spdx => build_spdx(&all_entries),
    };

    match opts.output {
        Some(path) => {
            let file = std::fs::File::create(&path)?;
            serde_json::to_writer_pretty(BufWriter::new(file), &doc)
                .map_err(|e| GripError::Other(e.to_string()))?;
        }
        None => {
            let stdout = std::io::stdout();
            serde_json::to_writer_pretty(BufWriter::new(stdout.lock()), &doc)
                .map_err(|e| GripError::Other(e.to_string()))?;
            println!(); // trailing newline
        }
    }

    Ok(())
}

// ── CycloneDX 1.5 ─────────────────────────────────────────────────────────────

fn build_cyclonedx(entries: &[&LockEntry]) -> Value {
    let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let serial = format!("urn:uuid:{}", deterministic_uuid(entries));

    let components: Vec<Value> = entries
        .iter()
        .map(|e| {
            let version = e.version.trim_start_matches('v');
            let mut component = json!({
                "type": "application",
                "name": e.name,
                "version": version,
                "purl": purl::purl_for_entry(e),
            });
            if let Some(sha) = &e.sha256 {
                component["hashes"] =
                    json!([{ "alg": "SHA-256", "content": sha }]);
            }
            if let Some(url) = &e.url {
                component["externalReferences"] =
                    json!([{ "type": "distribution", "url": url }]);
            }
            component
        })
        .collect();

    json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": serial,
        "version": 1,
        "metadata": {
            "timestamp": timestamp,
            "tools": [{
                "vendor": "grip",
                "name": "grip",
                "version": env!("CARGO_PKG_VERSION")
            }]
        },
        "components": components
    })
}

// ── SPDX 2.3 ──────────────────────────────────────────────────────────────────

fn build_spdx(entries: &[&LockEntry]) -> Value {
    let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let grip_version = env!("CARGO_PKG_VERSION");

    let mut packages: Vec<Value> = Vec::new();
    let mut relationships: Vec<Value> = Vec::new();

    for e in entries {
        let spdx_id = spdx_ref(&e.name, &e.version);
        let download_loc = e.url.as_deref().unwrap_or("NOASSERTION");

        let mut pkg = json!({
            "SPDXID": spdx_id,
            "name": e.name,
            "versionInfo": e.version,
            "downloadLocation": download_loc,
            "filesAnalyzed": false,
            "externalRefs": [{
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": purl::purl_for_entry(e)
            }]
        });

        if let Some(sha) = &e.sha256 {
            pkg["checksums"] = json!([{ "algorithm": "SHA256", "checksumValue": sha }]);
        }

        packages.push(pkg);
        relationships.push(json!({
            "spdxElementId": "SPDXRef-DOCUMENT",
            "relationshipType": "DESCRIBES",
            "relatedSpdxElement": spdx_id
        }));
    }

    json!({
        "SPDXID": "SPDXRef-DOCUMENT",
        "spdxVersion": "SPDX-2.3",
        "creationInfo": {
            "created": timestamp,
            "creators": [format!("Tool: grip-{grip_version}")]
        },
        "name": "grip-sbom",
        "dataLicense": "CC0-1.0",
        "documentNamespace": format!("https://grip.dev/sbom/{timestamp}"),
        "packages": packages,
        "relationships": relationships
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Sanitize `name` + `version` into a valid `SPDXRef-*` identifier.
fn spdx_ref(name: &str, version: &str) -> String {
    let sanitize = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect()
    };
    format!("SPDXRef-{}-{}", sanitize(name), sanitize(version))
}

/// Generate a deterministic UUID-like string from the sorted set of entry names.
/// Uses the first 16 bytes of SHA-256, formatted as a UUID v4 variant.
/// No `uuid` crate needed — `sha2` is already a dependency.
fn deterministic_uuid(entries: &[&LockEntry]) -> String {
    let mut hasher = Sha256::new();
    let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    for name in &names {
        hasher.update(name.as_bytes());
        hasher.update(b"|");
    }
    let b = hasher.finalize();
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3],
        b[4], b[5],
        b[6], b[7],
        b[8], b[9],
        b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::lockfile::LockEntry;
    use chrono::Utc;

    fn entry(name: &str, version: &str, source: &str, url: Option<&str>, sha256: Option<&str>) -> LockEntry {
        LockEntry {
            name: name.to_string(),
            version: version.to_string(),
            source: source.to_string(),
            url: url.map(|s| s.to_string()),
            sha256: sha256.map(|s| s.to_string()),
            installed_at: Utc::now(),
            extra_binaries: vec![],
            auto_binary: None,
            auto_extra_binaries: vec![],
        }
    }

    // ── CycloneDX ─────────────────────────────────────────────────────────────

    #[test]
    fn cyclonedx_has_required_top_level_fields() {
        let e = entry("jq", "1.7.1", "github",
            Some("https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux"),
            Some("abc123"));
        let doc = build_cyclonedx(&[&e]);
        assert_eq!(doc["bomFormat"], "CycloneDX");
        assert_eq!(doc["specVersion"], "1.5");
        assert!(doc["serialNumber"].as_str().unwrap().starts_with("urn:uuid:"));
        assert_eq!(doc["version"], 1);
    }

    #[test]
    fn cyclonedx_component_has_purl_and_hash() {
        let e = entry("jq", "1.7.1", "github",
            Some("https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux"),
            Some("deadbeef"));
        let doc = build_cyclonedx(&[&e]);
        let comp = &doc["components"][0];
        assert_eq!(comp["name"], "jq");
        assert_eq!(comp["version"], "1.7.1");
        assert!(comp["purl"].as_str().unwrap().contains("jqlang/jq"));
        assert_eq!(comp["hashes"][0]["alg"], "SHA-256");
        assert_eq!(comp["hashes"][0]["content"], "deadbeef");
    }

    #[test]
    fn cyclonedx_omits_hashes_when_no_sha256() {
        let e = entry("ripgrep", "14.1.0", "apt", None, None);
        let doc = build_cyclonedx(&[&e]);
        assert!(doc["components"][0].get("hashes").is_none());
    }

    #[test]
    fn cyclonedx_omits_external_refs_when_no_url() {
        let e = entry("ripgrep", "14.1.0", "apt", None, None);
        let doc = build_cyclonedx(&[&e]);
        assert!(doc["components"][0].get("externalReferences").is_none());
    }

    #[test]
    fn cyclonedx_strips_v_prefix_from_version() {
        let e = entry("rg", "v14.1.1", "apt", None, None);
        let doc = build_cyclonedx(&[&e]);
        assert_eq!(doc["components"][0]["version"], "14.1.1");
    }

    // ── SPDX ─────────────────────────────────────────────────────────────────

    #[test]
    fn spdx_has_required_top_level_fields() {
        let e = entry("jq", "1.7.1", "github",
            Some("https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux"),
            None);
        let doc = build_spdx(&[&e]);
        assert_eq!(doc["spdxVersion"], "SPDX-2.3");
        assert_eq!(doc["dataLicense"], "CC0-1.0");
        assert!(doc["documentNamespace"].as_str().unwrap().starts_with("https://grip.dev/sbom/"));
    }

    #[test]
    fn spdx_package_has_purl_external_ref() {
        let e = entry("jq", "1.7.1", "github",
            Some("https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-linux"),
            None);
        let doc = build_spdx(&[&e]);
        let pkg = &doc["packages"][0];
        assert_eq!(pkg["name"], "jq");
        assert_eq!(pkg["versionInfo"], "1.7.1");
        let ext_ref = &pkg["externalRefs"][0];
        assert_eq!(ext_ref["referenceCategory"], "PACKAGE-MANAGER");
        assert_eq!(ext_ref["referenceType"], "purl");
        assert!(ext_ref["referenceLocator"].as_str().unwrap().contains("jqlang/jq"));
    }

    #[test]
    fn spdx_no_url_sets_noassertion() {
        let e = entry("ripgrep", "14.1.0", "apt", None, None);
        let doc = build_spdx(&[&e]);
        assert_eq!(doc["packages"][0]["downloadLocation"], "NOASSERTION");
    }

    #[test]
    fn spdx_describes_relationship_present() {
        let e = entry("jq", "1.7.1", "github", None, None);
        let doc = build_spdx(&[&e]);
        let rel = &doc["relationships"][0];
        assert_eq!(rel["spdxElementId"], "SPDXRef-DOCUMENT");
        assert_eq!(rel["relationshipType"], "DESCRIBES");
    }

    #[test]
    fn spdx_omits_checksums_when_no_sha256() {
        let e = entry("ripgrep", "14.1.0", "apt", None, None);
        let doc = build_spdx(&[&e]);
        assert!(doc["packages"][0].get("checksums").is_none());
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    #[test]
    fn spdx_ref_sanitizes_dots_and_colons() {
        let id = spdx_ref("libssl-dev", "3.0.2+dfsg");
        assert!(!id.contains('.'));
        assert!(!id.contains('+'));
        assert!(id.starts_with("SPDXRef-"));
    }

    #[test]
    fn deterministic_uuid_is_stable() {
        let e = entry("jq", "1.7.1", "github", None, None);
        let a = deterministic_uuid(&[&e]);
        let b = deterministic_uuid(&[&e]);
        assert_eq!(a, b);
    }

    #[test]
    fn deterministic_uuid_differs_for_different_entries() {
        let e1 = entry("jq", "1.7.1", "github", None, None);
        let e2 = entry("rg", "14.1.1", "github", None, None);
        assert_ne!(deterministic_uuid(&[&e1]), deterministic_uuid(&[&e2]));
    }
}
