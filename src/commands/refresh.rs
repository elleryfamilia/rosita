//! `rosita refresh` — re-render overlays for already-initialized agents.
//!
//! Without `--agent`, refreshes every agent whose generated overlay already
//! exists; if none do, falls back to the default agent. Hash-skipping means a
//! refresh with no context change is a cheap no-op (unless `--force`).

use super::{prepare, resolve_agents, Prepared, Runtime};
use crate::adapters::ApplyOptions;
use crate::cli::RefreshArgs;
use crate::config;

/// Entry point for `rosita refresh`.
pub fn run(rt: &Runtime, args: &RefreshArgs) -> crate::Result<()> {
    let prep = prepare(rt)?;

    let agents: Vec<String> = match &args.agent {
        Some(_) => resolve_agents(args.agent.as_deref(), &prep.config)?,
        None => {
            let existing = existing_overlay_agents(&prep);
            if existing.is_empty() {
                println!(
                    "no generated overlays found; rendering the default agent ({})",
                    prep.config.default_agent
                );
                vec![prep.config.default_agent.clone()]
            } else {
                existing
            }
        }
    };

    let opts = ApplyOptions {
        codex_override: args.codex_override,
        force: args.force,
    };
    super::render::apply_for_agents(rt, &prep, &agents, &opts)
}

/// Which agents already have a generated overlay on disk.
fn existing_overlay_agents(prep: &Prepared) -> Vec<String> {
    let dir = config::generated_dir(&prep.repo_base);
    prep.config
        .agents
        .iter()
        .filter(|a| dir.join(&a.generated_filename).exists())
        .map(|a| a.id.clone())
        .collect()
}
