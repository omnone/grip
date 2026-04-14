//! `grip suggest` — discover unmanaged CLI tools from shell history, project
//! scripts, CI YAML, and source code.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::config::manifest::{find_manifest_dir, Manifest};
use crate::error::GripError;
use crate::output;

// ── Public API ────────────────────────────────────────────────────────────────

/// Options for `grip suggest`.
pub struct SuggestOptions {
    /// Extra paths to scan for subprocess/exec calls in source code.
    pub scan_paths: Vec<PathBuf>,
    /// Scan shell history files (bash, zsh, fish). Default: true.
    pub history: bool,
    /// Also surface candidates not present in the curated known-tools list.
    pub show_unknown: bool,
    pub quiet: bool,
    pub color: bool,
}

/// Main entry point for `grip suggest`.
pub fn run_suggest(root: Option<PathBuf>, opts: SuggestOptions) -> Result<(), GripError> {
    let project_root = match root {
        Some(r) => r,
        None => {
            let cwd = std::env::current_dir()?;
            find_manifest_dir(&cwd).unwrap_or(cwd)
        }
    };

    // Tools already declared in grip.toml — skip them.
    let manifest_path = project_root.join("grip.toml");
    let already_declared: HashSet<String> = if manifest_path.exists() {
        let m = Manifest::load(&manifest_path)?;
        m.binaries
            .keys()
            .cloned()
            .chain(m.libraries.keys().cloned())
            .collect()
    } else {
        HashSet::new()
    };

    let builtins = system_builtins();
    let known = known_tools();

    // name → list of (source_label, count)
    let mut raw: HashMap<String, Vec<(String, usize)>> = HashMap::new();

    // 1. Shell history
    if opts.history {
        for (name, count) in scan_shell_history() {
            raw.entry(name).or_default().push(("shell history".to_string(), count));
        }
    }

    // 2. Project files: Makefile, scripts/, .github/workflows/
    for (name, label) in scan_project_files(&project_root) {
        accumulate(&mut raw, name, label);
    }

    // 3. Source-code paths provided by the user
    for (name, label) in scan_source_code(&opts.scan_paths) {
        accumulate(&mut raw, name, label);
    }

    // Build sorted candidates, dropping builtins and already-declared tools.
    let mut candidates: BTreeMap<
        String,
        (Option<(&'static str, &'static str)>, Vec<(String, usize)>),
    > = BTreeMap::new();

    for (name, occ) in raw {
        if name.len() < 2 || name.starts_with('-') {
            continue;
        }
        if builtins.contains(name.as_str()) {
            continue;
        }
        if already_declared.contains(&name) {
            continue;
        }
        let info = known.get(name.as_str()).copied();
        if info.is_none() && !opts.show_unknown {
            continue;
        }
        candidates.insert(name, (info, occ));
    }

    if !opts.quiet {
        print_suggestions(&candidates, opts.color);
    }

    Ok(())
}

// ── Output ────────────────────────────────────────────────────────────────────

fn print_suggestions(
    candidates: &BTreeMap<String, (Option<(&'static str, &'static str)>, Vec<(String, usize)>)>,
    color: bool,
) {
    println!();
    if candidates.is_empty() {
        println!(
            "  {}",
            output::dim(color, "No unmanaged tools found in the scanned sources.")
        );
        println!();
        return;
    }

    println!("  {}", output::dim(color, "Suggested additions to grip.toml"));
    println!();

    let n_known = candidates.values().filter(|(k, _)| k.is_some()).count();
    let n_unknown = candidates.len() - n_known;

    for (name, (info, occ)) in candidates {
        let found: Vec<String> = occ
            .iter()
            .map(|(lbl, cnt)| {
                if *cnt > 1 {
                    format!("{lbl} ({cnt}×)")
                } else {
                    lbl.clone()
                }
            })
            .collect();
        let found_str = found.join(", ");

        match info {
            Some((repo, desc)) => {
                let bullet = output::green(color, "✦");
                println!("  {bullet}  {name:<20}  {repo}  —  {desc}");
            }
            None => {
                let bullet = output::yellow(color, "?");
                println!(
                    "  {bullet}  {name:<20}  {}",
                    output::dim(color, "(no known GitHub source)")
                );
            }
        }
        println!(
            "     {:<20}  {}",
            "",
            output::dim(color, &format!("↳ found in: {found_str}"))
        );
        println!();
    }

    let sep = output::dim(color, &"─".repeat(60));
    println!("  {sep}");

    let mut parts: Vec<String> = Vec::new();
    if n_known > 0 {
        parts.push(format!(
            "{} with known sources",
            output::green(color, &n_known.to_string())
        ));
    }
    if n_unknown > 0 {
        parts.push(format!(
            "{} unknown",
            output::yellow(color, &n_unknown.to_string())
        ));
    }
    if !parts.is_empty() {
        println!("  {}", parts.join(", "));
    }
    println!(
        "  {}",
        output::dim(
            color,
            "Run `grip add <name> --source github` to add, or `grip add <name>` for system packages."
        )
    );
    println!();
}

// ── Shell history ─────────────────────────────────────────────────────────────

/// Scan bash, zsh, and fish history files.
/// Returns tool_name → occurrence count.
fn scan_shell_history() -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return counts,
    };
    let paths = [
        home.join(".bash_history"),
        home.join(".zsh_history"),
        home.join(".local/share/fish/fish_history"),
    ];
    for path in &paths {
        if !path.exists() {
            continue;
        }
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().flatten() {
                if let Some(name) = extract_command_name(&line) {
                    *counts.entry(name).or_default() += 1;
                }
            }
        }
    }
    counts
}

// ── Project file scanning ─────────────────────────────────────────────────────

/// Scan Makefile, scripts/, and .github/workflows/ relative to `root`.
fn scan_project_files(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();

    for name in &["Makefile", "makefile", "GNUmakefile"] {
        let p = root.join(name);
        if p.is_file() {
            scan_makefile(&p, root, &mut out);
        }
    }

    for dir_name in &["scripts", "script", "bin", "hack", "tools"] {
        let d = root.join(dir_name);
        if d.is_dir() {
            scan_shell_dir(&d, root, &mut out);
        }
    }

    let workflows = root.join(".github/workflows");
    if workflows.is_dir() {
        scan_ci_yaml_dir(&workflows, root, &mut out);
    }

    out
}

fn rel_label(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn scan_makefile(path: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let label = rel_label(path, root);
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        if !line.starts_with('\t') {
            continue;
        }
        // Strip make-specific @ (silent) and - (ignore-error) prefixes.
        let cmd = line.trim().trim_start_matches(|c| c == '@' || c == '-');
        if let Some(name) = extract_command_name(cmd) {
            out.push((name, label.clone()));
        }
    }
}

fn scan_shell_dir(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "sh" | "bash" | "zsh") || has_shell_shebang(&p) {
            scan_shell_file(&p, root, out);
        }
    }
}

fn has_shell_shebang(path: &Path) -> bool {
    let Ok(f) = fs::File::open(path) else {
        return false;
    };
    let mut r = BufReader::new(f);
    let mut first = String::new();
    let _ = r.read_line(&mut first);
    first.starts_with("#!")
        && (first.contains("/sh")
            || first.contains("/bash")
            || first.contains("/zsh")
            || first.contains("env sh")
            || first.contains("env bash")
            || first.contains("env zsh"))
}

fn scan_shell_file(path: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let label = rel_label(path, root);
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        if let Some(name) = extract_command_name(line) {
            out.push((name, label.clone()));
        }
    }
}

fn scan_ci_yaml_dir(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "yml" | "yaml") {
            scan_ci_yaml(&p, root, out);
        }
    }
}

fn scan_ci_yaml(path: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let label = rel_label(path, root);
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let mut in_run = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("run:") {
            in_run = true;
            let after = trimmed.strip_prefix("run:").unwrap_or("").trim();
            let after = after.trim_matches('"').trim_matches('\'');
            if !after.is_empty() && after != "|" && after != "|-" {
                if let Some(name) = extract_command_name(after) {
                    out.push((name, label.clone()));
                }
            }
            continue;
        }
        if in_run {
            if line.starts_with(' ') || line.starts_with('\t') {
                if let Some(name) = extract_command_name(trimmed) {
                    out.push((name, label.clone()));
                }
            } else if !trimmed.is_empty() {
                in_run = false;
            }
        }
    }
}

// ── Source-code scanning ──────────────────────────────────────────────────────

/// Scan source files under the given paths for subprocess / exec calls and
/// binary path literals.
fn scan_source_code(paths: &[PathBuf]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in paths {
        // When a single file is passed, use its parent as the base so the
        // label shows the filename rather than an empty string.
        let base = if p.is_file() {
            p.parent()
                .map(|parent| parent.to_path_buf())
                .unwrap_or_else(|| p.clone())
        } else {
            p.clone()
        };
        walk_source(p, &base, &mut out);
    }
    out
}

fn walk_source(path: &Path, base: &Path, out: &mut Vec<(String, String)>) {
    if path.is_file() {
        scan_source_file(path, base, out);
    } else if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            let fname = child.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip hidden dirs and known build artifact / dependency directories.
            if fname.starts_with('.')
                || matches!(
                    fname,
                    "target"
                        | "node_modules"
                        | "vendor"
                        | "__pycache__"
                        | "dist"
                        | "build"
                        | ".git"
                )
            {
                continue;
            }
            walk_source(&child, base, out);
        }
    }
}

// Language-specific subprocess / exec patterns.
// After matching the prefix the scanner reads until the next `"` for the argument.
const RUST_PATTERNS: &[&str] = &["Command::new(\"", "command::new(\""];

const PYTHON_PATTERNS: &[&str] = &[
    // List-form: subprocess.run(["tool", ...])
    "subprocess.run([\"",
    "subprocess.call([\"",
    "subprocess.check_output([\"",
    "subprocess.Popen([\"",
    // Direct import: Popen(["tool", ...])
    "Popen([\"",
    // String-form: os.system("tool arg1 arg2") – we take the first token
    "os.system(\"",
    "os.popen(\"",
    "shutil.which(\"",
];

const JS_PATTERNS: &[&str] = &[
    "exec(\"",
    "execSync(\"",
    "spawn(\"",
    "spawnSync(\"",
    "execa(\"",
    "execFile(\"",
    "execFileSync(\"",
];

const GO_PATTERNS: &[&str] = &["exec.Command(\"", "exec.LookPath(\""];

const RUBY_PATTERNS: &[&str] = &[
    "system(\"",
    "IO.popen(\"",
    "Open3.capture3(\"",
    "Open3.popen3(\"",
];

fn scan_source_file(path: &Path, base: &Path, out: &mut Vec<(String, String)>) {
    // Compute a stable, non-empty label relative to `base`.
    let rel = path.strip_prefix(base).unwrap_or(path);
    let label: String = if rel.as_os_str().is_empty() {
        // path == base (single file passed directly)
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string())
    } else {
        rel.to_string_lossy().to_string()
    };

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    let is_shell = matches!(ext, "sh" | "bash" | "zsh");
    let is_lang = matches!(
        ext,
        "rs" | "py" | "js" | "ts" | "mjs" | "cjs" | "jsx" | "tsx" | "go" | "rb"
    );
    // Files where we only run the binary-path scanner (no language API patterns).
    let is_paths_only = matches!(
        ext,
        "yml" | "yaml" | "toml" | "json" | "mk" | "dockerfile" | "conf" | "cfg" | "ini"
    ) || (ext.is_empty()
        && matches!(
            fname,
            "Dockerfile" | "Makefile" | "makefile" | "GNUmakefile" | "Justfile"
        ));

    if !is_shell && !is_lang && !is_paths_only {
        return;
    }

    let Ok(content) = fs::read_to_string(path) else {
        return;
    };

    // Per-file dedup: avoid double-counting the same tool from overlapping
    // patterns (e.g. `subprocess.Popen` matching both `subprocess.Popen(["` and
    // `Popen(["`).
    let mut seen: HashSet<String> = HashSet::new();

    // ── Shell scripts: line-by-line command extraction ────────────────────────
    if is_shell {
        for line in content.lines() {
            if let Some(name) = extract_command_name(line) {
                if seen.insert(name.clone()) {
                    out.push((name, label.clone()));
                }
            }
        }
        // Also catch full-path invocations in shell scripts.
        for name in scan_binary_paths(&content) {
            let name = name.to_string();
            if seen.insert(name.clone()) {
                out.push((name, label.clone()));
            }
        }
        return;
    }

    // ── Language-specific subprocess / exec patterns ──────────────────────────
    if is_lang {
        let patterns: &[&str] = match ext {
            "rs" => RUST_PATTERNS,
            "py" => PYTHON_PATTERNS,
            "js" | "ts" | "mjs" | "cjs" | "jsx" | "tsx" => JS_PATTERNS,
            "go" => GO_PATTERNS,
            "rb" => RUBY_PATTERNS,
            _ => &[],
        };

        for pattern in patterns {
            let skip = pattern.len();
            let mut pos = 0;
            while pos < content.len() {
                let Some(found) = content[pos..].find(pattern) else {
                    break;
                };
                let start = pos + found + skip;
                if start < content.len() {
                    if let Some(end) = content[start..].find('"') {
                        let raw = &content[start..start + end];
                        // Take only the first whitespace-delimited token so that
                        // os.system("kubectl get pods") → "kubectl" (not the full string).
                        let first = raw.split_whitespace().next().unwrap_or(raw);
                        // Strip any path prefix: /usr/bin/kubectl → kubectl.
                        let name = first.rsplit('/').next().unwrap_or(first);
                        if is_valid_tool_name(name) && seen.insert(name.to_string()) {
                            out.push((name.to_string(), label.clone()));
                        }
                    }
                }
                pos += found + pattern.len();
            }
        }
    }

    // ── Universal binary-path scan ────────────────────────────────────────────
    // Runs for all supported file types (language files, shell scripts already
    // returned above, and paths-only files like Dockerfile / YAML).
    // Catches literals like /usr/local/bin/kubectl → "kubectl".
    for name in scan_binary_paths(&content) {
        let name = name.to_string();
        if seen.insert(name.clone()) {
            out.push((name, label.clone()));
        }
    }
}

/// Find absolute binary-path references (`/usr/bin/jq`, `/usr/local/bin/kubectl`, …).
/// Returns a slice of tool-name `&str`s borrowed from `content`.
fn scan_binary_paths(content: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut pos = 0;
    while let Some(found) = content[pos..].find("/bin/") {
        let name_start = pos + found + 5; // skip "/bin/"
        if name_start < content.len() {
            let rest = &content[name_start..];
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if is_valid_tool_name(name) {
                names.push(name);
            }
        }
        pos += found + 5;
    }
    names
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Aggregate (name, label) findings into the raw map.
fn accumulate(raw: &mut HashMap<String, Vec<(String, usize)>>, name: String, label: String) {
    let entry = raw.entry(name).or_default();
    if let Some(slot) = entry.iter_mut().find(|(l, _)| *l == label) {
        slot.1 += 1;
    } else {
        entry.push((label, 1));
    }
}

/// Extract the leading command name from a shell-like line.
/// Returns `None` for blank lines, comments, variable assignments, etc.
fn extract_command_name(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    // zsh extended-history format: ": 1234567890:0;actual command"
    let line = if line.starts_with(": ") && line.contains(';') {
        line.splitn(2, ';').nth(1)?.trim()
    } else {
        line
    };
    // fish history format: "- cmd: actual command"
    let line = if let Some(rest) = line.strip_prefix("- cmd:") {
        rest.trim()
    } else {
        line
    };
    // fish "when:" metadata lines
    if line.starts_with("when:") {
        return None;
    }

    let token = line.split_whitespace().next()?;
    // Strip any path prefix so "./bin/jq" → "jq"
    let token = token.rsplit('/').next().unwrap_or(token);

    // Skip shell metacharacters, assignments, redirections, etc.
    if token.starts_with('$')
        || token.starts_with('-')
        || token.starts_with('!')
        || token.starts_with('"')
        || token.starts_with('\'')
        || token.starts_with('(')
        || token.starts_with('{')
        || token.starts_with('[')
        || token.starts_with('`')
        || token.contains('=')
        || token.contains('>')
        || token.contains('<')
        || token.ends_with(':') // YAML keys
    {
        return None;
    }

    if !is_valid_tool_name(token) {
        return None;
    }

    Some(token.to_lowercase())
}

fn is_valid_tool_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 64
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
}

// ── Curated knowledge base ────────────────────────────────────────────────────

/// Known tools: (command_name, github_repo, short_description)
static KNOWN_TOOLS: &[(&str, &str, &str)] = &[
    // JSON / data
    ("jq",            "jqlang/jq",                   "JSON processor"),
    ("yq",            "mikefarah/yq",                "YAML/JSON/TOML processor"),
    ("gron",          "tomnomnom/gron",               "Flatten JSON to greppable lines"),
    ("dasel",         "TomWright/dasel",              "Query/update JSON, YAML, TOML, CSV"),
    // Search
    ("rg",            "BurntSushi/ripgrep",           "Fast grep alternative"),
    ("fd",            "sharkdp/fd",                   "Fast find alternative"),
    ("fzf",           "junegunn/fzf",                 "Fuzzy finder"),
    ("ag",            "ggreer/the_silver_searcher",   "Fast code search"),
    // File tools
    ("bat",           "sharkdp/bat",                  "cat with syntax highlighting"),
    ("delta",         "dandavison/delta",              "Better git diff"),
    ("difft",         "Wilfred/difftastic",            "Structural diff"),
    ("eza",           "eza-community/eza",             "Modern ls replacement"),
    ("exa",           "ogham/exa",                     "Modern ls replacement (legacy)"),
    ("lsd",           "lsd-rs/lsd",                    "ls with file icons"),
    ("zoxide",        "ajeetdsouza/zoxide",            "Smarter cd command"),
    ("dust",          "bootandy/dust",                 "Intuitive du"),
    ("duf",           "muesli/duf",                    "Better df"),
    ("gdu",           "dundee/gdu",                    "Disk usage analyser"),
    // HTTP / network
    ("xh",            "ducaale/xh",                    "Friendly HTTP client"),
    ("hurl",          "Orange-OpenSource/hurl",        "HTTP requests from text files"),
    // System / process
    ("procs",         "dalance/procs",                 "Modern ps"),
    ("btm",           "ClementTsang/bottom",           "System resource monitor"),
    ("bandwhich",     "imsnif/bandwhich",              "Network usage by process"),
    // Git
    ("lazygit",       "jesseduffield/lazygit",         "Terminal UI for git"),
    ("gitui",         "extrawurst/gitui",              "Fast terminal git UI"),
    ("gh",            "cli/cli",                       "GitHub CLI"),
    ("hub",           "mislav/hub",                    "GitHub workflow CLI"),
    ("gitleaks",      "gitleaks/gitleaks",             "Scan for secrets in git repos"),
    ("trufflehog",    "trufflesecurity/trufflehog",    "Find leaked credentials"),
    // Dev tooling
    ("just",          "casey/just",                    "Command runner (make alternative)"),
    ("hyperfine",     "sharkdp/hyperfine",             "Command-line benchmarking"),
    ("tokei",         "XAMPPRocky/tokei",              "Count lines of code"),
    ("shellcheck",    "koalaman/shellcheck",           "Shell script linter"),
    ("shfmt",         "mvdan/sh",                      "Shell script formatter"),
    ("hadolint",      "hadolint/hadolint",             "Dockerfile linter"),
    // Kubernetes / cloud
    ("k9s",           "derailed/k9s",                  "Kubernetes TUI"),
    ("kubectl",       "kubernetes/kubectl",             "Kubernetes CLI"),
    ("helm",          "helm/helm",                     "Kubernetes package manager"),
    ("kustomize",     "kubernetes-sigs/kustomize",     "Kubernetes config customization"),
    ("flux",          "fluxcd/flux2",                  "GitOps for Kubernetes"),
    ("argocd",        "argoproj/argo-cd",              "GitOps CD tool"),
    ("eksctl",        "eksctl/eksctl",                 "Amazon EKS CLI"),
    // Infrastructure
    ("terraform",     "hashicorp/terraform",           "Infrastructure as code"),
    ("vault",         "hashicorp/vault",               "Secrets management"),
    ("packer",        "hashicorp/packer",              "Machine image builder"),
    ("pulumi",        "pulumi/pulumi",                 "Infrastructure as code"),
    ("act",           "nektos/act",                    "Run GitHub Actions locally"),
    // Security
    ("cosign",        "sigstore/cosign",               "Container image signing"),
    ("syft",          "anchore/syft",                  "SBOM generator"),
    ("grype",         "anchore/grype",                 "Vulnerability scanner"),
    ("trivy",         "aquasecurity/trivy",            "Security scanner"),
    ("mkcert",        "FiloSottile/mkcert",            "Local TLS certificates"),
    ("age",           "FiloSottile/age",               "Simple file encryption"),
    ("osv-scanner",   "google/osv-scanner",            "Open-source vulnerability scanner"),
    // Code quality
    ("golangci-lint", "golangci/golangci-lint",        "Go meta-linter"),
    ("tflint",        "terraform-linters/tflint",      "Terraform linter"),
    ("semgrep",       "semgrep/semgrep",               "Code analysis tool"),
    ("opa",           "open-policy-agent/opa",         "Open Policy Agent"),
    ("conftest",      "open-policy-agent/conftest",    "Policy testing for config files"),
    // gRPC / protobuf
    ("grpcurl",       "fullstorydev/grpcurl",          "cURL for gRPC"),
    ("buf",           "bufbuild/buf",                  "Protobuf toolchain"),
    // CI/CD
    ("goreleaser",    "goreleaser/goreleaser",         "Release automation"),
    ("earthly",       "earthly/earthly",              "Build automation"),
    ("dive",          "wagoodman/dive",                "Explore Docker image layers"),
    ("crane",         "google/go-containerregistry",   "Container registry CLI"),
    // Shell / env
    ("starship",      "starship-rs/starship",          "Cross-shell prompt"),
    ("direnv",        "direnv/direnv",                 "Per-directory env vars"),
    ("mise",          "jdx/mise",                      "Dev tools version manager"),
];

fn known_tools() -> HashMap<&'static str, (&'static str, &'static str)> {
    KNOWN_TOOLS.iter().map(|&(n, r, d)| (n, (r, d))).collect()
}

/// Standard Unix tools, shell builtins, and language runtimes — never suggest these.
fn system_builtins() -> HashSet<&'static str> {
    [
        // Shells
        "bash","sh","zsh","fish","dash","ksh","tcsh","csh","ash","nu","elvish",
        // Builtins
        "echo","printf","read","export","source","eval","exec","set","unset",
        "alias","unalias","return","exit","break","continue","shift","builtin",
        // File ops
        "ls","cat","cp","mv","rm","mkdir","rmdir","ln","chmod","chown","chgrp",
        "touch","stat","find","locate","file","basename","dirname","realpath","readlink",
        // Text processing
        "grep","egrep","fgrep","sed","awk","gawk","mawk","cut","sort","uniq",
        "tr","head","tail","wc","diff","patch","comm","join","paste","column",
        "fold","fmt","nl","pr","expand","unexpand","split","csplit","truncate",
        // Archives
        "tar","gzip","gunzip","bzip2","bunzip2","xz","unxz","zip","unzip","zstd",
        // Network
        "curl","wget","nc","netcat","ssh","scp","sftp","rsync","ftp","telnet",
        // VCS
        "git","svn","hg","cvs","fossil",
        // Build systems
        "make","cmake","ninja","meson","bazel","ant",
        // Python
        "python","python3","python2","pip","pip3","pipenv","poetry","virtualenv","conda","mamba",
        // Node.js
        "node","nodejs","npm","yarn","pnpm","npx","deno","bun","tsc",
        // Ruby
        "ruby","gem","bundle","bundler","rake",
        // Go
        "go","gofmt","gopls",
        // Rust
        "cargo","rustc","rustup","rustfmt",
        // JVM
        "javac","java","mvn","gradle","kotlin","kotlinc",
        // C/C++
        "clang","clang++","gcc","g++","cc","c++","ld","ar","nm","objdump","strip","ranlib","ldd",
        // Containers
        "docker","podman","buildah","containerd","runc","skopeo",
        // System utils
        "env","printenv","tee","xargs","parallel","seq","yes","tput",
        "date","cal","time","sleep","wait","timeout","watch",
        "kill","pkill","killall","ps","top","htop","pgrep","nice","renice","nohup",
        "id","whoami","groups","su","sudo","doas",
        "pwd","cd","pushd","popd","dirs",
        "true","false","test",":",".",
        "vim","vi","nano","emacs","code","nvim","neovim",
        "less","more","most",
        "man","info","help",
        "which","whereis","type","command","hash",
        "uname","hostname","hostnamectl","lsb_release",
        "mount","umount","df","du","free","lsblk",
        "ip","ifconfig","ping","traceroute","ss","netstat","dig","nslookup","host",
        "gpg","gpg2","openssl","ssh-keygen","ssh-agent","ssh-add",
        "cron","crontab","at",
        "strace","ltrace","gdb","lldb","valgrind","perf",
        "apt","apt-get","dpkg","yum","dnf","rpm","zypper","pacman","brew","snap","flatpak","nix",
        "systemctl","journalctl","service",
        "lsof","fuser","iostat","vmstat","sar",
        "cmp","md5sum","sha1sum","sha256sum","sha512sum","cksum",
        "bc","dc","expr","factor",
        "dd","pv",
        "tmux","screen",
        "w","who","last","uptime",
        "open","xdg-open","xclip","xsel","pbcopy","pbpaste",
        "dmesg","lsmod","modprobe",
        "mktemp",
        "iconv","locale",
        "grip", // don't suggest grip itself
        "pwsh","powershell","cmd",
        "nmap","ping6","arping","tracepath",
        "hexdump","xxd","od","strings",
        "diff3","colordiff",
        "mail","sendmail",
        "fc","history","jobs","bg","fg","wait",
    ]
    .iter()
    .cloned()
    .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_plain_command() {
        assert_eq!(extract_command_name("jq --arg x y"), Some("jq".into()));
    }

    #[test]
    fn extract_zsh_history_line() {
        assert_eq!(
            extract_command_name(": 1234567890:0;fd . --type f"),
            Some("fd".into())
        );
    }

    #[test]
    fn extract_fish_history_line() {
        assert_eq!(
            extract_command_name("- cmd: rg 'pattern' src/"),
            Some("rg".into())
        );
    }

    #[test]
    fn skip_comment() {
        assert!(extract_command_name("# this is a comment").is_none());
    }

    #[test]
    fn skip_variable_assignment() {
        assert!(extract_command_name("FOO=bar jq").is_none());
    }

    #[test]
    fn skip_blank_line() {
        assert!(extract_command_name("   ").is_none());
    }

    #[test]
    fn skip_yaml_key() {
        assert!(extract_command_name("run:").is_none());
        assert!(extract_command_name("steps:").is_none());
    }

    #[test]
    fn strip_path_prefix() {
        assert_eq!(extract_command_name("./bin/jq --help"), Some("jq".into()));
    }

    #[test]
    fn is_valid_tool_name_rejects_empty_and_numeric_start() {
        assert!(!is_valid_tool_name(""));
        assert!(!is_valid_tool_name("123tool"));
    }

    #[test]
    fn is_valid_tool_name_accepts_hyphen_and_underscore() {
        assert!(is_valid_tool_name("golangci-lint"));
        assert!(is_valid_tool_name("osv_scanner"));
    }

    #[test]
    fn known_tools_has_expected_entries() {
        let m = known_tools();
        assert!(m.contains_key("jq"));
        assert!(m.contains_key("rg"));
        assert!(m.contains_key("kubectl"));
    }

    #[test]
    fn system_builtins_excludes_tools() {
        let b = system_builtins();
        assert!(b.contains("grep"));
        assert!(b.contains("curl"));
        assert!(!b.contains("jq"));
        assert!(!b.contains("fd"));
    }
}
