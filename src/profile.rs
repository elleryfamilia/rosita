//! Rule-based profile selection.
//!
//! A profile is matched when **all** of its `when` clauses match the detected
//! [`Context`](crate::context::Context). Among matching profiles the highest
//! `priority` wins (ties broken by declaration order). A profile with no `when`
//! clauses always matches and acts as the fallback — the built-in `default`
//! profile has empty rules and priority 0, so selection never fails.

use serde::{Deserialize, Serialize};

use crate::context::Context;

/// A configured profile and the guidance/template it contributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    /// Profile name (also used to find `profiles/<name>.md.j2` templates).
    pub name: String,
    /// Conditions; all must match. Empty = always matches (fallback).
    #[serde(default)]
    pub when: Vec<Rule>,
    /// Higher wins among multiple matches.
    #[serde(default)]
    pub priority: i32,
    /// Optional base-template override (per agent the renderer appends the
    /// agent suffix). Rarely needed.
    #[serde(default)]
    pub template: Option<String>,
    /// Inline guidance markdown (itself rendered as a template).
    #[serde(default)]
    pub guidance: Option<String>,
}

/// A single match condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Which context field to test.
    pub field: Field,
    /// How to compare.
    pub op: Op,
    /// The value to compare against.
    pub value: String,
}

/// Context fields that rules can match against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Detected stack keys, e.g. `rust`, `nextjs`, `node`, `go`, `python`.
    Stack,
    /// Detected language names, e.g. `Rust`, `TypeScript`.
    Language,
    /// Detected package managers, e.g. `cargo`, `pnpm`, `uv`.
    PackageManager,
    /// cwd path relative to the repo root (forward slashes), e.g. `infra/db`.
    Path,
    /// git branch.
    Branch,
    /// repository name.
    Repo,
    /// host class derived from config (`work`, …).
    HostClass,
    /// operating system (`macos`, `linux`, …).
    Os,
    /// CPU architecture (`aarch64`, `x86_64`, …).
    Arch,
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// Exact equality (case-sensitive).
    Equals,
    /// Prefix match.
    StartsWith,
    /// Substring match.
    Contains,
    /// Regex match (anchored as written).
    Matches,
}

/// The outcome of selection.
#[derive(Debug, Clone)]
pub struct Selection {
    /// The chosen profile.
    pub profile: ProfileConfig,
    /// Human-readable reasons the chosen profile matched.
    pub reasons: Vec<String>,
}

/// Select the best profile for `ctx` from `profiles`.
///
/// Returns `None` only if `profiles` is empty (callers always include the
/// built-in `default`, so in practice this is `Some`).
pub fn select<'a>(ctx: &Context, profiles: &'a [ProfileConfig]) -> Option<Selection> {
    let mut best: Option<(&'a ProfileConfig, Vec<String>)> = None;

    for (idx, p) in profiles.iter().enumerate() {
        if let Some(reasons) = matches(ctx, p) {
            let take = match &best {
                None => true,
                Some((cur, _)) => {
                    // Higher priority wins; ties keep the earlier declaration.
                    p.priority > cur.priority
                }
            };
            // `idx` is implicitly the tie-break: we only replace on strictly
            // greater priority, so the first of equal-priority matches stays.
            let _ = idx;
            if take {
                best = Some((p, reasons));
            }
        }
    }

    best.map(|(p, reasons)| Selection {
        profile: p.clone(),
        reasons,
    })
}

/// If `profile` matches `ctx`, return the per-rule reasons; else `None`.
pub fn matches(ctx: &Context, profile: &ProfileConfig) -> Option<Vec<String>> {
    if profile.when.is_empty() {
        return Some(vec!["fallback profile (no rules)".to_string()]);
    }
    let mut reasons = Vec::with_capacity(profile.when.len());
    for rule in &profile.when {
        if rule_matches(ctx, rule) {
            reasons.push(describe(rule));
        } else {
            return None; // AND semantics: one failure disqualifies.
        }
    }
    Some(reasons)
}

fn describe(rule: &Rule) -> String {
    let op = match rule.op {
        Op::Equals => "equals",
        Op::StartsWith => "starts with",
        Op::Contains => "contains",
        Op::Matches => "matches",
    };
    format!("{:?} {} {:?}", rule.field, op, rule.value)
}

fn rule_matches(ctx: &Context, rule: &Rule) -> bool {
    let candidates = field_values(ctx, rule.field);
    candidates.iter().any(|c| apply(rule.op, c, &rule.value))
}

fn apply(op: Op, candidate: &str, value: &str) -> bool {
    match op {
        Op::Equals => candidate == value,
        Op::StartsWith => candidate.starts_with(value),
        Op::Contains => candidate.contains(value),
        Op::Matches => regex::Regex::new(value)
            .map(|re| re.is_match(candidate))
            .unwrap_or_else(|e| {
                crate::warn_user!("invalid regex {value:?} in profile rule: {e}");
                false
            }),
    }
}

fn field_values(ctx: &Context, field: Field) -> Vec<String> {
    match field {
        Field::Stack => ctx.stacks.clone(),
        Field::Language => ctx.languages.clone(),
        Field::PackageManager => ctx.package_managers.clone(),
        Field::Path => vec![ctx.rel_path()],
        Field::Branch => ctx
            .git
            .as_ref()
            .and_then(|g| g.branch.clone())
            .into_iter()
            .collect(),
        Field::Repo => ctx.repo_name.clone().into_iter().collect(),
        Field::HostClass => ctx.system.host_class.clone().into_iter().collect(),
        Field::Os => vec![ctx.system.os.clone()],
        Field::Arch => vec![ctx.system.arch.clone()],
    }
}

/// The built-in profiles, always present as a base layer (overridable by name
/// in user config). Inline guidance is intentionally terse.
pub fn builtin_profiles() -> Vec<ProfileConfig> {
    fn rule(field: Field, op: Op, value: &str) -> Rule {
        Rule {
            field,
            op,
            value: value.to_string(),
        }
    }
    fn p(name: &str, when: Vec<Rule>, priority: i32, guidance: &str) -> ProfileConfig {
        ProfileConfig {
            name: name.to_string(),
            when,
            priority,
            template: None,
            guidance: Some(guidance.to_string()),
        }
    }

    vec![
        // Path/branch profiles are most specific → highest priority.
        p(
            "infra",
            vec![rule(Field::Path, Op::StartsWith, "infra/")],
            50,
            "This is infrastructure code. Be conservative: prefer plans over \
             direct mutation, never apply changes to shared/remote state without \
             explicit confirmation, and call out anything that touches \
             production.",
        ),
        p(
            "experimental",
            vec![rule(Field::Branch, Op::StartsWith, "experiment/")],
            40,
            "Experimental branch — optimize for iteration speed. Throwaway \
             spikes are fine; keep changes scoped to this branch and don't wire \
             them into shared modules yet.",
        ),
        // Stack profiles.
        p(
            "rust",
            vec![rule(Field::Stack, Op::Equals, "rust")],
            20,
            "Rust project. Build with cargo, format with rustfmt, lint with \
             clippy (`cargo clippy --all-targets`). Prefer `?`/`Result` over \
             `unwrap()` in non-test code.",
        ),
        p(
            "nextjs",
            vec![rule(Field::Stack, Op::Equals, "nextjs")],
            25,
            "Next.js app. Respect the existing app/pages router convention. Use \
             the detected package manager. Keep server/client component \
             boundaries explicit.",
        ),
        p(
            "node",
            vec![rule(Field::Stack, Op::Equals, "node")],
            20,
            "Node.js project. Use the detected package manager for scripts; \
             prefer TypeScript where the project already uses it.",
        ),
        p(
            "go",
            vec![rule(Field::Stack, Op::Equals, "go")],
            20,
            "Go project. Use the standard toolchain: `go build`, `go test`, \
             `go vet`, `gofmt`.",
        ),
        p(
            "python",
            vec![rule(Field::Stack, Op::Equals, "python")],
            20,
            "Python project. Prefer the detected tool (uv/poetry) for envs and \
             deps; run tests with pytest.",
        ),
        // Always-on fallback.
        p(
            "default",
            vec![],
            0,
            "No specialized profile matched. Follow the repository's existing \
             conventions and keep changes minimal and well-tested.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::sample_context;

    #[test]
    fn empty_rules_is_fallback() {
        let ctx = sample_context();
        let profile = ProfileConfig {
            name: "default".into(),
            when: vec![],
            priority: 0,
            template: None,
            guidance: None,
        };
        assert!(matches(&ctx, &profile).is_some());
    }

    #[test]
    fn stack_equals_matches() {
        let mut ctx = sample_context();
        ctx.stacks = vec!["rust".into()];
        let r = Rule {
            field: Field::Stack,
            op: Op::Equals,
            value: "rust".into(),
        };
        assert!(rule_matches(&ctx, &r));
        let r2 = Rule {
            field: Field::Stack,
            op: Op::Equals,
            value: "go".into(),
        };
        assert!(!rule_matches(&ctx, &r2));
    }

    #[test]
    fn path_prefix_and_branch_prefix() {
        let mut ctx = sample_context();
        ctx.cwd = ctx.git.as_ref().unwrap().root.join("infra/db");
        let r = Rule {
            field: Field::Path,
            op: Op::StartsWith,
            value: "infra/".into(),
        };
        assert!(rule_matches(&ctx, &r));

        if let Some(g) = ctx.git.as_mut() {
            g.branch = Some("experiment/foo".into());
        }
        let rb = Rule {
            field: Field::Branch,
            op: Op::StartsWith,
            value: "experiment/".into(),
        };
        assert!(rule_matches(&ctx, &rb));
    }

    #[test]
    fn priority_breaks_ties_toward_higher() {
        let mut ctx = sample_context();
        ctx.stacks = vec!["rust".into()];
        ctx.cwd = ctx.git.as_ref().unwrap().root.join("infra/x");
        let sel = select(&ctx, &builtin_profiles()).unwrap();
        // infra (50) beats rust (20)
        assert_eq!(sel.profile.name, "infra");
    }

    #[test]
    fn falls_back_to_default() {
        let ctx = sample_context(); // no stack, not in infra/
        let sel = select(&ctx, &builtin_profiles()).unwrap();
        assert_eq!(sel.profile.name, "default");
    }

    #[test]
    fn regex_match_op() {
        let mut ctx = sample_context();
        if let Some(g) = ctx.git.as_mut() {
            g.branch = Some("release/2026.05".into());
        }
        let r = Rule {
            field: Field::Branch,
            op: Op::Matches,
            value: r"^release/\d{4}\.\d{2}$".into(),
        };
        assert!(rule_matches(&ctx, &r));
    }
}
