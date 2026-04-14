//! `grip audit` — cross-reference installed tools against the OSV vulnerability database.
//!
//! Sends a single purl-based batch query to <https://api.osv.dev/v1/querybatch>.
//! Exits non-zero when vulnerabilities are found (suitable for CI).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::lockfile::{LockEntry, LockFile};
use crate::config::manifest::find_manifest_dir;
use crate::error::GripError;
use crate::output;
use crate::purl;

// ── Public API ────────────────────────────────────────────────────────────────

pub struct AuditOptions {
    /// Exit non-zero if any vulnerabilities are found (default: `true`).
    pub fail: bool,
    pub root: Option<PathBuf>,
    pub quiet: bool,
    pub color: bool,
}

const OSV_BATCH_URL: &str = "https://api.osv.dev/v1/querybatch";

pub async fn run_audit(opts: AuditOptions) -> Result<(), GripError> {
    let project_root = match opts.root {
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

    if all_entries.is_empty() {
        if !opts.quiet {
            println!(
                "\n  {}\n",
                output::dim(opts.color, "No entries in grip.lock to audit.")
            );
        }
        return Ok(());
    }

    if !opts.quiet {
        println!(
            "  {} Querying OSV for {} installed {}…",
            output::dim(opts.color, "·"),
            all_entries.len(),
            if all_entries.len() == 1 { "tool" } else { "tools" }
        );
    }

    let queries: Vec<OsvQuery> = all_entries
        .iter()
        .map(|e| OsvQuery {
            package: OsvPackage {
                purl: purl::purl_for_entry(e),
            },
        })
        .collect();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("grip/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let response: OsvBatchResponse = client
        .post(OSV_BATCH_URL)
        .json(&OsvBatchRequest { queries })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // Zip entries with parallel results array.
    let findings: Vec<(&LockEntry, &[OsvVuln])> = all_entries
        .iter()
        .zip(response.results.iter())
        .filter_map(|(entry, result)| {
            if result.vulns.is_empty() {
                None
            } else {
                Some((*entry, result.vulns.as_slice()))
            }
        })
        .collect();

    if !opts.quiet {
        print_findings(&findings, all_entries.len(), opts.color);
    }

    if opts.fail && !findings.is_empty() {
        if !opts.quiet {
            eprintln!(
                "error: {} vulnerable tool{} found",
                findings.len(),
                if findings.len() == 1 { "" } else { "s" }
            );
        }
        std::process::exit(1);
    }

    Ok(())
}

// ── Output ────────────────────────────────────────────────────────────────────

fn print_findings(findings: &[(&LockEntry, &[OsvVuln])], total: usize, color: bool) {
    println!();
    if findings.is_empty() {
        println!(
            "  {}",
            output::dim(
                color,
                &format!("No known vulnerabilities found in {total} installed tools.")
            )
        );
        println!();
        return;
    }

    println!(
        "  {} {} vulnerability {} found\n",
        output::fail_glyph(color),
        findings.len(),
        if findings.len() == 1 { "finding" } else { "findings" }
    );

    for (entry, vulns) in findings {
        println!(
            "  {}  {}  {}",
            output::red(color, "✗"),
            entry.name,
            output::dim(color, &entry.version)
        );
        for vuln in *vulns {
            let cve = vuln
                .aliases
                .as_ref()
                .and_then(|a| a.iter().find(|id| id.starts_with("CVE-")))
                .map(|s| s.as_str())
                .unwrap_or("");
            let summary = vuln.summary.as_deref().unwrap_or("(no summary available)");
            if cve.is_empty() {
                println!("     {}  {}", output::dim(color, &vuln.id), summary);
            } else {
                println!(
                    "     {}  {}  {}",
                    output::dim(color, &vuln.id),
                    output::yellow(color, cve),
                    summary
                );
            }
        }
        println!();
    }

    println!(
        "  {}",
        output::dim(color, "Run `grip update <name>` to upgrade a vulnerable tool.")
    );
    println!();
}

// ── OSV API types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OsvBatchRequest {
    queries: Vec<OsvQuery>,
}

#[derive(Serialize)]
struct OsvQuery {
    package: OsvPackage,
}

#[derive(Serialize)]
struct OsvPackage {
    purl: String,
}

#[derive(Deserialize)]
struct OsvBatchResponse {
    #[serde(default)]
    results: Vec<OsvResult>,
}

#[derive(Deserialize)]
struct OsvResult {
    #[serde(default)]
    vulns: Vec<OsvVuln>,
}

#[derive(Deserialize, Clone)]
struct OsvVuln {
    id: String,
    summary: Option<String>,
    aliases: Option<Vec<String>>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osv_batch_request_serializes_purl() {
        let req = OsvBatchRequest {
            queries: vec![OsvQuery {
                package: OsvPackage {
                    purl: "pkg:github/jqlang/jq@1.7.1".to_string(),
                },
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("pkg:github/jqlang/jq@1.7.1"));
        assert!(json.contains("\"purl\""));
    }

    #[test]
    fn osv_batch_response_deserializes_empty_results() {
        let json = r#"{"results": []}"#;
        let resp: OsvBatchResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results.is_empty());
    }

    #[test]
    fn osv_batch_response_deserializes_vulns() {
        let json = r#"{
            "results": [
                {
                    "vulns": [
                        {
                            "id": "GHSA-1234-5678-9012",
                            "summary": "A critical vulnerability",
                            "aliases": ["CVE-2023-12345"]
                        }
                    ]
                }
            ]
        }"#;
        let resp: OsvBatchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        let vuln = &resp.results[0].vulns[0];
        assert_eq!(vuln.id, "GHSA-1234-5678-9012");
        assert_eq!(vuln.summary.as_deref(), Some("A critical vulnerability"));
        assert_eq!(
            vuln.aliases.as_ref().unwrap()[0],
            "CVE-2023-12345"
        );
    }

    #[test]
    fn osv_result_missing_vulns_field_defaults_to_empty() {
        let json = r#"{"results": [{}]}"#;
        let resp: OsvBatchResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results[0].vulns.is_empty());
    }

    #[test]
    fn osv_vuln_missing_optional_fields_deserializes() {
        let json = r#"{"results": [{"vulns": [{"id": "GHSA-xxxx"}]}]}"#;
        let resp: OsvBatchResponse = serde_json::from_str(json).unwrap();
        let vuln = &resp.results[0].vulns[0];
        assert_eq!(vuln.id, "GHSA-xxxx");
        assert!(vuln.summary.is_none());
        assert!(vuln.aliases.is_none());
    }
}
