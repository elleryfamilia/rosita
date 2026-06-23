//! Per-stage slash-command generation — the workflow "command channel".
//!
//! A bound workflow renders two ways. Channel 1 (see [`crate::render`]) is the
//! always-on `## Workflow` context section. Channel 2 — this module — is for
//! agents that support project slash commands: one generated command file per
//! stage, carrying that stage's contract (its purpose, the handoff artifact to
//! read/write, the gate, the exit checklist, and an argument slot for the
//! specific task).
//!
//! Files land in a dedicated [`COMMAND_NAMESPACE`] subdirectory of the agent's
//! command dir (e.g. `.claude/commands/loadout/plan.md`) — a dir loadout owns
//! entirely, so the commands invoke as `/loadout:<stage>` and cleanup can remove
//! the whole dir without touching the user's own commands.

use serde::{Deserialize, Serialize};

use crate::workflow::{self, Workflow, WorkflowStage, ARTIFACT_SUBDIR};

/// The namespace subdir loadout owns under an agent's command directory.
pub const COMMAND_NAMESPACE: &str = "loadout";

/// On-disk format for an agent's command files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandFormat {
    /// Markdown with YAML frontmatter (Claude Code, opencode).
    Markdown,
    /// TOML with `description` + `prompt` (Gemini CLI).
    Toml,
}

impl CommandFormat {
    /// File extension for this format.
    pub fn ext(self) -> &'static str {
        match self {
            CommandFormat::Markdown => "md",
            CommandFormat::Toml => "toml",
        }
    }

    /// The placeholder this agent substitutes the user's command text into.
    fn arg_placeholder(self) -> &'static str {
        match self {
            CommandFormat::Markdown => "$ARGUMENTS",
            CommandFormat::Toml => "{{args}}",
        }
    }
}

/// A generated command file: its name within the namespace dir + its content.
pub struct StageCommand {
    /// Filename (e.g. `plan.md`), written under `<commands_dir>/loadout/`.
    pub filename: String,
    /// Full file content (frontmatter/TOML header + prompt body).
    pub content: String,
}

/// Render one command file per stage of `wf` in `format`, in stage order.
pub fn stage_commands(wf: &Workflow, format: CommandFormat) -> Vec<StageCommand> {
    wf.stages
        .iter()
        .enumerate()
        .map(|(i, stage)| render_stage_command(wf, i, stage, format))
        .collect()
}

fn render_stage_command(
    wf: &Workflow,
    idx: usize,
    stage: &WorkflowStage,
    format: CommandFormat,
) -> StageCommand {
    let filename = format!("{}.{}", slug(&stage.name), format.ext());
    let description = stage
        .purpose
        .clone()
        .unwrap_or_else(|| format!("{} — {} stage", wf.title(), stage.name));
    let body = stage_body(wf, idx, stage, format.arg_placeholder());
    let content = match format {
        CommandFormat::Markdown => {
            format!(
                "---\ndescription: {}\n---\n\n{body}\n",
                yaml_dq(&description)
            )
        }
        // Build via the toml crate so escaping is always correct.
        CommandFormat::Toml => toml::to_string(&GeminiCommandFile {
            description: &description,
            prompt: &body,
        })
        .unwrap_or_default(),
    };
    StageCommand { filename, content }
}

/// Serializable shape of a Gemini CLI command file (`description` + `prompt`).
#[derive(Serialize)]
struct GeminiCommandFile<'a> {
    description: &'a str,
    prompt: &'a str,
}

/// The stage's prompt body — the contract the agent follows when the command
/// runs: where it sits in the spine, what to do, the handoff to read/write, the
/// gate, the exit checklist, and the user's per-run focus via `arg`.
fn stage_body(wf: &Workflow, idx: usize, stage: &WorkflowStage, arg: &str) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "You're at the **{}** stage ({}/{}) of the _{}_ workflow.\n",
        stage.name,
        idx + 1,
        wf.stages.len(),
        wf.title()
    );
    if let Some(purpose) = &stage.purpose {
        let _ = writeln!(s, "{purpose}\n");
    }
    // `artifact_env_var` returns `None` for an unsafe name, so this both
    // validates the artifact and yields its `LOADOUT_<NAME>_PATH` env var.
    if let Some(reads) = &stage.reads {
        if let Some(env) = workflow::artifact_env_var(reads) {
            let _ = writeln!(
                s,
                "First read the handoff from `.loadout/{ARTIFACT_SUBDIR}/{reads}` \
                 (its path is also in `${env}`).\n"
            );
        }
    }
    if let Some(writes) = &stage.writes {
        if let Some(env) = workflow::artifact_env_var(writes) {
            let _ = writeln!(
                s,
                "Write your output to `.loadout/{ARTIFACT_SUBDIR}/{writes}` \
                 (its path is also in `${env}`) so the next stage can pick it up.\n"
            );
        }
    }
    if stage.gate {
        let _ = writeln!(
            s,
            "This stage is a checkpoint — pause and let me review before moving on.\n"
        );
    }
    if !stage.exit.is_empty() {
        let _ = writeln!(s, "Done when:");
        for item in &stage.exit {
            let _ = writeln!(s, "- {item}");
        }
        s.push('\n');
    }
    let _ = write!(s, "Focus for this run: {arg}");
    s.trim_end().to_string()
}

/// Slugify a free-string stage name into a safe command filename stem: lowercase
/// alphanumerics, runs of anything else collapsed to a single `-`, no leading or
/// trailing `-`. Falls back to `stage` for a name with no alphanumerics.
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() {
        "stage".to_string()
    } else {
        out
    }
}

/// Double-quote and escape a string for a single-line YAML frontmatter value.
fn yaml_dq(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin(id: &str) -> Workflow {
        crate::workflow::builtin_workflows()
            .into_iter()
            .find(|w| w.id == id)
            .unwrap()
    }

    #[test]
    fn markdown_command_has_frontmatter_args_and_handoff() {
        let cmds = stage_commands(&builtin("lean"), CommandFormat::Markdown);
        // One file per stage, named by the stage slug, in order.
        let names: Vec<&str> = cmds.iter().map(|c| c.filename.as_str()).collect();
        assert_eq!(
            names,
            vec!["explore.md", "plan.md", "implement.md", "commit.md"]
        );

        let plan = cmds.iter().find(|c| c.filename == "plan.md").unwrap();
        assert!(plan.content.starts_with("---\ndescription: "));
        assert!(plan.content.contains("$ARGUMENTS"));
        // The plan stage writes the handoff artifact (path + env var).
        assert!(plan.content.contains(".loadout/workflow/artifacts/plan.md"));
        assert!(plan.content.contains("$LOADOUT_PLAN_PATH"));

        // The implement stage reads that same handoff.
        let implement = cmds.iter().find(|c| c.filename == "implement.md").unwrap();
        assert!(implement.content.contains("read the handoff"));
        assert!(implement.content.contains("plan.md"));

        // The commit stage is a gate with an exit checklist.
        let commit = cmds.iter().find(|c| c.filename == "commit.md").unwrap();
        assert!(commit.content.contains("checkpoint"));
        assert!(commit.content.contains("Done when:"));
    }

    #[test]
    fn toml_command_is_valid_and_uses_gemini_args() {
        let cmds = stage_commands(&builtin("spec-driven"), CommandFormat::Toml);
        let plan = cmds.iter().find(|c| c.filename == "plan.toml").unwrap();
        // Parses as TOML with description + prompt.
        let v: toml::Value = toml::from_str(&plan.content).expect("valid TOML");
        assert!(v.get("description").and_then(|d| d.as_str()).is_some());
        let prompt = v.get("prompt").and_then(|p| p.as_str()).unwrap();
        // Gemini's placeholder, not Claude's.
        assert!(prompt.contains("{{args}}"), "gemini arg placeholder");
        assert!(!prompt.contains("$ARGUMENTS"));
        // spec's plan reads spec.md and writes plan.md.
        assert!(prompt.contains("spec.md"));
        assert!(prompt.contains("plan.md"));
    }

    #[test]
    fn slug_cleans_free_string_stage_names() {
        assert_eq!(slug("plan"), "plan");
        assert_eq!(slug("Plan It!"), "plan-it");
        assert_eq!(slug("  spec / design  "), "spec-design");
        assert_eq!(slug("!!!"), "stage");
    }

    #[test]
    fn description_is_escaped_in_yaml_frontmatter() {
        // A purpose with a quote/colon must not break the markdown frontmatter.
        let wf = Workflow {
            id: "x".into(),
            description: None,
            stages: vec![WorkflowStage {
                name: "plan".into(),
                purpose: Some("Write the \"spec\": be precise".into()),
                reads: None,
                writes: None,
                gate: false,
                exit: vec![],
            }],
            modeled_on: None,
            researched: None,
            disabled: false,
            origin: crate::fragment::Layer::Global,
        };
        let cmds = stage_commands(&wf, CommandFormat::Markdown);
        assert!(cmds[0]
            .content
            .contains(r#"description: "Write the \"spec\": be precise""#));
    }

    #[test]
    fn unsafe_artifact_name_is_not_referenced() {
        // A hostile/malformed artifact name never becomes a path in the command.
        let wf = Workflow {
            id: "x".into(),
            description: None,
            stages: vec![WorkflowStage {
                name: "plan".into(),
                purpose: Some("do".into()),
                reads: None,
                writes: Some("../escape.md".into()),
                gate: false,
                exit: vec![],
            }],
            modeled_on: None,
            researched: None,
            disabled: false,
            origin: crate::fragment::Layer::Global,
        };
        let cmds = stage_commands(&wf, CommandFormat::Markdown);
        assert!(!cmds[0].content.contains("escape.md"));
        assert!(!cmds[0].content.contains("Write your output"));
    }
}
