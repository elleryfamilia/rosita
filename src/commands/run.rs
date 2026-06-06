//! `rosita run <agent> [args...]` — render the overlay, then launch the agent.
//!
//! This is the simple "preflight/wrapper" approach: refresh the generated files
//! for the chosen agent, then hand control to the real agent CLI (replacing this
//! process on Unix so signals and exit codes pass through cleanly). FUSE-style
//! live virtual files are explicitly out of scope for the MVP.
//!
//! Because rosita is the launching parent, it passes a freshness signal to the
//! agent: `ROSITA_RUN=1` + `ROSITA_RENDERED_AT` in the environment (so an agent
//! that can read env — or its hook — knows the context is current), and, for
//! agents with an `append_prompt_flag` (e.g. Claude's `--append-system-prompt`),
//! a short "context is fresh" note injected directly into the launch.
//!
//! For an agent with no persistent local hook but a `launch_context_dir_env`
//! (e.g. Copilot's `COPILOT_CUSTOM_INSTRUCTIONS_DIRS`), rosita also sets that env
//! var to the directory holding the generated overlay, so the agent discovers it
//! at launch without any committed file being touched.

use std::io::{IsTerminal, Write as _};
use std::process::Command;

use anyhow::anyhow;

use super::{now_rfc3339, prepare_with, Choice, ProfileChooser, Runtime};
use crate::adapters::{self, AgentDescriptor, ApplyOptions};
use crate::cli::RunArgs;
use crate::context::Context;
use crate::profile::ProfileConfig;
use crate::vlog;

/// Interactive "which profile?" prompt for `rosita run` when 2+ profiles match
/// and no choice is remembered yet. Falls back to no-profile (no prompt) when
/// stdin/stdout isn't a terminal, so CI/piped runs never block.
struct StdinChooser;

impl ProfileChooser for StdinChooser {
    fn choose(&self, ctx: &Context, candidates: &[ProfileConfig]) -> crate::Result<Choice> {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            crate::warn_user!(
                "{} profiles match but this isn't an interactive terminal — applying none. \
                 Re-run `rosita run` interactively (or use `rosita studio`) to pick.",
                candidates.len()
            );
            return Ok(Choice::Skip);
        }

        let langs = if ctx.stacks.is_empty() {
            "this".to_string()
        } else {
            ctx.stacks.join("/")
        };
        println!(
            "rosita › this {langs} project matches {} profiles:",
            candidates.len()
        );
        for (i, p) in candidates.iter().enumerate() {
            println!("   {}) {}", i + 1, p.name);
        }
        let none_choice = candidates.len() + 1;
        println!("   {none_choice}) none (don't apply rosita here)");

        loop {
            print!(" ❯ ");
            std::io::stdout().flush().ok();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line)? == 0 {
                return Ok(Choice::Skip); // EOF — don't decide.
            }
            match line.trim().parse::<usize>() {
                Ok(n) if (1..=candidates.len()).contains(&n) => {
                    let name = candidates[n - 1].name.clone();
                    println!("rosita › bound \"{name}\" → remembered for this project; launching…");
                    return Ok(Choice::Profile(name));
                }
                Ok(n) if n == none_choice => {
                    println!("rosita › remembered: no rosita profile here.");
                    return Ok(Choice::None);
                }
                _ => println!("  please enter a number between 1 and {none_choice}."),
            }
        }
    }
}

/// Entry point for `rosita run`.
pub fn run(rt: &Runtime, args: &RunArgs) -> crate::Result<()> {
    let agent = args.agent.as_str();
    let prep = prepare_with(rt, &StdinChooser)?;
    let descriptor = adapters::descriptor(&prep.config, agent)
        .ok_or_else(|| anyhow!("unknown agent '{agent}'"))?
        .clone();
    let program = descriptor
        .launch
        .clone()
        .ok_or_else(|| anyhow!("agent '{agent}' is not launchable (no `launch` program)"))?;

    // Fail gracefully before doing any work if the agent CLI isn't installed —
    // no half-rendered overlay or stray global registration for a missing tool.
    // (Dry-run skips this: it only simulates and shouldn't require the binary.)
    if !rt.dry_run && !super::program_on_path(&program) {
        return Err(anyhow!(
            "the '{agent}' CLI ('{program}') isn't on your PATH — install it (or fix PATH), \
             then retry. `rosita render --agent {agent}` still writes the overlay."
        ));
    }

    // Preflight render (unless skipped).
    let rendered = !args.skip_render;
    if rendered {
        let opts = ApplyOptions {
            codex_override: args.codex_override,
            codex_no_override: args.codex_no_override,
            force: false,
        };
        super::render::apply_for_agents(rt, &prep, &[agent.to_string()], &opts)?;
    } else {
        vlog!("skipping pre-launch render (--skip-render)");
    }

    let rendered_at = now_rfc3339();
    let launch_args = build_launch_args(&descriptor, &prep, rendered, &rendered_at, &args.args);
    let extra_env = launch_context_env(&descriptor, &prep);

    if rt.dry_run {
        let env_prefix: String = extra_env.iter().map(|(k, v)| format!("{k}={v} ")).collect();
        println!(
            "dry run — would exec: {env_prefix}{program} {}",
            launch_args.join(" ")
        );
        return Ok(());
    }

    launch(&program, &launch_args, &rendered_at, &rt.cwd, &extra_env)
}

/// Env vars `rosita run` injects so an agent with no persistent local hook finds
/// the overlay at launch: maps `launch_context_dir_env` → the absolute
/// `launch_context_dir` under `.rosita/generated/` (e.g. Copilot's
/// `COPILOT_CUSTOM_INSTRUCTIONS_DIRS` → `<repo>/.rosita/generated/copilot`).
fn launch_context_env(
    descriptor: &AgentDescriptor,
    prep: &super::Prepared,
) -> Vec<(String, String)> {
    let (Some(var), Some(rel)) = (
        &descriptor.launch_context_dir_env,
        &descriptor.launch_context_dir,
    ) else {
        return Vec::new();
    };
    let dir = crate::config::generated_dir(&prep.repo_base).join(rel);
    vec![(var.clone(), dir.to_string_lossy().into_owned())]
}

/// Prepend a freshness note via the agent's prompt flag when we just rendered.
fn build_launch_args(
    descriptor: &AgentDescriptor,
    prep: &super::Prepared,
    rendered: bool,
    rendered_at: &str,
    user_args: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    if rendered {
        if let Some(flag) = &descriptor.append_prompt_flag {
            out.push(flag.clone());
            out.push(format!(
                "rosita: project context refreshed for profile '{}' at {rendered_at} — current. \
                 Run `rosita refresh` if the project changes mid-session.",
                prep.profile_label()
            ));
        }
    }
    out.extend(user_args.iter().cloned());
    out
}

/// Launch `program` with `args` in `cwd`, passing the rosita freshness signal in
/// the environment. On Unix this replaces the current process; elsewhere it
/// spawns, waits, and mirrors the exit code.
fn launch(
    program: &str,
    args: &[String],
    rendered_at: &str,
    cwd: &std::path::Path,
    extra_env: &[(String, String)],
) -> crate::Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env("ROSITA_RUN", "1")
        .env("ROSITA_RENDERED_AT", rendered_at);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    vlog!("launching: {program} {}", args.join(" "));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // exec only returns on failure.
        let err = cmd.exec();
        Err(anyhow!("failed to exec '{program}': {err}")
            .context("is the agent CLI installed and on PATH?"))
    }

    #[cfg(not(unix))]
    {
        use anyhow::Context as _;
        let status = cmd
            .status()
            .with_context(|| format!("failed to launch '{program}'"))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}
