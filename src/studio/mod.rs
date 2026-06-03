//! `rosita studio` — a local, ephemeral web UI for viewing and editing
//! capabilities and profiles as a lens over your plain TOML config.
//!
//! - [`edit`] — the headless comment/format-preserving `toml_edit` write engine
//!   (Slice 0): `Session`/`StagedOp`/diff/apply.
//! - [`state`] — session state + the read-only model computations (selection,
//!   ReadOnly overlay preview, library snapshot), socket-free for testing.
//! - [`server`] — the `tiny_http` spine: bind 127.0.0.1, bootstrap-token +
//!   Host/Origin/cookie guards, and the `(method, path)` router ([`serve`]).
//! - [`views`] — `maud` server-rendered HTML (shell + htmx fragments).
//! - [`assets`] — CSS + the htmx-shim JS, embedded via `rust-embed`.
//!
//! Slice 1 is **read-only**: `GET /` shell, `GET /library`, `POST /preview`
//! (ReadOnly render), `GET /fs-status`, `GET /assets/*`. The write routes
//! (editor, composer, diff/apply, trust) arrive in Slice 2. See
//! `docs/studio-design.md`.

pub mod assets;
pub mod edit;
pub mod server;
pub mod state;
pub mod views;

pub use edit::{FileDiff, Session, StagedOp};
pub use server::serve;
