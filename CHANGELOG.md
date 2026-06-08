# Changelog

All notable changes to rosita are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions aim for
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

`dist` pulls the section matching a tagged version into that release's notes, so
keep entries user-facing. When cutting a release, rename **Unreleased** to the
version and date (see [RELEASING.md](RELEASING.md)).

## 0.2.0 — 2026-06-08

### Added

- **Targets** in `rosita studio`: a Targets tab listing every detection target
  and the rule that powers it, plus a way to author your own. Custom targets can
  be declarative (file exists — with `*` globs — file contains, and any/all
  combinations) or a **script predicate** that rosita runs safely (in the repo,
  with a timeout, results cached). Custom targets feed profile selection exactly
  like the built-ins.
- Built-in detection for **Java** (Maven/Gradle), **Ruby**, **PHP**, **Swift**,
  and **.NET**, alongside the existing Rust/Node/Next.js/Go/Python.
- An **arrow-key profile chooser** for `rosita run`: when several profiles match,
  pick with ↑/↓ and Enter (number keys still work; Ctrl-C aborts the run). Falls
  back to a numbered prompt when the terminal isn't interactive.
- A profile that declares **no `targets` is the catch-all default** — it applies
  wherever nothing more specific matches. When nothing matches at all, `run` and
  `render` now report what was detected and how to fix it.
- Live machine grounding in the **everyday** starter pack — real host and runtime
  facts refreshed at launch, not a hand-typed snapshot.

### Changed

- `rosita run` no longer offers an opt-out. When 2+ profiles match it lists
  **only the matching profiles** — invoking rosita means you want one of them.
- Relicensed to **MIT-only** (previously MIT OR Apache-2.0).
- Removed the per-project "bound" badge from studio (it was noise).

### Fixed

- Running rosita **outside a repo** (e.g. from `$HOME`) no longer writes a managed
  importer that bleeds a stale machine-context block into every repo beneath that
  directory. Off-repo context now reaches Claude via `--append-system-prompt`.
- A legacy remembered opt-out (`[binding] none = true` from an older rosita) is
  now **ignored** rather than honored, so a project stuck on "none" re-prompts
  for a profile instead of silently rendering an empty overlay.

## 0.1.0 — 2026-06-08

First tagged release.

### Added

- `rosita studio` guided first-run: lands on the **Profiles** tab and walks a
  three-step onboarding — welcome (detect stack + pick a starter pack) → review
  what will change → "you're set" (names `rosita run <agent>`). A top-bar **?**
  button re-opens the tour anytime.
- Release pipeline via [`dist`](https://opensource.axo.dev/cargo-dist/): tagging
  `vX.Y.Z` builds prebuilt binaries for macOS (Apple Silicon + Intel) and Linux
  (x86_64 + ARM64), with a shell installer attached to the GitHub Release.
  (Windows is omitted for now — rosita is unix-only today.)
- CI workflow: rustfmt, clippy (`-D warnings`), the test suite on Linux + macOS,
  and an MSRV (1.85) check.
