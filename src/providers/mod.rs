//! Native environment discovery — the "agent-env idea", built in.
//!
//! A [`EnvProvider`] probes the live environment (host, tailnet, docker,
//! installed toolchains and agent CLIs) and returns a [`ProviderOutput`] a
//! dynamic capability can embed. Probing is **best-effort**: a missing tool
//! degrades to `None`, never an error.
//!
//! Provider output is machine-specific and volatile, so it is:
//! - **redacted** ([`crate::redact::redact_secrets`]) before it leaves here,
//! - kept **out of [`crate::context::Context`]** so it never affects the context
//!   hash (it lives in a separate [`Probes`] value used only for display/render),
//! - **cached** under `.rosita/cache/<id>.json` with a TTL.
//!
//! New providers are rows in [`builtin_providers`]; a generic trust-gated
//! `command` provider arrives in a later phase.

mod ai_tools;
mod docker;
mod host;
mod tailnet;
mod toolchain;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config;
use crate::context::Context;
use crate::{redact, vlog};

/// What a provider discovered: a human-readable summary plus structured data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderOutput {
    /// Redacted, human-readable summary (markdown-ish).
    pub text: String,
    /// Structured form for templates / `--json`.
    pub data: serde_json::Value,
}

/// A unit of live environment discovery.
pub trait EnvProvider {
    /// Stable id (`host`, `tailnet`, `docker`, `toolchain`, `ai-tools`).
    fn id(&self) -> &'static str;
    /// Probe the environment. `Ok(None)` means "not available here" (tool
    /// absent, daemon down, logged out) — never fatal.
    fn probe(&self, ctx: &Context) -> crate::Result<Option<ProviderOutput>>;
}

/// The built-in provider registry, in display order.
pub fn builtin_providers() -> Vec<Box<dyn EnvProvider>> {
    vec![
        Box::new(host::HostProvider),
        Box::new(toolchain::ToolchainProvider),
        Box::new(ai_tools::AiToolsProvider),
        Box::new(tailnet::TailnetProvider),
        Box::new(docker::DockerProvider),
    ]
}

/// Probe results for the available providers, in registry order.
#[derive(Debug, Clone, Default)]
pub struct Probes {
    /// `(provider id, output)` for each available provider.
    pub entries: Vec<(String, ProviderOutput)>,
}

impl Probes {
    /// Whether nothing was discovered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Probe every built-in provider (cache-backed, redacted), returning what is
/// available. Provider errors are logged at verbose level and skipped.
///
/// `now` is injected for deterministic cache-freshness tests.
pub fn gather(ctx: &Context, repo_base: &Path, ttl: Duration, now: DateTime<Utc>) -> Probes {
    let mut entries = Vec::new();
    for p in builtin_providers() {
        match probe_cached(p.as_ref(), ctx, repo_base, ttl, now) {
            Ok(Some(out)) => entries.push((p.id().to_string(), out)),
            Ok(None) => {}
            Err(e) => vlog!("provider '{}' degraded: {e:#}", p.id()),
        }
    }
    Probes { entries }
}

/// A cache entry on disk.
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    generated_at: String,
    text: String,
    data: serde_json::Value,
}

/// Probe `p`, serving a fresh cache hit when available; otherwise probe live,
/// redact, and (best-effort) write the cache. `None` results are not cached.
fn probe_cached(
    p: &dyn EnvProvider,
    ctx: &Context,
    repo_base: &Path,
    ttl: Duration,
    now: DateTime<Utc>,
) -> crate::Result<Option<ProviderOutput>> {
    let path = config::cache_dir(repo_base).join(format!("{}.json", p.id()));

    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(entry) = serde_json::from_str::<CacheEntry>(&text) {
            if is_fresh(&entry.generated_at, ttl, now) {
                return Ok(Some(ProviderOutput {
                    text: entry.text,
                    data: entry.data,
                }));
            }
        }
    }

    let Some(mut out) = p.probe(ctx)? else {
        return Ok(None);
    };
    out.text = redact::redact_secrets(&out.text);

    let entry = CacheEntry {
        generated_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        text: out.text.clone(),
        data: out.data.clone(),
    };
    if let Ok(serialized) = serde_json::to_string(&entry) {
        if std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).is_ok() {
            let _ = std::fs::write(&path, serialized); // best-effort
        }
    }
    Ok(Some(out))
}

/// Whether a cache `generated_at` (RFC3339) is within `ttl` of `now`.
pub fn is_fresh(generated_at: &str, ttl: Duration, now: DateTime<Utc>) -> bool {
    let Ok(t) = DateTime::parse_from_rfc3339(generated_at) else {
        return false;
    };
    let age = now.signed_duration_since(t.with_timezone(&Utc));
    match age.to_std() {
        Ok(age) => age <= ttl, // 0 <= age <= ttl
        Err(_) => false,       // negative age (future stamp) → re-probe
    }
}

/// Parse a duration like `60s`, `5m`, `2h`, `500ms`, or a bare seconds count.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    match s.find(|c: char| c.is_ascii_alphabetic()) {
        None => s.parse::<u64>().ok().map(Duration::from_secs),
        Some(i) => {
            let n: u64 = s[..i].trim().parse().ok()?;
            match s[i..].trim() {
                "ms" => Some(Duration::from_millis(n)),
                "s" => Some(Duration::from_secs(n)),
                "m" => Some(Duration::from_secs(n * 60)),
                "h" => Some(Duration::from_secs(n * 3600)),
                _ => None,
            }
        }
    }
}

// --- shared exec helpers (used by the shell-out providers) -------------------

/// Run `program args…` and return its stdout (trimmed) on success, falling back
/// to stderr when stdout is empty. `None` when the program can't be spawned
/// (not installed) or exits non-zero (daemon down, logged out, …).
pub(crate) fn run_ok(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Some(stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    (!stderr.is_empty()).then_some(stderr)
}

/// Probe `<tool> --version` for each tool, collecting `(tool, version)` for the
/// ones present. Shared by the `toolchain` and `ai-tools` providers.
pub(crate) fn probe_versions(tools: &[&str]) -> Vec<(String, String)> {
    tools
        .iter()
        .filter_map(|t| run_ok(t, &["--version"]).map(|raw| (t.to_string(), parse_version(&raw))))
        .collect()
}

/// Extract a version token (`1.85.0`, `2.43.0`, …) from a `--version` line,
/// falling back to the trimmed first line when no version-looking token is found.
pub(crate) fn parse_version(raw: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"\d+\.\d+(?:\.\d+)?(?:[\w.\-+]*)").unwrap());
    let first = raw.lines().next().unwrap_or("").trim();
    re.find(first)
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_units() {
        assert_eq!(parse_duration("60s"), Some(Duration::from_secs(60)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration("90"), Some(Duration::from_secs(90))); // bare = seconds
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("10x"), None);
        assert_eq!(parse_duration("abc"), None);
    }

    #[test]
    fn cache_freshness_within_ttl() {
        let now = DateTime::parse_from_rfc3339("2026-05-29T00:01:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // 30s old, ttl 60s → fresh.
        assert!(is_fresh(
            "2026-05-29T00:00:30Z",
            Duration::from_secs(60),
            now
        ));
        // 60s old, ttl 60s → still fresh (boundary inclusive).
        assert!(is_fresh(
            "2026-05-29T00:00:00Z",
            Duration::from_secs(60),
            now
        ));
        // 61s old, ttl 60s → stale.
        assert!(!is_fresh(
            "2026-05-28T23:59:59Z",
            Duration::from_secs(60),
            now
        ));
        // garbage / future / unparseable → not fresh.
        assert!(!is_fresh("not-a-date", Duration::from_secs(60), now));
        assert!(!is_fresh(
            "2026-05-29T00:02:00Z",
            Duration::from_secs(60),
            now
        ));
    }

    #[test]
    fn parse_version_extracts_token() {
        assert_eq!(parse_version("cargo 1.85.0 (abc 2025-01-01)"), "1.85.0");
        assert_eq!(parse_version("git version 2.43.0"), "2.43.0");
        assert_eq!(parse_version("Python 3.12.1"), "3.12.1");
        assert_eq!(parse_version("v22.1.0"), "22.1.0");
        // No version token → trimmed first line.
        assert_eq!(parse_version("weird tool\nsecond line"), "weird tool");
    }

    #[test]
    fn registry_has_the_five_builtins() {
        let ids: Vec<&str> = builtin_providers().iter().map(|p| p.id()).collect();
        assert_eq!(ids, ["host", "toolchain", "ai-tools", "tailnet", "docker"]);
    }
}
