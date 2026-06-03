//! `maud` server-rendered HTML: the page shell and the htmx-swappable fragments
//! (library control surface, edit forms, diff/review, overlay preview). No
//! client framework — a tiny embedded JS shim drives fragment swaps from `hx-*`
//! attributes (and re-binds swapped-in content).
//!
//! Pane convention: `#library` (left) is the control surface (New/Review/Apply +
//! lists), `#center` is the work area (forms, diff), `#overlay-pane` (right) is
//! the live preview. Mutations target `#center` and return a result fragment
//! whose `hx-trigger="load"` refreshers re-pull `#library` + the preview.

use std::path::Path;

use maud::{html, Markup, DOCTYPE};

use crate::capability::{Capability, Layer, Risk};
use crate::context::Scope;
use crate::profile::ProfileConfig;
use crate::studio::edit::FileDiff;
use crate::studio::state::{CapView, LibraryView, PreviewOutcome, ProfileView, Simulated};

/// Coarse language/platform options offered in the simulator and as profile targets.
const LANGS: &[&str] = &["rust", "node", "nextjs", "go", "python", "android", "java"];
const TARGETS: &[&str] = &[
    "rust", "node", "nextjs", "go", "python", "android", "java", "machine",
];

/// The full page: top-bar simulator, left control surface, center work area,
/// right live overlay preview.
pub fn shell(
    lib: &LibraryView,
    staged: usize,
    sim: &Simulated,
    agents: &[String],
    preview: &PreviewOutcome,
) -> String {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "rosita studio" }
                link rel="stylesheet" href="/assets/studio.css";
                script src="/assets/studio.js" defer {}
            }
            body {
                header class="topbar" {
                    span class="brand" { "rosita studio" }
                    (simulator_bar(sim, agents))
                }
                main class="layout" {
                    nav class="pane nav" id="library" { (library(lib, staged, None, false)) }
                    section class="pane center" id="center" { (center_placeholder()) }
                    aside class="pane preview" id="overlay-pane" { (preview_pane(preview)) }
                }
            }
        }
    }
    .into_string()
}

fn simulator_bar(sim: &Simulated, agents: &[String]) -> Markup {
    html! {
        form class="simulator" hx-post="/preview" hx-target="#overlay-pane" hx-trigger="change" {
            label { "lang "
                select name="lang" {
                    option value="" selected[sim.lang.is_none()] { "(detected)" }
                    @for &l in LANGS {
                        option value=(l) selected[sim.lang.as_deref() == Some(l)] { (l) }
                    }
                }
            }
            label { "scope "
                select name="scope" {
                    option value="" selected[sim.scope.is_none()] { "(detected)" }
                    option value="repo" selected[matches!(sim.scope, Some(Scope::Repo))] { "repo" }
                    option value="machine" selected[matches!(sim.scope, Some(Scope::Machine))] { "machine" }
                }
            }
            label { "agent "
                select name="agent" {
                    @for a in agents {
                        option value=(a.as_str()) selected[&sim.agent == a] { (a.as_str()) }
                    }
                }
            }
        }
    }
}

/// The center work area's idle placeholder.
pub fn center_placeholder() -> Markup {
    html! {
        p class="hint" {
            "Pick a capability or profile to edit, or create one. Stage changes, "
            "review the exact TOML diff, then apply. The overlay on the right updates live."
        }
    }
}

/// `GET /welcome` — reset the center pane to its placeholder.
pub fn center_placeholder_fragment() -> String {
    center_placeholder().into_string()
}

// --- left control surface ----------------------------------------------------

/// The left pane: New buttons, the staged-changes control, and the library
/// lists. `flash` shows a transient note; `refresh_preview` injects a one-shot
/// loader that re-pulls the preview after this fragment is swapped in.
pub fn library(
    lib: &LibraryView,
    staged: usize,
    flash: Option<&str>,
    refresh_preview: bool,
) -> Markup {
    html! {
        div class="library" {
            @if let Some(msg) = flash { p class="flash" { (msg) } }
            div class="actions" {
                button hx-get="/capabilities/new" hx-target="#center" { "＋ New cap" }
                button hx-get="/profiles/new" hx-target="#center" { "＋ New profile" }
            }
            div class="staged" {
                @if staged > 0 {
                    span class="staged-count" { "◍ " (staged) " staged" }
                    button hx-get="/diff" hx-target="#center" { "Review" }
                    button class="apply"
                        hx-post="/apply" hx-target="#center"
                        hx-confirm="Apply staged changes to your config files?" { "Apply " (staged) }
                } @else {
                    span class="muted" { "no staged changes" }
                }
            }
            h2 { "Capabilities" }
            div class="section-label" { "YOURS" }
            @if lib.yours.is_empty() { p class="muted" { "(none yet)" } }
            @for c in &lib.yours { (cap_row(c, true)) }
            div class="section-label" { "PALETTE" }
            @for c in &lib.palette { (cap_row(c, false)) }
            h2 { "Profiles" }
            @if lib.profiles.is_empty() { p class="muted" { "(none yet)" } }
            @for p in &lib.profiles { (profile_row(p)) }
            @if refresh_preview {
                div hx-post="/preview" hx-trigger="load" hx-target="#overlay-pane" {}
            }
        }
    }
}

/// `GET /library` fragment (no preview refresh — used as a load-triggered pull).
pub fn library_fragment(lib: &LibraryView, staged: usize) -> String {
    library(lib, staged, None, false).into_string()
}

fn cap_row(c: &CapView, owned: bool) -> Markup {
    let id = c.id.as_str();
    let e = enc(id);
    html! {
        div class="cap-row" {
            span class="mark" { (if c.active { "●" } else { "○" }) }
            span class="cap-id" { (id) }
            span class="cap-title muted" { (c.title) }
            @if c.kind != "static" { span class="badge" { (c.kind) } }
            span class="row-actions" {
                @if owned {
                    button hx-get=(format!("/capabilities/{e}/edit")) hx-target="#center" { "edit" }
                    button class="danger"
                        hx-delete=(format!("/capabilities/{e}")) hx-target="#center"
                        hx-confirm=(format!("Stage deletion of capability \"{id}\"?")) { "✕" }
                } @else {
                    button hx-post=(format!("/capabilities/{e}/duplicate")) hx-target="#center"
                        title="duplicate into your config to own it" { "⎘" }
                }
            }
        }
    }
}

fn profile_row(p: &ProfileView) -> Markup {
    let name = p.name.as_str();
    let e = enc(name);
    html! {
        div class="profile-row" {
            span class="mark" {
                @if p.selected { "→" } @else if p.candidate { "·" } @else { " " }
            }
            span class="prof-name" { (name) }
            span class="prof-targets muted" { "targets [" (p.targets.join(", ")) "]" }
            span class="row-actions" {
                button hx-get=(format!("/profiles/{e}/edit")) hx-target="#center" { "edit" }
                button class="danger"
                    hx-delete=(format!("/profiles/{e}")) hx-target="#center"
                    hx-confirm=(format!("Stage deletion of profile \"{name}\"?")) { "✕" }
            }
        }
    }
}

// --- editor forms ------------------------------------------------------------

fn layer_scope(layer: Layer) -> (&'static str, bool) {
    match layer {
        Layer::Global => ("global", false),
        Layer::GlobalLocal => ("global", true),
        Layer::RepoLocal => ("repo", true),
        _ => ("repo", false),
    }
}

fn lives_in(layer: Layer) -> Markup {
    let (scope, private) = layer_scope(layer);
    html! {
        fieldset class="lives-in" {
            legend { "lives in" }
            label { input type="radio" name="scope" value="repo" checked[scope == "repo"]; " repo" }
            label { input type="radio" name="scope" value="global" checked[scope == "global"]; " global" }
            " · "
            label { input type="radio" name="visibility" value="public" checked[!private]; " public" }
            label { input type="radio" name="visibility" value="private" checked[private]; " private" }
        }
    }
}

/// The capability editor. `cap` populates an edit; `None` is a new capability.
/// A palette item (`owned == false`) renders read-only with a duplicate action.
pub fn capability_form(cap: Option<&Capability>, layer: Layer, owned: bool) -> String {
    let is_new = cap.is_none();
    let id = cap.map(|c| c.id.as_str()).unwrap_or("");
    let read_only = !is_new && !owned;
    html! {
        @if read_only {
            div class="form" {
                h3 { "Palette capability — read-only" }
                p class="muted" { "Palette items are templates. Duplicate “" (id) "” into your config to own and edit it." }
                button hx-post=(format!("/capabilities/{}/duplicate", enc(id))) hx-target="#center" { "⎘ Duplicate into my config" }
            }
        } @else {
            form class="form" hx-post="/capabilities" hx-target="#center" {
                h3 { (if is_new { "New capability" } else { "Edit capability" }) }
                label { "id"
                    @if is_new {
                        input type="text" name="id" value="" placeholder="rust-conventions" required;
                    } @else {
                        input type="text" name="id" value=(id) readonly;
                    }
                }
                label { "description" input type="text" name="description" value=(cap.and_then(|c| c.description.as_deref()).unwrap_or("")); }
                label { "tags (comma-separated)" input type="text" name="tags" value=(cap.map(|c| c.tags.join(", ")).unwrap_or_default()); }
                fieldset class="risk" {
                    legend { "risk" }
                    @let risk = cap.map(|c| c.risk).unwrap_or(Risk::Info);
                    label { input type="radio" name="risk" value="info" checked[risk == Risk::Info]; " info" }
                    label { input type="radio" name="risk" value="caution" checked[risk == Risk::Caution]; " caution" }
                    label { input type="radio" name="risk" value="dangerous" checked[risk == Risk::Dangerous]; " dangerous" }
                }
                label { "provider (dynamic; built-in probe)" input type="text" name="provider" value=(cap.and_then(|c| c.provider.as_deref()).unwrap_or("")) placeholder="host | docker | …"; }
                label { "command (dynamic; trust-gated in a repo)" input type="text" name="command" value=(cap.and_then(|c| c.command.as_deref()).unwrap_or("")); }
                label { "cache TTL" input type="text" name="cache" value=(cap.and_then(|c| c.cache.as_deref()).unwrap_or("")) placeholder="60s"; }
                label { "requires (comma-separated capability ids)" input type="text" name="requires" value=(cap.map(|c| c.requires.join(", ")).unwrap_or_default()); }
                label { "agents (comma-separated; empty = all)" input type="text" name="agents" value=(cap.map(|c| c.agents.join(", ")).unwrap_or_default()); }
                label { "guidance (markdown / minijinja)"
                    textarea name="guidance" rows="6" { (cap.map(|c| c.guidance.as_str()).unwrap_or("")) }
                }
                (lives_in(layer))
                div class="form-buttons" {
                    button type="button" hx-get="/welcome" hx-target="#center" { "Discard" }
                    button type="submit" { "Stage change" }
                }
            }
        }
    }
    .into_string()
}

/// The profile composer. `profile` populates an edit; `None` is a new profile.
/// `available` is every capability id you can compose (yours + palette).
pub fn profile_form(profile: Option<&ProfileConfig>, available: &[String]) -> String {
    let is_new = profile.is_none();
    let name = profile.map(|p| p.name.as_str()).unwrap_or("");
    let targets: Vec<&str> = profile
        .map(|p| p.targets.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let chosen: Vec<&str> = profile
        .map(|p| p.capabilities.iter().map(|r| r.id()).collect())
        .unwrap_or_default();
    html! {
        form class="form" hx-post="/profiles" hx-target="#center" {
            h3 { (if is_new { "New profile" } else { "Edit profile" }) }
            label { "name"
                @if is_new {
                    input type="text" name="name" value="" placeholder="rust — browser" required;
                } @else {
                    input type="text" name="name" value=(name) readonly;
                }
            }
            fieldset class="targets" {
                legend { "targets (language/platform tie)" }
                @for &t in TARGETS {
                    label { input type="checkbox" name="targets" value=(t) checked[targets.contains(&t)]; " " (t) }
                }
            }
            fieldset class="cap-picker" {
                legend { "capabilities (need ≥1 to save)" }
                @if available.is_empty() { p class="muted" { "(no capabilities yet — create one first)" } }
                @for id in available {
                    label { input type="checkbox" name="capabilities" value=(id.as_str()) checked[chosen.contains(&id.as_str())]; " " (id) }
                }
            }
            label { "inline guidance (optional)"
                textarea name="guidance" rows="3" { (profile.and_then(|p| p.guidance.as_deref()).unwrap_or("")) }
            }
            fieldset class="lives-in" {
                legend { "lives in" }
                label { input type="radio" name="scope" value="repo" checked; " repo" }
                label { input type="radio" name="scope" value="global"; " global" }
            }
            div class="form-buttons" {
                button type="button" hx-get="/welcome" hx-target="#center" { "Discard" }
                button type="submit" { "Stage change" }
            }
        }
    }
    .into_string()
}

// --- diff / review -----------------------------------------------------------

/// Trust state to surface in the review when repo-authored `command` caps exist.
pub struct TrustBanner {
    /// Repo-layer command-backed capability ids (need trust to run).
    pub command_caps: Vec<String>,
    /// `trusted` / `stale (…)` / `untrusted`.
    pub status: String,
    pub trusted: bool,
}

/// The review pane: per-file diff vs raw bytes, the leak warning, the
/// external-edit banner, and the trust banner, plus Apply.
pub fn diff_view(
    diffs: &[FileDiff],
    leaks: &[String],
    fs_changed: &[std::path::PathBuf],
    trust: &TrustBanner,
    staged: usize,
) -> String {
    html! {
        div class="review" {
            h3 { "Review staged changes (" (staged) ")" }

            @if !trust.command_caps.is_empty() {
                div class="trust-banner" {
                    "⚠ repo command capabilities (" (trust.command_caps.join(", ")) ") won't run until you trust this repo — currently "
                    span class="trust-status" { (trust.status) } "."
                    @if !trust.trusted {
                        button hx-post="/trust/allow" hx-target="#center"
                            hx-confirm="Trust this repo to run its command-backed capabilities?" { "Allow this repo" }
                    } @else {
                        button class="danger" hx-post="/trust/deny" hx-target="#center" { "Revoke trust" }
                    }
                    p class="muted" { "An apply changes the repo config bundle, which re-locks trust — re-allow afterward." }
                }
            }

            @if !leaks.is_empty() {
                div class="leak-warn" {
                    "⚠ leak check: these public values look machine-specific — consider moving to local.toml: "
                    (leaks.join(", "))
                }
            } @else {
                p class="muted" { "⚠ leak check: clean." }
            }

            @if !fs_changed.is_empty() {
                div class="fs-changed" {
                    "⚠ config changed on disk since load ("
                    (fs_changed.iter().map(|p| display_name(p)).collect::<Vec<_>>().join(", "))
                    ") — applying will overwrite it."
                }
            }

            @if diffs.is_empty() {
                p class="muted" { "No staged changes." }
            } @else {
                @for d in diffs { (file_diff(d)) }
                div class="form-buttons" {
                    button type="button" hx-get="/welcome" hx-target="#center" { "Cancel" }
                    button class="apply" hx-post="/apply" hx-target="#center"
                        hx-confirm="Apply staged changes to your config files?" { "Apply " (staged) " change(s)" }
                }
            }
        }
    }
    .into_string()
}

fn file_diff(d: &FileDiff) -> Markup {
    let (scope, private) = layer_scope(d.layer);
    let vis = if private { "private" } else { "public" };
    html! {
        div class="file-diff" {
            div class="file-head" {
                span class="file-path" { (display_name(&d.path)) }
                span class="muted" { " " (scope) " · " (vis) }
            }
            @if d.reformats_untouched {
                p class="muted" { "ⓘ rosita will also reformat some untouched lines it parsed." }
            }
            pre class="diff-body" { (d.unified) }
        }
    }
}

// --- preview -----------------------------------------------------------------

pub fn preview_pane(p: &PreviewOutcome) -> Markup {
    html! {
        div class="overlay" {
            div class="overlay-head" {
                span { "Live overlay · " (p.agent) }
                span class="profile-label" { "profile " (p.profile_label) }
            }
            @if let Some(note) = &p.note { p class="note" { (note) } }
            pre class="overlay-body" { (p.overlay) }
            p class="updates" { "⟳ updates as you edit (ReadOnly — probes not executed)" }
        }
    }
}

pub fn preview_fragment(p: &PreviewOutcome) -> String {
    preview_pane(p).into_string()
}

// --- small result / error fragments ------------------------------------------

/// A mutation result swapped into `#center`: a note plus one-shot loaders that
/// refresh the library control surface and the live preview.
pub fn action_result(msg: &str) -> String {
    html! {
        div class="result" {
            p { (msg) }
            div hx-get="/library" hx-trigger="load" hx-target="#library" {}
            div hx-post="/preview" hx-trigger="load" hx-target="#overlay-pane" {}
        }
    }
    .into_string()
}

/// An inline error fragment (validation / minijinja / config errors never 500).
pub fn error_fragment(msg: &str) -> String {
    html! { div class="error" { "⚠ " (msg) } }.into_string()
}

/// A minimal full-page error (when the shell itself can't be assembled).
pub fn error_page(msg: &str) -> String {
    html! {
        (DOCTYPE)
        html { head { title { "rosita studio — error" } } body { pre class="error" { (msg) } } }
    }
    .into_string()
}

/// `GET /fs-status` fragment — the light external-edit poll banner.
pub fn fs_status_fragment(changed: &[std::path::PathBuf]) -> String {
    if changed.is_empty() {
        return html! { span class="fs-clean" { "on-disk unchanged since load" } }.into_string();
    }
    let names: Vec<String> = changed.iter().map(|p| display_name(p)).collect();
    html! {
        div class="fs-changed" {
            "⚠ config changed on disk: " (names.join(", ")) " — reload before applying."
        }
    }
    .into_string()
}

fn display_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Percent-encode a path segment (profile names can contain spaces / em-dashes).
/// The server decodes with `state::percent_decode`; we never emit a bare `+`.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
