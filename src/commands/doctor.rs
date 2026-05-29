//! `rosita doctor` — diagnose environment, config, and generated state.

use std::path::Path;
use std::process::Command;

use super::{prepare, Runtime};
use crate::render::header;
use crate::writer::BLOCK_BEGIN;
use crate::{config, templates};

#[derive(Clone, Copy)]
enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn symbol(self) -> &'static str {
        match self {
            Status::Ok => "✓",
            Status::Warn => "⚠",
            Status::Fail => "✗",
        }
    }
}

struct Checks {
    warns: usize,
    fails: usize,
}

impl Checks {
    fn new() -> Self {
        Checks { warns: 0, fails: 0 }
    }
    fn line(&mut self, status: Status, msg: impl AsRef<str>) {
        match status {
            Status::Warn => self.warns += 1,
            Status::Fail => self.fails += 1,
            Status::Ok => {}
        }
        println!("  {} {}", status.symbol(), msg.as_ref());
    }
}

/// Entry point for `rosita doctor`.
pub fn run(rt: &Runtime) -> crate::Result<()> {
    let mut c = Checks::new();

    println!("Environment");
    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => {
            c.line(
                Status::Ok,
                format!("git: {}", String::from_utf8_lossy(&o.stdout).trim()),
            );
        }
        _ => c.line(
            Status::Fail,
            "git not found on PATH (git detection disabled)",
        ),
    }
    // Config + context.
    println!("\nConfiguration");
    let prep = match prepare(rt) {
        Ok(p) => p,
        Err(e) => {
            c.line(
                Status::Fail,
                format!("failed to load config / detect context: {e:#}"),
            );
            print_summary(&c);
            return Ok(());
        }
    };
    if prep.config.sources.is_empty() {
        c.line(
            Status::Warn,
            "no config files found; using built-in defaults (run `rosita init`)",
        );
    } else {
        for s in &prep.config.sources {
            c.line(Status::Ok, format!("loaded config: {}", s.display()));
        }
    }
    // Allowlist/denylist consistency.
    check_env_policy(&mut c, &prep.config);
    // Private-data leak lint over public config layers.
    check_public_leaks(&mut c, &prep);

    // Agents + their launch CLIs.
    println!("\nAgents ({} configured)", prep.config.agents.len());
    for a in &prep.config.agents {
        match &a.launch {
            Some(prog) if on_path(prog) => {
                c.line(Status::Ok, format!("{}: CLI '{prog}' found", a.id))
            }
            Some(prog) => c.line(
                Status::Warn,
                format!(
                    "{}: CLI '{prog}' not on PATH (needed for `run {}`)",
                    a.id, a.id
                ),
            ),
            None => c.line(Status::Ok, format!("{}: render-only", a.id)),
        }
    }

    // Templates.
    println!("\nTemplates");
    match templates::resolve(&prep.repo_base, "overlay") {
        Ok(t) => c.line(Status::Ok, format!("overlay template ← {}", t.source)),
        Err(e) => c.line(Status::Fail, format!("overlay template: {e:#}")),
    }

    // Writability.
    println!("\nFilesystem");
    match writable(&prep.repo_base) {
        true => c.line(
            Status::Ok,
            format!("base dir is writable: {}", prep.repo_base.display()),
        ),
        false => c.line(
            Status::Fail,
            format!("base dir not writable: {}", prep.repo_base.display()),
        ),
    }
    if prep.context.git.is_some() {
        check_gitignore(&mut c, &prep.repo_base);
    } else {
        c.line(
            Status::Ok,
            "not a git repo — non-repo mode (.gitignore not managed)",
        );
    }
    check_claude_marker(&mut c, &prep.repo_base);

    // Generated overlays freshness.
    println!(
        "\nGenerated overlays (context {})",
        crate::hash::short(&prep.context.compute_hash())
    );
    check_overlays(&mut c, &prep);

    print_summary(&c);
    Ok(())
}

fn print_summary(c: &Checks) {
    println!();
    if c.fails > 0 {
        println!("doctor: {} failure(s), {} warning(s)", c.fails, c.warns);
    } else if c.warns > 0 {
        println!("doctor: healthy, {} warning(s)", c.warns);
    } else {
        println!("doctor: all good ✓");
    }
}

fn on_path(program: &str) -> bool {
    // `command -v` is portable across the shells we target.
    Command::new(program)
        .arg("--version")
        .output()
        .map(|o| o.status.success() || !o.stdout.is_empty())
        .unwrap_or(false)
}

fn writable(dir: &Path) -> bool {
    tempfile::Builder::new()
        .prefix(".rosita-doctor-")
        .tempfile_in(dir)
        .is_ok()
}

fn check_env_policy(c: &mut Checks, cfg: &config::Config) {
    let deny: Vec<regex::Regex> = cfg
        .env
        .deny_name_patterns
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect();
    let conflicting: Vec<&String> = cfg
        .env
        .allowlist
        .iter()
        .filter(|name| deny.iter().any(|re| re.is_match(name)))
        .collect();
    if conflicting.is_empty() {
        c.line(
            Status::Ok,
            format!(
                "env allowlist: {} name(s), denylist consistent",
                cfg.env.allowlist.len()
            ),
        );
    } else {
        c.line(
            Status::Warn,
            format!("env names allowlisted but denied (will be dropped): {conflicting:?}"),
        );
    }
}

/// Warn when a **public** config layer (`config.toml`) contains literals that
/// look machine-specific — IPv4 addresses, `*.domain.tld` globs, or
/// multi-label hostnames — which belong in the gitignored `local.toml`. Only
/// public layers are scanned; `local.toml` is the place for these.
fn check_public_leaks(c: &mut Checks, prep: &super::Prepared) {
    let patterns = leak_patterns();
    let mut scanned = 0usize;
    let mut flagged = 0usize;
    for src in &prep.config.sources {
        if src.file_name().and_then(|s| s.to_str()) != Some("config.toml") {
            continue; // local.toml is the private layer — never linted
        }
        let Ok(text) = std::fs::read_to_string(src) else {
            continue;
        };
        let Ok(value) = toml::from_str::<toml::Value>(&text) else {
            continue; // parse errors surface elsewhere
        };
        scanned += 1;
        let mut hits: Vec<String> = Vec::new();
        collect_leaky_strings(&value, &patterns, &mut hits);
        hits.sort();
        hits.dedup();
        for h in &hits {
            flagged += 1;
            c.line(
                Status::Warn,
                format!(
                    "{}: {h:?} looks private — move to local.toml",
                    src.display()
                ),
            );
        }
    }
    if scanned > 0 && flagged == 0 {
        c.line(Status::Ok, "public config has no private-looking literals");
    }
}

/// Regexes for machine-specific literals (compiled once per call). Patterns are
/// static and valid, so `unwrap` is sound.
fn leak_patterns() -> Vec<regex::Regex> {
    [
        r"\b(?:\d{1,3}\.){3}\d{1,3}\b",                    // IPv4
        r"\*\.[A-Za-z0-9-]+\.[A-Za-z0-9.-]+",              // *.domain.tld glob
        r"\b[A-Za-z0-9-]+\.[A-Za-z0-9-]+\.[A-Za-z]{2,}\b", // multi-label hostname
    ]
    .iter()
    .map(|p| regex::Regex::new(p).unwrap())
    .collect()
}

/// Walk a TOML value, recording each string leaf that matches any leak pattern.
fn collect_leaky_strings(value: &toml::Value, patterns: &[regex::Regex], out: &mut Vec<String>) {
    match value {
        toml::Value::String(s) => {
            if patterns.iter().any(|re| re.is_match(s)) {
                out.push(s.clone());
            }
        }
        toml::Value::Array(items) => {
            for v in items {
                collect_leaky_strings(v, patterns, out);
            }
        }
        toml::Value::Table(t) => {
            for v in t.values() {
                collect_leaky_strings(v, patterns, out);
            }
        }
        _ => {}
    }
}

fn check_gitignore(c: &mut Checks, repo_base: &Path) {
    let gi = std::fs::read_to_string(repo_base.join(".gitignore")).unwrap_or_default();
    if gi
        .lines()
        .any(|l| l.trim().trim_end_matches('/') == ".rosita/generated")
    {
        c.line(Status::Ok, ".gitignore covers .rosita/generated/");
    } else {
        c.line(
            Status::Warn,
            ".gitignore missing .rosita/generated/ (run `rosita init`)",
        );
    }
}

fn check_claude_marker(c: &mut Checks, repo_base: &Path) {
    let path = repo_base.join("CLAUDE.local.md");
    if !path.exists() {
        return; // nothing rendered for Claude yet; not a problem
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    if content.contains(BLOCK_BEGIN) {
        c.line(Status::Ok, "CLAUDE.local.md has the managed import block");
    } else {
        c.line(
            Status::Warn,
            "CLAUDE.local.md exists but lacks the managed block (re-run render)",
        );
    }
}

fn check_overlays(c: &mut Checks, prep: &super::Prepared) {
    let dir = config::generated_dir(&prep.repo_base);
    let current = prep.context.compute_hash();
    let mut found = false;
    for a in &prep.config.agents {
        let path = dir.join(&a.generated_filename);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        found = true;
        match header::extract_context_hash(&content) {
            Some(h) if h == current => c.line(Status::Ok, format!("{}: up to date", a.id)),
            Some(_) => c.line(
                Status::Warn,
                format!("{}: stale (run `rosita refresh`)", a.id),
            ),
            None => c.line(
                Status::Warn,
                format!("{}: present but missing rosita header", a.id),
            ),
        }
    }
    if !found {
        c.line(
            Status::Warn,
            "no overlays generated yet (run `rosita render`)",
        );
    }
}
