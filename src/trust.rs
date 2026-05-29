//! direnv-style trust for repo-authored commands.
//!
//! A `command`-backed capability defined in a **repo** layer (`.rosita/
//! config.toml` or `.rosita/local.toml`) executes an arbitrary command, so it is
//! refused until the user explicitly trusts the repo with `rosita allow`. Trust
//! is recorded as the sha256 of the repo's `.rosita` config **bundle**
//! (`config.toml` + `local.toml`); any edit to that bundle re-locks trust, so a
//! command can't be slipped in after approval.
//!
//! Commands authored in the **global** layers (and built-in providers) are
//! trusted without `allow` — you authored them. See [`crate::dynamic`] for how
//! the decision is applied at render time.
//!
//! The store lives at `<global config dir>/trust.toml`:
//! ```toml
//! [trusted]
//! "/abs/path/to/repo" = "sha256:…"
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::config;

/// The on-disk trust store: repo path → trusted config-bundle hash.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustStore {
    #[serde(default)]
    trusted: BTreeMap<String, String>,
}

/// Trust state of a repo relative to its current config bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Recorded and the config bundle is unchanged since.
    Trusted,
    /// Recorded, but the config bundle changed since — treated as untrusted.
    Stale,
    /// Never recorded.
    Untrusted,
}

impl Status {
    /// Short label for display.
    pub fn label(self) -> &'static str {
        match self {
            Status::Trusted => "trusted",
            Status::Stale => "stale (config changed since `rosita allow`)",
            Status::Untrusted => "untrusted",
        }
    }
}

/// Path to the trust store, if a global config dir can be resolved.
pub fn store_path() -> Option<PathBuf> {
    config::global_config_dir().map(|d| d.join("trust.toml"))
}

/// sha256 of the repo's `.rosita` config bundle (`config.toml` + `local.toml`).
/// Missing files count as empty, so adding either re-locks trust.
pub fn compute_bundle_hash(repo_base: &Path) -> String {
    let config = std::fs::read_to_string(config::repo_config_path(repo_base)).unwrap_or_default();
    let local = std::fs::read_to_string(config::repo_local_path(repo_base)).unwrap_or_default();
    // Reuse the deterministic JSON-sha256 helper over a stable structure.
    crate::hash::context_hash(&serde_json::json!({ "config": config, "local": local }))
}

// --- global-resolving wrappers ----------------------------------------------

/// Whether `repo_base` is currently trusted (recorded hash matches the bundle).
pub fn is_trusted(repo_base: &Path) -> bool {
    store_path()
        .map(|p| status_at(&p, repo_base) == Status::Trusted)
        .unwrap_or(false)
}

/// Record the repo's current config-bundle hash as trusted.
pub fn allow(repo_base: &Path) -> Result<()> {
    let path = store_path().ok_or_else(|| anyhow!("no global config dir to store trust in"))?;
    allow_at(&path, repo_base)
}

/// Remove a repo's trust entry. Returns whether one existed.
pub fn deny(repo_base: &Path) -> Result<bool> {
    let path = store_path().ok_or_else(|| anyhow!("no global config dir to store trust in"))?;
    deny_at(&path, repo_base)
}

/// The repo's trust [`Status`].
pub fn status(repo_base: &Path) -> Status {
    match store_path() {
        Some(p) => status_at(&p, repo_base),
        None => Status::Untrusted,
    }
}

// --- testable cores (explicit store path) -----------------------------------

fn key(repo_base: &Path) -> String {
    repo_base.display().to_string()
}

fn load(store_path: &Path) -> TrustStore {
    std::fs::read_to_string(store_path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(store_path: &Path, store: &TrustStore) -> Result<()> {
    if let Some(parent) = store_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = toml::to_string(store).context("serializing trust store")?;
    std::fs::write(store_path, text).with_context(|| format!("writing {}", store_path.display()))
}

/// [`status`] against an explicit store path.
pub fn status_at(store_path: &Path, repo_base: &Path) -> Status {
    let store = load(store_path);
    match store.trusted.get(&key(repo_base)) {
        None => Status::Untrusted,
        Some(recorded) if recorded == &compute_bundle_hash(repo_base) => Status::Trusted,
        Some(_) => Status::Stale,
    }
}

/// [`allow`] against an explicit store path.
pub fn allow_at(store_path: &Path, repo_base: &Path) -> Result<()> {
    let mut store = load(store_path);
    store
        .trusted
        .insert(key(repo_base), compute_bundle_hash(repo_base));
    save(store_path, &store)
}

/// [`deny`] against an explicit store path.
pub fn deny_at(store_path: &Path, repo_base: &Path) -> Result<bool> {
    let mut store = load(store_path);
    let existed = store.trusted.remove(&key(repo_base)).is_some();
    if existed {
        save(store_path, &store)?;
    }
    Ok(existed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo dir with a `.rosita/config.toml`, plus an isolated store path.
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(config::repo_dir(repo.path())).unwrap();
        std::fs::write(
            config::repo_config_path(repo.path()),
            "agent = \"claude\"\n",
        )
        .unwrap();
        let store = repo.path().join("store").join("trust.toml");
        let base = repo.path().to_path_buf();
        (repo, base, store)
    }

    #[test]
    fn bundle_hash_changes_with_config() {
        let (_g, base, _s) = fixture();
        let h1 = compute_bundle_hash(&base);
        std::fs::write(config::repo_config_path(&base), "agent = \"codex\"\n").unwrap();
        let h2 = compute_bundle_hash(&base);
        assert_ne!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn allow_then_stale_after_change_then_reallow_then_deny() {
        let (_g, base, store) = fixture();

        // Initially untrusted.
        assert_eq!(status_at(&store, &base), Status::Untrusted);

        // allow → trusted.
        allow_at(&store, &base).unwrap();
        assert_eq!(status_at(&store, &base), Status::Trusted);

        // Editing the config bundle re-locks trust (→ stale).
        std::fs::write(config::repo_config_path(&base), "agent = \"codex\"\n").unwrap();
        assert_eq!(status_at(&store, &base), Status::Stale);

        // Re-allow with the new bundle → trusted again.
        allow_at(&store, &base).unwrap();
        assert_eq!(status_at(&store, &base), Status::Trusted);

        // local.toml is part of the bundle too.
        std::fs::write(config::repo_local_path(&base), "# secrets\n").unwrap();
        assert_eq!(status_at(&store, &base), Status::Stale);
        allow_at(&store, &base).unwrap();
        assert_eq!(status_at(&store, &base), Status::Trusted);

        // deny → untrusted; second deny reports nothing removed.
        assert!(deny_at(&store, &base).unwrap());
        assert_eq!(status_at(&store, &base), Status::Untrusted);
        assert!(!deny_at(&store, &base).unwrap());
    }

    #[test]
    fn trust_is_per_repo_path() {
        let (_g, base_a, store) = fixture();
        let (_g2, base_b, _s2) = fixture();
        allow_at(&store, &base_a).unwrap();
        assert_eq!(status_at(&store, &base_a), Status::Trusted);
        // A different repo path is independent.
        assert_eq!(status_at(&store, &base_b), Status::Untrusted);
    }
}
