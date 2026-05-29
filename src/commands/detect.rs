//! `rosita detect` — show the detected context.

use super::{prepare, Runtime};
use crate::cli::DetectArgs;
use crate::context::Context;

/// Entry point for `rosita detect`.
pub fn run(rt: &Runtime, args: &DetectArgs) -> crate::Result<()> {
    let prep = prepare(rt)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&prep.context)?);
    } else {
        print_human(&prep.context);
    }
    Ok(())
}

fn print_human(ctx: &Context) {
    let dash = |o: &Option<String>| o.clone().unwrap_or_else(|| "-".into());
    let list = |v: &[String]| {
        if v.is_empty() {
            "-".to_string()
        } else {
            v.join(", ")
        }
    };

    println!("Context");
    println!("  cwd        : {}", ctx.cwd.display());
    println!("  base dir   : {}", ctx.repo_base.display());
    println!("  name       : {}", dash(&ctx.repo_name));

    match &ctx.git {
        Some(g) => {
            println!(
                "  git        : branch {} · {} remote(s){}",
                g.branch.clone().unwrap_or_else(|| "(detached)".into()),
                g.remotes.len(),
                if g.is_worktree { " · worktree" } else { "" }
            );
            for r in &g.remotes {
                println!("               {} {}", r.name, r.url);
            }
        }
        None => println!("  git        : (not a git repo — non-repo mode)"),
    }

    println!("  stacks     : {}", list(&ctx.stacks));
    println!("  languages  : {}", list(&ctx.languages));
    println!("  pkg mgrs   : {}", list(&ctx.package_managers));

    if !ctx.commands.is_empty() {
        println!("  commands   :");
        print_cmds("build", &ctx.commands.build);
        print_cmds("test", &ctx.commands.test);
        print_cmds("lint", &ctx.commands.lint);
        print_cmds("run", &ctx.commands.run);
    }

    println!(
        "  system     : {} / {} · host {} · user {}",
        ctx.system.os, ctx.system.arch, ctx.system.hostname, ctx.system.user
    );
    if let Some(c) = &ctx.system.host_class {
        println!("  host class : {c}");
    }
    if let Some(p) = &ctx.system.parent_process {
        println!("  caller     : {p}");
    }
    if !ctx.env.is_empty() {
        println!(
            "  env        : {}",
            ctx.env.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    println!("  hash       : {}", ctx.compute_hash());
}

fn print_cmds(label: &str, cmds: &[String]) {
    if !cmds.is_empty() {
        println!("                 {label:<6} {}", cmds.join(", "));
    }
}
