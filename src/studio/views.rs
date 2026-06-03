//! `maud` server-rendered HTML: the page shell and the htmx-swappable fragments
//! (library list, overlay preview, fs-status). No client framework — a tiny
//! embedded JS shim drives fragment swaps from `hx-*` attributes.

use std::path::Path;

use maud::{html, Markup, DOCTYPE};

use crate::context::Scope;
use crate::studio::state::{CapView, LibraryView, PreviewOutcome, ProfileView, Simulated};

/// Coarse language/platform options offered in the simulator.
const LANGS: &[&str] = &["rust", "node", "nextjs", "go", "python", "android", "java"];

/// The full page: top-bar simulator, left library, center placeholder, right
/// live overlay preview. Updates happen via htmx fragment swaps.
pub fn shell(
    lib: &LibraryView,
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
                    nav class="pane nav" { (library(lib)) }
                    section class="pane center" {
                        p class="hint" {
                            "Read-only preview. Pick a context in the simulator and watch the "
                            "overlay update on the right. Editing arrives in the next slice."
                        }
                    }
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

/// The left-pane library (served standalone by `GET /library` too).
pub fn library(lib: &LibraryView) -> Markup {
    html! {
        div class="library" {
            h2 { "Capabilities" }
            div class="section-label" { "YOURS" }
            @if lib.yours.is_empty() {
                p class="muted" { "(none yet)" }
            }
            @for c in &lib.yours { (cap_row(c)) }
            div class="section-label" { "PALETTE" }
            @for c in &lib.palette { (cap_row(c)) }
            h2 { "Profiles" }
            @if lib.profiles.is_empty() {
                p class="muted" { "(none yet)" }
            }
            @for p in &lib.profiles { (profile_row(p)) }
        }
    }
}

/// `GET /library` fragment.
pub fn library_fragment(lib: &LibraryView) -> String {
    library(lib).into_string()
}

fn cap_row(c: &CapView) -> Markup {
    html! {
        div class="cap-row" {
            span class="mark" { (if c.active { "●" } else { "○" }) }
            span class="cap-id" { (c.id) }
            span class="cap-title muted" { (c.title) }
            @if c.kind != "static" { span class="badge" { (c.kind) } }
        }
    }
}

fn profile_row(p: &ProfileView) -> Markup {
    html! {
        div class="profile-row" {
            span class="mark" {
                @if p.selected { "→" } @else if p.candidate { "·" } @else { " " }
            }
            span class="prof-name" { (p.name) }
            span class="prof-targets muted" { "targets [" (p.targets.join(", ")) "]" }
            @if !p.capabilities.is_empty() {
                span class="prof-caps muted" { " · " (p.capabilities.join(", ")) }
            }
        }
    }
}

/// The right-pane overlay preview.
pub fn preview_pane(p: &PreviewOutcome) -> Markup {
    html! {
        div class="overlay" {
            div class="overlay-head" {
                span { "Live overlay · " (p.agent) }
                span class="profile-label" { "profile " (p.profile_label) }
            }
            @if let Some(note) = &p.note {
                p class="note" { (note) }
            }
            pre class="overlay-body" { (p.overlay) }
            p class="updates" { "⟳ updates as you change the simulator (ReadOnly — probes not executed)" }
        }
    }
}

/// `POST /preview` fragment (swapped into `#overlay-pane`).
pub fn preview_fragment(p: &PreviewOutcome) -> String {
    preview_pane(p).into_string()
}

/// An inline error fragment (minijinja/config errors never 500).
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
            "⚠ config changed on disk: " (names.join(", "))
            " — reload before applying."
        }
    }
    .into_string()
}

fn display_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}
