//! Git detection by shelling out to `git` (no libgit2 C dependency).
//!
//! The pure parsing helpers ([`parse_branch`], [`parse_remotes`]) are unit
//! tested without a real repo; an integration test exercises the live path.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::context::{Context, ContextDetector, DetectInput, GitContext, GitRemote};
use crate::redact::sanitize_url;

/// Detector that fills [`Context::git`] and [`Context::repo_name`].
pub struct GitDetector;

impl ContextDetector for GitDetector {
    fn name(&self) -> &'static str {
        "git"
    }

    fn detect(&self, input: &DetectInput, ctx: &mut Context) -> crate::Result<()> {
        let cwd = &input.cwd;
        let Some(root) = find_repo_root(cwd) else {
            return Ok(()); // not a git repo; leave ctx.git = None
        };

        // `--abbrev-ref HEAD` returns "HEAD" for both detached *and* unborn
        // (no-commit) branches; `symbolic-ref` distinguishes them — it resolves
        // an unborn branch but fails on a truly detached HEAD.
        let branch = git_output(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
            .and_then(parse_branch)
            .or_else(|| {
                git_output(cwd, &["symbolic-ref", "--short", "HEAD"]).and_then(parse_branch)
            });
        let remotes = git_output(cwd, &["remote", "-v"])
            .map(|s| parse_remotes(&s))
            .unwrap_or_default();
        let is_worktree = detect_worktree(cwd);

        ctx.repo_name = repo_name(&root, &remotes);
        ctx.git = Some(GitContext {
            root,
            branch,
            remotes,
            is_worktree,
        });
        Ok(())
    }
}

/// Find the git work-tree root containing `cwd`, if any.
pub fn find_repo_root(cwd: &Path) -> Option<PathBuf> {
    let out = git_output(cwd, &["rev-parse", "--show-toplevel"])?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Run `git -C <cwd> <args...>`, returning stdout on success (status 0).
fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

/// A linked worktree's git-dir differs from the common git-dir.
fn detect_worktree(cwd: &Path) -> bool {
    let git_dir = git_output(cwd, &["rev-parse", "--absolute-git-dir"]);
    let common = git_output(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    // older gits don't support --path-format; fall back.
    .or_else(|| git_output(cwd, &["rev-parse", "--git-common-dir"]));
    match (git_dir, common) {
        (Some(g), Some(c)) => g.trim() != c.trim(),
        _ => false,
    }
}

/// Parse `rev-parse --abbrev-ref HEAD` output into a branch name.
///
/// Returns `None` for a detached HEAD (`"HEAD"`).
pub fn parse_branch(raw: String) -> Option<String> {
    let b = raw.trim();
    if b.is_empty() || b == "HEAD" {
        None
    } else {
        Some(b.to_string())
    }
}

/// Parse `git remote -v` output into deduplicated, sanitized remotes.
pub fn parse_remotes(raw: &str) -> Vec<GitRemote> {
    let mut out: Vec<GitRemote> = Vec::new();
    for line in raw.lines() {
        // Format: "<name>\t<url> (fetch|push)"
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(url)) = (parts.next(), parts.next()) else {
            continue;
        };
        if out.iter().any(|r| r.name == name) {
            continue; // collapse fetch/push duplicates
        }
        out.push(GitRemote {
            name: name.to_string(),
            url: sanitize_url(url),
        });
    }
    out
}

/// Derive a repo name from the first remote URL, falling back to the root dir.
fn repo_name(root: &Path, remotes: &[GitRemote]) -> Option<String> {
    if let Some(remote) = remotes.first() {
        if let Some(name) = name_from_url(&remote.url) {
            return Some(name);
        }
    }
    root.file_name().map(|n| n.to_string_lossy().into_owned())
}

fn name_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    // Handle both "scheme://host/org/repo(.git)" and "git@host:org/repo(.git)".
    let tail = trimmed
        .rsplit(['/', ':'])
        .next()
        .filter(|s| !s.is_empty())?;
    let name = tail.strip_suffix(".git").unwrap_or(tail);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_detached_is_none() {
        assert_eq!(parse_branch("HEAD\n".into()), None);
        assert_eq!(parse_branch("  \n".into()), None);
        assert_eq!(parse_branch("main\n".into()), Some("main".to_string()));
        assert_eq!(
            parse_branch("feature/x\n".into()),
            Some("feature/x".to_string())
        );
    }

    #[test]
    fn remotes_dedup_and_sanitize() {
        let raw = "origin\thttps://user:tok@github.com/org/repo.git (fetch)\n\
                   origin\thttps://user:tok@github.com/org/repo.git (push)\n\
                   upstream\tgit@github.com:other/repo.git (fetch)\n";
        let remotes = parse_remotes(raw);
        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].url, "https://github.com/org/repo.git");
        assert_eq!(remotes[1].name, "upstream");
        assert_eq!(remotes[1].url, "git@github.com:other/repo.git");
    }

    #[test]
    fn name_from_various_urls() {
        assert_eq!(
            name_from_url("https://github.com/org/repo.git").as_deref(),
            Some("repo")
        );
        assert_eq!(
            name_from_url("git@github.com:org/my-repo.git").as_deref(),
            Some("my-repo")
        );
        assert_eq!(
            name_from_url("https://github.com/org/repo/").as_deref(),
            Some("repo")
        );
    }
}
