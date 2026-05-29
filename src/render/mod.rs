//! Template rendering: context + profile + template → a finished overlay.
//!
//! The low-level [`TemplateRenderer`] trait abstracts the engine (here
//! minijinja). [`render`] is the high-level entry the adapters call: it resolves
//! the base template, renders the profile guidance, prepends the generated
//! header, and returns the content plus the context hash and provenance.

pub mod header;

use std::path::Path;

use minijinja::{Environment, UndefinedBehavior, Value};
use serde::Serialize;

use crate::config::{self, Config};
use crate::context::Context;
use crate::profile::Selection;
use crate::templates;

/// Abstraction over a template engine.
pub trait TemplateRenderer {
    /// Render `source` against `model`, returning the output string.
    fn render_str(&self, source: &str, model: &Value) -> crate::Result<String>;
}

/// minijinja-backed renderer with lenient undefined handling (so optional
/// context fields render as empty rather than erroring).
pub struct MinijinjaRenderer {
    env: Environment<'static>,
}

impl Default for MinijinjaRenderer {
    fn default() -> Self {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Lenient);
        MinijinjaRenderer { env }
    }
}

impl TemplateRenderer for MinijinjaRenderer {
    fn render_str(&self, source: &str, model: &Value) -> crate::Result<String> {
        self.env
            .render_str(source, model)
            .map_err(|e| anyhow::anyhow!("template render error: {e:#}"))
    }
}

/// Inputs for a render.
pub struct RenderRequest<'a> {
    /// Agent id shown in the header (`claude`/`codex`/`generic`).
    pub agent: &'a str,
    /// Base template name (`claude`/`agents`/`generic`).
    pub template_name: &'a str,
    /// Detected context.
    pub context: &'a Context,
    /// Selected profile + reasons.
    pub selection: &'a Selection,
    /// Loaded config (for source provenance).
    pub config: &'a Config,
    /// Injected generation timestamp (RFC3339) — passed in for testability.
    pub generated_at: String,
}

/// Result of a render.
pub struct RenderOutput {
    /// Header + body, ready to write.
    pub content: String,
    /// `sha256:…` of the context that produced it.
    pub context_hash: String,
    /// Where the base template came from.
    pub template_source: String,
    /// Rendered profile guidance (may be empty).
    pub profile_guidance: String,
}

/// The serializable model exposed to templates.
#[derive(Serialize)]
struct RenderModel<'a> {
    agent: &'a str,
    profile: &'a str,
    profile_guidance: &'a str,
    context: &'a Context,
}

/// Render an overlay for `req`.
pub fn render(req: &RenderRequest) -> crate::Result<RenderOutput> {
    let renderer = MinijinjaRenderer::default();
    let profile_name = req.selection.profile.name.as_str();

    // 1. Resolve the base template (the override of template_name, if any).
    let template_name = req
        .selection
        .profile
        .template
        .as_deref()
        .unwrap_or(req.template_name);
    let base = templates::resolve(&req.context.repo_base, template_name)?;

    // 2. Render profile guidance (inline string, or profiles/<name>.md.j2 file).
    let profile_guidance =
        render_profile_guidance(&renderer, req.context, req.selection, req.agent)?;

    // 3. Context hash.
    let context_hash = req.context.compute_hash();

    // 4. Header.
    let sources: Vec<String> = req
        .config
        .sources
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let header = header::build(&header::HeaderMeta {
        generated_at: &req.generated_at,
        host: &req.context.system.hostname,
        agent: req.agent,
        profile: profile_name,
        context_hash: &context_hash,
        template_source: &base.source,
        sources: &sources,
    });

    // 5. Body.
    let model = RenderModel {
        agent: req.agent,
        profile: profile_name,
        profile_guidance: &profile_guidance,
        context: req.context,
    };
    let model_value = Value::from_serialize(&model);
    let body = renderer.render_str(&base.content, &model_value)?;

    Ok(RenderOutput {
        content: format!("{header}{body}"),
        context_hash,
        template_source: base.source,
        profile_guidance,
    })
}

/// Resolve + render the profile guidance. Inline config guidance wins; otherwise
/// a `profiles/<name>.md.j2` file under repo/global templates is used.
fn render_profile_guidance(
    renderer: &MinijinjaRenderer,
    ctx: &Context,
    selection: &Selection,
    agent: &str,
) -> crate::Result<String> {
    // Precedence: a `profiles/<name>.md.j2` template file (repo, then global)
    // wins over the inline `guidance` string — so dropping a file overrides
    // both built-in and user-config inline guidance. Falls back to inline.
    let raw = read_profile_template(&ctx.repo_base, &selection.profile.name)
        .or_else(|| selection.profile.guidance.clone());

    let Some(raw) = raw else {
        return Ok(String::new());
    };

    // Render guidance with the same context, but no nested profile_guidance.
    let model = RenderModel {
        agent,
        profile: &selection.profile.name,
        profile_guidance: "",
        context: ctx,
    };
    let value = Value::from_serialize(&model);
    renderer.render_str(&raw, &value)
}

fn read_profile_template(repo_base: &Path, profile: &str) -> Option<String> {
    let file = format!("profiles/{profile}.md.j2");
    let repo = config::repo_templates_dir(repo_base).join(&file);
    if let Ok(s) = std::fs::read_to_string(&repo) {
        return Some(s);
    }
    if let Some(global) = config::global_templates_dir() {
        if let Ok(s) = std::fs::read_to_string(global.join(&file)) {
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::sample_context;
    use crate::profile::ProfileConfig;

    fn selection(name: &str, guidance: Option<&str>) -> Selection {
        Selection {
            profile: ProfileConfig {
                name: name.to_string(),
                when: vec![],
                priority: 0,
                template: None,
                guidance: guidance.map(String::from),
            },
            reasons: vec!["fallback".into()],
        }
    }

    #[test]
    fn renders_header_and_body() {
        let mut ctx = sample_context();
        ctx.stacks = vec!["rust".into()];
        ctx.languages = vec!["Rust".into()];
        ctx.package_managers = vec!["cargo".into()];
        ctx.commands.test = vec!["cargo test".into()];
        let cfg = Config::defaults();
        let sel = selection(
            "rust",
            Some("Use cargo for **{{ context.stacks | join(\",\") }}**."),
        );

        let out = render(&RenderRequest {
            agent: "claude",
            template_name: "claude",
            context: &ctx,
            selection: &sel,
            config: &cfg,
            generated_at: "2026-05-29T00:00:00Z".into(),
        })
        .unwrap();

        assert!(out.content.starts_with(header::GENERATED_MARKER));
        assert!(out.content.contains("profile   : rust"));
        assert!(out.content.contains("Stack:** rust"));
        assert!(out.content.contains("`cargo test`"));
        // Guidance template was itself rendered against the context.
        assert!(out.content.contains("Use cargo for **rust**."));
        assert!(out.context_hash.starts_with("sha256:"));
    }

    #[test]
    fn profile_template_file_overrides_inline_guidance() {
        let d = tempfile::tempdir().unwrap();
        let pdir = config::repo_templates_dir(d.path()).join("profiles");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(pdir.join("rust.md.j2"), "FILE GUIDANCE for {{ profile }}").unwrap();

        let mut ctx = sample_context();
        ctx.repo_base = d.path().to_path_buf();
        ctx.cwd = d.path().to_path_buf();
        let cfg = Config::defaults();
        // Inline guidance is present but the file must win.
        let sel = selection("rust", Some("INLINE GUIDANCE"));

        let out = render(&RenderRequest {
            agent: "claude",
            template_name: "claude",
            context: &ctx,
            selection: &sel,
            config: &cfg,
            generated_at: "2026-05-29T00:00:00Z".into(),
        })
        .unwrap();

        assert!(out.content.contains("FILE GUIDANCE for rust"));
        assert!(!out.content.contains("INLINE GUIDANCE"));
    }

    #[test]
    fn empty_guidance_renders_no_guidance_section() {
        let ctx = sample_context();
        let cfg = Config::defaults();
        let sel = selection("default", None);
        let out = render(&RenderRequest {
            agent: "generic",
            template_name: "generic",
            context: &ctx,
            selection: &sel,
            config: &cfg,
            generated_at: "2026-05-29T00:00:00Z".into(),
        })
        .unwrap();
        assert!(!out.content.contains("Profile guidance —"));
    }

    #[test]
    fn missing_optional_git_does_not_error() {
        let mut ctx = sample_context();
        ctx.git = None; // exercise lenient undefined handling
        let cfg = Config::defaults();
        let sel = selection("default", None);
        let out = render(&RenderRequest {
            agent: "claude",
            template_name: "claude",
            context: &ctx,
            selection: &sel,
            config: &cfg,
            generated_at: "2026-05-29T00:00:00Z".into(),
        })
        .unwrap();
        assert!(out.content.contains("agent context"));
    }
}
