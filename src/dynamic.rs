//! Resolving dynamic capabilities — live provider/command output.
//!
//! A capability is *dynamic* when it has a `provider` (a built-in probe) or a
//! `command` (a shell command). At render time [`resolve`] produces its output,
//! honoring the cache:
//!
//! - built-in `provider` → always run (safe probes),
//! - `command` → run unless `allow_exec` is `false` (the per-capability
//!   off-switch), in which case a skip note is rendered instead.
//!
//! [`DynamicMode::ReadOnly`] (used by `explain` and dry-run) never executes or
//! writes — it surfaces only what is already cached. This keeps `explain`
//! side-effect-free and honors "dry-run touches nothing".

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::capability::Capability;
use crate::context::Context;
use crate::providers::{self, ProviderOutput};

/// Default cache TTL when a dynamic capability sets no `cache`.
const DEFAULT_TTL: Duration = Duration::from_secs(60);

/// How dynamic capabilities are resolved during a render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicMode {
    /// May execute providers/commands and write the cache (real renders).
    Live,
    /// Cache-only: never executes or writes (explain, dry-run).
    ReadOnly,
}

/// The outcome of resolving one dynamic capability.
pub struct Resolution {
    /// Embedded output, if available (absent when unavailable or not cached).
    pub output: Option<ProviderOutput>,
    /// Set when a command was skipped (e.g. `allow_exec = false`); the value is
    /// the human note, rendered in place of output.
    pub skipped: Option<String>,
}

/// Resolve a capability's dynamic output, or `None` if it isn't dynamic.
pub fn resolve(
    cap: &Capability,
    ctx: &Context,
    repo_base: &Path,
    mode: DynamicMode,
    now: DateTime<Utc>,
) -> Option<Resolution> {
    if !cap.is_dynamic() {
        return None;
    }
    let live = mode == DynamicMode::Live;
    let ttl = cap
        .cache
        .as_deref()
        .and_then(providers::parse_duration)
        .unwrap_or(DEFAULT_TTL);

    // An explicit command wins over a provider when both are set.
    if let Some(command) = &cap.command {
        // Per-capability off-switch (`allow_exec = false` disables execution).
        if !cap.allow_exec {
            return Some(Resolution {
                output: None,
                skipped: Some(
                    "execution disabled for this capability (allow_exec = false)".to_string(),
                ),
            });
        }
        let key = format!("cmd-{}", cap.id);
        return Some(Resolution {
            output: providers::run_command(
                command,
                cap.script_lang.as_deref(),
                repo_base,
                &key,
                ttl,
                now,
                live,
            ),
            skipped: None,
        });
    }

    let pid = cap.provider.as_deref()?;
    Some(Resolution {
        output: providers::probe_one(pid, ctx, repo_base, ttl, now, live),
        skipped: None,
    })
}
