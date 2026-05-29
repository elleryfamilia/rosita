//! `rosita init` — scaffold `.rosita/` (config + templates) in the repo,
//! and optionally the global config.

use std::path::Path;

use super::{Runtime, SAMPLE_LOCAL_CONFIG, SAMPLE_REPO_CONFIG};
use crate::cli::InitArgs;
use crate::context::{git, repo_base_for};
use crate::writer::{self, AtomicWriter, Writer};
use crate::{config, templates};

/// Entry point for `rosita init`.
pub fn run(rt: &Runtime, args: &InitArgs) -> crate::Result<()> {
    let repo_base = repo_base_for(&rt.cwd);
    let in_repo = git::find_repo_root(&rt.cwd).is_some();
    let writer = AtomicWriter::new(rt.dry_run);

    println!("Initializing rosita in {}", repo_base.display());
    if !in_repo {
        println!("(not a git repository — non-repo mode; .gitignore not managed)");
    }
    if rt.dry_run {
        println!("(dry run — no files written)");
    }

    // Repo config (public) + private local.toml stub.
    scaffold(
        &writer,
        &config::repo_config_path(&repo_base),
        SAMPLE_REPO_CONFIG,
        args.force,
    )?;
    scaffold(
        &writer,
        &config::repo_local_path(&repo_base),
        SAMPLE_LOCAL_CONFIG,
        args.force,
    )?;

    // Repo templates.
    let tdir = config::repo_templates_dir(&repo_base);
    for (name, content) in templates::embedded_all() {
        scaffold(&writer, &tdir.join(name), content, args.force)?;
    }

    // Ensure generated/logs are gitignored — only meaningful inside a repo.
    if in_repo {
        ensure_gitignore(&writer, &repo_base)?;
    }

    // Create the generated dir so it exists for the first render.
    if !rt.dry_run {
        std::fs::create_dir_all(config::generated_dir(&repo_base))?;
    }

    // Optionally scaffold the global config.
    if args.global {
        match config::global_config_dir() {
            Some(dir) => {
                println!("\nGlobal config in {}", dir.display());
                scaffold(
                    &writer,
                    &dir.join("config.toml"),
                    SAMPLE_REPO_CONFIG,
                    args.force,
                )?;
                scaffold(
                    &writer,
                    &dir.join("local.toml"),
                    SAMPLE_LOCAL_CONFIG,
                    args.force,
                )?;
                // Keep the private global layer out of any VCS that dir lives in.
                let gi = dir.join(".gitignore");
                if let Some(updated) =
                    writer::ensure_line(std::fs::read_to_string(&gi).ok().as_deref(), "local.toml")
                {
                    let wf = writer.write(&gi, &updated)?;
                    println!("  {:<13} {}", wf.action.label(), gi.display());
                }
                for (name, content) in templates::embedded_all() {
                    scaffold(
                        &writer,
                        &dir.join("templates").join(name),
                        content,
                        args.force,
                    )?;
                }
            }
            None => println!("\n⚠️  could not resolve a global config dir (no HOME?)"),
        }
    }

    println!("\nNext: `rosita detect`, then `rosita render --agent claude` (or `run claude`).");
    Ok(())
}

/// Write `content` to `path` unless it exists (and `--force` wasn't given).
fn scaffold(writer: &dyn Writer, path: &Path, content: &str, force: bool) -> crate::Result<()> {
    if path.exists() && !force {
        println!("  {:<13} {}", "exists", path.display());
        return Ok(());
    }
    let wf = writer.write(path, content)?;
    println!("  {:<13} {}", wf.action.label(), path.display());
    Ok(())
}

fn ensure_gitignore(writer: &dyn Writer, repo_base: &Path) -> crate::Result<()> {
    let gitignore = repo_base.join(".gitignore");
    let mut content = std::fs::read_to_string(&gitignore).ok();
    let mut changed = false;
    for entry in [".rosita/generated/", ".rosita/logs/", ".rosita/local.toml"] {
        if let Some(updated) = writer::ensure_line(content.as_deref(), entry) {
            content = Some(updated);
            changed = true;
        }
    }
    if changed {
        if let Some(c) = content {
            let wf = writer.write(&gitignore, &c)?;
            println!("  {:<13} {}", wf.action.label(), gitignore.display());
        }
    } else {
        println!("  {:<13} {}", "ok", gitignore.display());
    }
    Ok(())
}
