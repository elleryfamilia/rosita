# Extending rosita

Where to plug in. **(implemented)** can be done today; **(planned)** lands with
the [implementation plan](implementation-plan.md).

## Add or change an agent **(implemented — no code)**

Agents are data. Add a `[[agents]]` entry (or override a built-in by `id`) in any
config layer. Minimum: `id` + `generated_filename`. See
[configuration](configuration.md#agents-implemented) for all fields. The single
`adapters::apply()` engine handles rendering, wiring, gitignore, and hash-skip
for any descriptor. The wiring it picks:

- `importer` set → managed `@import` block in that (local) file; gitignore it if
  rosita created it.
- `override_target` set + opted in → merge the overlay into that (gitignored)
  file, seeded from `override_base`.
- otherwise → emit-only: write the gitignored overlay + print `wire_hint`.

To add a genuinely new *delivery mechanism* (beyond import / override / emit),
extend the wiring branch in `adapters::apply()` and add the descriptor field(s).

## Add a context detector **(implemented — code)**

Implement `context::ContextDetector` (`name()` + `detect(&DetectInput, &mut
Context)`), add fields to `context::Context` if needed, and register it in
`context::default_detectors()`. Keep it best-effort: return `Ok(())` and leave
fields unset on failure; never panic. Put pure parsing in testable free
functions (see `context/git.rs::parse_remotes`).

## Add a rule field or operator **(implemented — code)**

In `src/profile.rs`: add a variant to `Field` (and map it in `field_values()`)
or to `Op` (and handle it in `apply()`). Both derive serde with snake_case, so
the TOML surface follows automatically. Add a unit test.

## Customize templates **(implemented — no code)**

Drop `<repo>/.rosita/templates/<name>.md.j2` or
`<global>/templates/<name>.md.j2`. Any name falls back to the embedded
`overlay.md.j2`. The model exposes `context`, `profile`, `profile_guidance`,
`agent` (and, planned, `params` and `provider.output` inside a capability). The
provenance/freshness banner is prepended in Rust (`render/header.rs`), so the
body template stays simple.

## Add a capability **(planned — no code)**

Add a `[[capabilities]]` entry and reference its `id` from a profile's
`capabilities` list. Static capabilities are just templated guidance; dynamic
ones name a `provider` (or, trust permitting, a `command`). See
[configuration](configuration.md#capabilities-planned).

## Add a native provider **(planned — code)**

Implement the `EnvProvider` trait (id + `probe(&Context) -> Result<ProviderOutput>`),
register it in the provider registry, and it becomes usable as
`provider = "<id>"` in a dynamic capability and as a `detect` section. Built-ins
to follow as the template: `tailnet`, `docker`, `toolchain`, `ai-tools`. Probes
must degrade gracefully (missing tool → empty), redact output, and be cacheable.

## Testing conventions

- Pure logic → in-module `#[cfg(test)]` unit tests. Share fixtures via
  `context::test_support::sample_context()`.
- End-to-end → `tests/cli.rs` drives the real binary against temp repos, with
  `ROSITA_CONFIG_DIR` pointed at an empty dir for hermeticity and `git_init()`
  when gitignore behavior is under test.
- Before declaring done: `cargo test`, `cargo clippy --all-targets`,
  `cargo fmt --check`.
