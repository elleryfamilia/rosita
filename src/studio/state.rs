//! Studio session state + the read-only "model" computations (selection,
//! ReadOnly overlay preview, the library snapshot) that the HTTP handlers and
//! views render. Kept free of `tiny_http` so it's unit-testable without a socket.
//!
//! Concurrency rule (design §2): handlers take a cheap [`Snapshot`] under the
//! session mutex, release it, then assemble/render **outside** the lock — never
//! hold the mutex across rendering, disk I/O, or probe execution.

use std::path::PathBuf;

use crate::adapters;
use crate::capability::{palette, Layer};
use crate::config::Config;
use crate::context::{Context, GitContext, Scope};
use crate::dynamic::DynamicMode;
use crate::profile::{self, Composition, Selection};
use crate::render::{self, RenderRequest};
use crate::studio::edit::Session;

/// The simulated context the preview is rendered for. Each field overrides the
/// real detected context; `None`/empty means "use what was detected".
#[derive(Debug, Clone)]
pub struct Simulated {
    /// Target agent id to render for.
    pub agent: String,
    /// Override the detected stack/language (empty ⇒ no stack).
    pub lang: Option<String>,
    /// Override repo-vs-machine scope.
    pub scope: Option<Scope>,
}

impl Simulated {
    /// Update the simulator from a posted urlencoded form (`lang`/`scope`/`agent`).
    /// Unrecognized/blank values reset to "use detected".
    pub fn update_from_form(&mut self, body: &str) {
        for (k, v) in parse_pairs(body) {
            match k.as_str() {
                "agent" if !v.is_empty() => self.agent = v,
                "lang" => self.lang = if v.is_empty() { None } else { Some(v) },
                "scope" => {
                    self.scope = match v.as_str() {
                        "repo" => Some(Scope::Repo),
                        "machine" => Some(Scope::Machine),
                        _ => None,
                    }
                }
                _ => {}
            }
        }
    }
}

/// A studio editing/viewing session: the edit engine + the detected context +
/// the simulator + the security token/port. Lives behind an `Arc<Mutex<…>>`.
pub struct StudioState {
    /// The comment-preserving edit engine over the writable layers.
    pub session: Session,
    /// The real detected context (the simulator overrides a clone of this).
    pub base_context: Context,
    /// Repo base (git root or cwd).
    pub repo_base: PathBuf,
    /// The simulated context the preview reflects.
    pub sim: Simulated,
    /// Per-session CSRF/session token (also the bootstrap-cookie value).
    pub token: String,
    /// Bound port (for Host/Origin checks).
    pub port: u16,
}

impl StudioState {
    /// A cheap, owned copy of everything the read-only handlers need, taken under
    /// the mutex so rendering can happen after the lock is released.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            base_context: self.base_context.clone(),
            sim: self.sim.clone(),
            layer_texts: self.session.staged_layer_texts(),
        }
    }
}

/// An owned, lock-free snapshot for rendering a view.
pub struct Snapshot {
    pub base_context: Context,
    pub sim: Simulated,
    pub layer_texts: Vec<(Layer, PathBuf, String)>,
}

/// The result of a ReadOnly preview render.
pub struct PreviewOutcome {
    /// Agent the overlay was rendered for.
    pub agent: String,
    /// Selected profile label (`none` when no profile applies).
    pub profile_label: String,
    /// The rendered overlay markdown (header + body).
    pub overlay: String,
    /// A human note when there's no single profile (empty / ambiguous).
    pub note: Option<String>,
}

/// One capability row for the library view.
pub struct CapView {
    pub id: String,
    pub title: String,
    pub kind: &'static str,
    /// Composed into the current preview overlay.
    pub active: bool,
}

/// One profile row for the library view.
pub struct ProfileView {
    pub name: String,
    pub targets: Vec<String>,
    pub selected: bool,
    pub candidate: bool,
    pub capabilities: Vec<String>,
}

/// The whole left-pane library snapshot for a context.
pub struct LibraryView {
    pub yours: Vec<CapView>,
    pub palette: Vec<CapView>,
    pub profiles: Vec<ProfileView>,
}

/// Assemble the staged config (origin-tagged) from a snapshot.
pub fn staged_config(snap: &Snapshot) -> crate::Result<Config> {
    Config::from_layer_strs(snap.layer_texts.clone())
}

/// Apply the simulator overrides to the detected context.
pub fn simulated_context(base: &Context, sim: &Simulated) -> Context {
    let mut ctx = base.clone();
    if let Some(lang) = &sim.lang {
        ctx.stacks = if lang.is_empty() {
            vec![]
        } else {
            vec![lang.clone()]
        };
    }
    match sim.scope {
        Some(Scope::Machine) => ctx.git = None,
        Some(Scope::Repo) if ctx.git.is_none() => {
            ctx.git = Some(GitContext {
                root: ctx.repo_base.clone(),
                branch: Some("main".to_string()),
                remotes: vec![],
                is_worktree: false,
            });
        }
        _ => {}
    }
    ctx
}

/// Select the profile for `(cfg, ctx)` honoring the on-disk binding.
pub fn select_for(cfg: &Config, ctx: &Context) -> Selection {
    let binding = crate::binding::read(ctx);
    profile::select(ctx, &cfg.profiles, binding.as_ref())
}

/// Render the overlay for the snapshot's simulated context in **ReadOnly** mode
/// (never executes providers/commands). Selection drives which profile (if any)
/// is composed.
pub fn render_preview(snap: &Snapshot) -> crate::Result<PreviewOutcome> {
    let cfg = staged_config(snap)?;
    let ctx = simulated_context(&snap.base_context, &snap.sim);
    let selection = select_for(&cfg, &ctx);

    let (composition, note) = match &selection {
        Selection::Use(p) => (
            profile::compose_profile(&ctx, p, &cfg.capabilities, &cfg.capability_params),
            None,
        ),
        Selection::None => (
            Composition::default(),
            Some("No profile applies to this context — the overlay is empty.".to_string()),
        ),
        Selection::Ambiguous(cands) => {
            let names: Vec<&str> = cands.iter().map(|p| p.name.as_str()).collect();
            (
                Composition::default(),
                Some(format!(
                    "{} profiles match ({}). Pick one with `rosita run` to bind it.",
                    cands.len(),
                    names.join(", ")
                )),
            )
        }
    };

    let agent_id = if snap.sim.agent.is_empty() {
        cfg.default_agent.clone()
    } else {
        snap.sim.agent.clone()
    };
    let descriptor = adapters::descriptor(&cfg, &agent_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unknown agent '{agent_id}'"))?;

    let out = render::render(&RenderRequest {
        agent: &descriptor.id,
        template_name: &descriptor.template,
        context: &ctx,
        composition: &composition,
        config: &cfg,
        generated_at: now_rfc3339(),
        dynamic: DynamicMode::ReadOnly,
    })?;

    Ok(PreviewOutcome {
        agent: agent_id,
        profile_label: composition.label().to_string(),
        overlay: out.content,
        note,
    })
}

/// Build the left-pane library view (your caps + the palette + your profiles),
/// marking what's active/selected for the snapshot's simulated context.
pub fn library_view(snap: &Snapshot) -> crate::Result<LibraryView> {
    let cfg = staged_config(snap)?;
    let ctx = simulated_context(&snap.base_context, &snap.sim);
    let selection = select_for(&cfg, &ctx);

    let selected_name = match &selection {
        Selection::Use(p) => Some(p.name.clone()),
        _ => None,
    };
    let active_ids: Vec<String> = match &selection {
        Selection::Use(p) => {
            profile::compose_profile(&ctx, p, &cfg.capabilities, &cfg.capability_params)
                .capabilities
                .iter()
                .map(|rc| rc.capability.id.clone())
                .collect()
        }
        _ => vec![],
    };

    let tags = ctx.selection_targets();
    let yours = cfg
        .capabilities
        .iter()
        .map(|c| CapView {
            kind: kind_of(c.command.is_some(), c.provider.is_some()),
            active: active_ids.contains(&c.id),
            title: c.title().to_string(),
            id: c.id.clone(),
        })
        .collect();
    // Palette items not already owned (by id) in your library.
    let owned: std::collections::HashSet<&str> =
        cfg.capabilities.iter().map(|c| c.id.as_str()).collect();
    let palette = palette()
        .into_iter()
        .filter(|c| !owned.contains(c.id.as_str()))
        .map(|c| CapView {
            kind: kind_of(c.command.is_some(), c.provider.is_some()),
            active: false,
            title: c.title().to_string(),
            id: c.id,
        })
        .collect();
    let profiles = cfg
        .profiles
        .iter()
        .map(|p| ProfileView {
            name: p.name.clone(),
            targets: p.targets.clone(),
            selected: selected_name.as_deref() == Some(p.name.as_str()),
            candidate: profile::profile_matches_targets(p, &tags),
            capabilities: p.capabilities.iter().map(|r| r.id().to_string()).collect(),
        })
        .collect();

    Ok(LibraryView {
        yours,
        palette,
        profiles,
    })
}

fn kind_of(has_command: bool, has_provider: bool) -> &'static str {
    if has_command {
        "command"
    } else if has_provider {
        "provider"
    } else {
        "static"
    }
}

/// Current UTC time as an RFC3339 (`…Z`) string for the rendered header.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Parse a urlencoded `a=b&c=d` body/query into decoded key/value pairs.
pub fn parse_pairs(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// Minimal `application/x-www-form-urlencoded` decode (`+`→space, `%XX`).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pairs_decodes() {
        let got = parse_pairs("lang=rust&agent=claude&q=a%20b");
        assert_eq!(got[0], ("lang".to_string(), "rust".to_string()));
        assert_eq!(got[2], ("q".to_string(), "a b".to_string()));
    }

    #[test]
    fn simulator_form_updates_and_resets() {
        let mut sim = Simulated {
            agent: "claude".into(),
            lang: None,
            scope: None,
        };
        sim.update_from_form("lang=go&scope=machine&agent=codex");
        assert_eq!(sim.lang.as_deref(), Some("go"));
        assert!(matches!(sim.scope, Some(Scope::Machine)));
        assert_eq!(sim.agent, "codex");
        // Blank lang resets to "use detected".
        sim.update_from_form("lang=&scope=");
        assert!(sim.lang.is_none());
        assert!(sim.scope.is_none());
    }
}
