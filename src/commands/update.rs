//! `rosita update` — self-update to the latest release via cargo-dist's updater.
//!
//! Works for installs done with the rosita installer (which leaves an install
//! receipt). Other installs (`cargo install`, package managers) report how to
//! switch to an installer-based install instead of erroring.

use super::Runtime;
use crate::cli::UpdateArgs;
use crate::style::Painter;
use crate::update::{self, Outcome};

/// Entry point for `rosita update`.
pub fn run(_rt: &Runtime, args: &UpdateArgs) -> crate::Result<()> {
    let p = Painter::auto();
    match update::perform(args.check)? {
        Outcome::Updated { from, to } => {
            let was = from
                .map(|f| format!("{} → ", p.dim(&f)))
                .unwrap_or_default();
            println!("  {} updated rosita {was}{}", p.green("✓"), p.bold(&to));
        }
        Outcome::AlreadyCurrent => println!(
            "  {} rosita {} is the latest release",
            p.green("✓"),
            p.bold(env!("CARGO_PKG_VERSION"))
        ),
        Outcome::UpdateAvailable => println!(
            "  {} a newer rosita is available — run {} to install",
            p.cyan("↑"),
            p.bold("rosita update")
        ),
        Outcome::NotManaged => {
            println!(
                "  {} this rosita wasn't installed via the rosita installer, so it can't \
                 self-update.",
                p.yellow("⚠")
            );
            println!(
                "    {}",
                p.dim("reinstall with the installer to enable `rosita update`:")
            );
            println!(
                "    {}",
                p.dim(
                    "curl -LsSf https://github.com/elleryfamilia/rosita/releases/latest/download/rosita-installer.sh | sh"
                )
            );
        }
    }
    Ok(())
}
