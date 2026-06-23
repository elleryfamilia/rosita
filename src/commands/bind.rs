//! `load use <loadout>` — pin this project to a named loadout.
//!
//! Writes the remembered binding directly, the same store the interactive
//! chooser uses when 2+ loadouts match (see [`crate::binding`]). The bind is
//! name-only (no `targets_hash`): an explicit manual choice is trusted by name
//! and keeps working even if the loadout is later retargeted.

use anyhow::{bail, Context as _};

use super::Runtime;
use crate::binding::{self, Binding};
use crate::cli::UseArgs;
use crate::config::Config;
use crate::context::{self, Scope};

/// Entry point for `load use`.
pub fn run(rt: &Runtime, args: &UseArgs) -> crate::Result<()> {
    let repo_base = context::repo_base_for(&rt.cwd);
    let config = Config::load(&repo_base).context("loading configuration")?;
    let context =
        context::detect_context_with(&rt.cwd, &config, false).context("detecting context")?;

    // The loadout must exist, or pinning it would just produce an empty overlay.
    if !config.profiles.iter().any(|p| p.name == args.loadout) {
        let names: Vec<&str> = config.profiles.iter().map(|p| p.name.as_str()).collect();
        if names.is_empty() {
            bail!(
                "no loadouts defined yet — create one with `load studio`, then `load use <name>`"
            );
        }
        bail!(
            "unknown loadout '{}' (known: {})",
            args.loadout,
            names.join(", ")
        );
    }

    let where_ = match context.scope() {
        Scope::Repo => ".loadout/local.toml",
        Scope::Machine => "the global bindings store",
    };

    if rt.dry_run {
        println!(
            "would pin this project to loadout '{}' (in {where_})",
            args.loadout
        );
        return Ok(());
    }

    binding::write(&context, &Binding::profile(args.loadout.clone()))
        .context("remembering the loadout choice")?;
    println!(
        "pinned this project to loadout '{}' — remembered in {where_}.",
        args.loadout
    );
    Ok(())
}
