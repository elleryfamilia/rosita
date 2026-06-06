//! `rosita sync` — git-backed sync of the global config (capabilities & profiles)
//! across machines. `init` sets a machine up, `clone` onboards a new one, and a
//! bare `rosita sync` pulls the latest and pushes local edits.

use std::io::{IsTerminal, Write as _};
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
            finish_init(&dir, a.remote.is_some(), &p)?;
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

/// After a local `sync init` with no remote: offer to create the GitHub repo via
/// `gh` (interactive), or print manual guidance.
fn finish_init(dir: &Path, remote_given: bool, p: &Painter) -> Result<()> {
    if remote_given {
        println!(
            "{} config repo ready · pushed to {}",
            p.green("✓"),
            p.dim(&sync::remote_name(dir))
        );
        return Ok(());
    }
    println!("{} config dir is now a git repo.", p.green("✓"));
    if sync::gh_available() && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        offer_gh(dir, p)
    } else {
        println!(
            "{}",
            p.dim("  publish it: `rosita sync init <url>`, or with gh:")
        );
        println!(
            "{}",
            p.dim("    gh repo create rosita-config --private --source . --push   (from your config dir)")
        );
        Ok(())
    }
}

/// Interactive `gh repo create` flow (name + visibility) when `gh` is available
/// and no remote was given.
fn offer_gh(dir: &Path, p: &Painter) -> Result<()> {
    if !prompt_yes("  Create a GitHub repo with gh and push now?", true)? {
        println!(
            "{}",
            p.dim("  ok — publish later with `rosita sync init <url>` or `gh repo create … --source . --push`.")
        );
        return Ok(());
    }
    let name = prompt_line("  Repo name", "rosita-config")?;
    // config.toml is secret-free by design, so public is a safe, zero-auth option.
    let public = prompt_yes(
        "  Make it public? (config.toml carries no secrets; public = no git auth on other boxes)",
        false,
    )?;
    sync::create_repo_gh(dir, &name, !public)?;
    println!(
        "{} created {} repo {} · pushed",
        p.green("✓"),
        if public { "public" } else { "private" },
        p.dim(&name)
    );
    Ok(())
}

/// A yes/no prompt with a default.
fn prompt_yes(question: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let t = line.trim().to_ascii_lowercase();
    Ok(if t.is_empty() {
        default_yes
    } else {
        t.starts_with('y')
    })
}

/// A line prompt with a default shown in brackets.
fn prompt_line(question: &str, default: &str) -> Result<String> {
    print!("{question} [{default}]: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let t = line.trim();
    Ok(if t.is_empty() {
        default.to_string()
    } else {
        t.to_string()
    })
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
