//! `rosita allow` / `deny` / `trust` — direnv-style trust for repo commands.
//!
//! These manage whether this repo's `command`-backed capabilities may execute.
//! Trust is keyed to the repo's `.rosita` config-bundle hash, so any edit to the
//! config re-locks it. See [`crate::trust`].

use super::Runtime;
use crate::context::repo_base_for;
use crate::trust;

/// `rosita allow` — record the current repo config-bundle hash as trusted.
pub fn allow(rt: &Runtime) -> crate::Result<()> {
    let repo_base = repo_base_for(&rt.cwd);
    if rt.dry_run {
        println!("dry run — would trust {}", repo_base.display());
        return Ok(());
    }
    trust::allow(&repo_base)?;
    println!(
        "trusted {} — its `command` capabilities will now run.",
        repo_base.display()
    );
    println!("(editing .rosita config re-locks trust; re-run `rosita allow`.)");
    Ok(())
}

/// `rosita deny` — revoke trust for this repo.
pub fn deny(rt: &Runtime) -> crate::Result<()> {
    let repo_base = repo_base_for(&rt.cwd);
    if rt.dry_run {
        println!("dry run — would revoke trust for {}", repo_base.display());
        return Ok(());
    }
    let removed = trust::deny(&repo_base)?;
    if removed {
        println!("revoked trust for {}", repo_base.display());
    } else {
        println!("{} was not trusted", repo_base.display());
    }
    Ok(())
}

/// `rosita trust [status]` — show the repo's trust status.
pub fn status(rt: &Runtime) -> crate::Result<()> {
    let repo_base = repo_base_for(&rt.cwd);
    let status = trust::status(&repo_base);
    println!("repo   : {}", repo_base.display());
    println!("status : {}", status.label());
    if let Some(p) = trust::store_path() {
        println!("store  : {}", p.display());
    }
    if status != trust::Status::Trusted {
        println!("\nRun `rosita allow` to trust this repo's `command` capabilities.");
    }
    Ok(())
}
