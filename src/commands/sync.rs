//! `rosita sync` — git-backed sync of the global config (capabilities & profiles)
//! across machines. `init` sets a machine up, `clone` onboards a new one, and a
//! bare `rosita sync` pulls the latest and pushes local edits.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context as _, Result};

use crate::cli::{SyncAction, SyncArgs};
use crate::style::Painter;
use crate::sync::{self, PullOutcome, PushOutcome};

/// Manual sync ops are interactive — the user is waiting — so give them a roomy
/// timeout. (Auto-pull on the `run` hot path uses the short `[sync] timeout`.)
const MANUAL_TIMEOUT: Duration = Duration::from_secs(30);

pub fn run(_rt: &super::Runtime, args: &SyncArgs) -> Result<()> {
    let dir = sync::config_dir()?;
    let p = Painter::auto();
    match &args.action {
        Some(SyncAction::Init(a)) => {
            sync::init(&dir, a.remote.as_deref(), MANUAL_TIMEOUT)
                .context("setting up the config repo")?;
            match a.remote.as_deref() {
                Some(_) => println!(
                    "{} config repo ready · pushed to {}",
                    p.green("✓"),
                    p.dim(&sync::remote_name(&dir))
                ),
                None => println!(
                    "{} config dir is now a git repo · add a remote, then `rosita sync` to publish",
                    p.green("✓")
                ),
            }
            println!(
                "{}",
                p.dim("  tracked: config.toml + templates/   ignored: local.toml, generated/, cache/, logs/")
            );
            Ok(())
        }
        Some(SyncAction::Clone(a)) => {
            sync::clone(&a.url, &dir, MANUAL_TIMEOUT).context("cloning the config repo")?;
            println!(
                "{} cloned your config into {}",
                p.green("✓"),
                p.dim(&dir.display().to_string())
            );
            println!(
                "{}",
                p.dim("  a fresh local.toml was created for this machine (gitignored).")
            );
            Ok(())
        }
        None => sync_now(&dir, &p),
    }
}

fn sync_now(dir: &Path, p: &Painter) -> Result<()> {
    if !sync::is_synced(dir) {
        bail!(
            "config isn't set up for sync yet — run `rosita sync init [remote-url]` \
             (or `rosita sync clone <url>` on a new machine) first"
        );
    }
    let remote = sync::remote_name(dir);

    match sync::pull(dir, MANUAL_TIMEOUT).context("pulling from the remote")? {
        PullOutcome::Pulled(0) => println!("{} already up to date · {}", p.green("✓"), p.dim(&remote)),
        PullOutcome::Pulled(n) => println!(
            "{} pulled {} · {}",
            p.green("✓"),
            changes(n),
            p.dim(&remote)
        ),
        PullOutcome::Diverged => bail!(
            "local and remote diverged — reconcile by hand in {} (e.g. `git -C {} pull --rebase`), then `rosita sync`",
            dir.display(),
            dir.display()
        ),
    }

    match sync::commit_push(dir, "rosita: sync config", MANUAL_TIMEOUT)
        .context("pushing to the remote")?
    {
        PushOutcome::Pushed => println!("{} pushed your changes", p.green("✓")),
        PushOutcome::NothingToPush => println!("{} nothing to push", p.dim("·")),
        PushOutcome::Diverged => {
            bail!("push rejected — the remote moved ahead; run `rosita sync` again to pull first")
        }
    }
    Ok(())
}

/// "1 change" / "N changes".
fn changes(n: usize) -> String {
    if n == 1 {
        "1 change".to_string()
    } else {
        format!("{n} changes")
    }
}
