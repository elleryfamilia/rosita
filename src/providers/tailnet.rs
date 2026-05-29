//! `tailnet` provider — Tailscale peers, parsed from `tailscale status`.
//!
//! Local-only: tailnet IPs/hostnames are machine-specific and never leave the
//! gitignored overlay/cache. The parser is pure for testing.

use super::{EnvProvider, ProviderOutput};
use crate::context::Context;

/// A peer on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailnetPeer {
    /// Tailscale IP (100.x.y.z).
    pub ip: String,
    /// Peer hostname.
    pub name: String,
    /// Operating system as reported by tailscale.
    pub os: String,
    /// Whether the peer is online (not reported `offline`).
    pub online: bool,
}

/// Surfaces tailnet peers.
pub struct TailnetProvider;

impl EnvProvider for TailnetProvider {
    fn id(&self) -> &'static str {
        "tailnet"
    }

    fn probe(&self, _ctx: &Context) -> crate::Result<Option<ProviderOutput>> {
        let Some(raw) = super::run_ok("tailscale", &["status"]) else {
            return Ok(None); // not installed, or logged out (non-zero exit)
        };
        let peers = parse_tailscale(&raw);
        if peers.is_empty() {
            return Ok(None);
        }
        let online = peers.iter().filter(|p| p.online).count();
        let lines: Vec<String> = peers
            .iter()
            .map(|p| {
                format!(
                    "- {} ({}) {} — {}",
                    p.name,
                    p.ip,
                    p.os,
                    if p.online { "online" } else { "offline" }
                )
            })
            .collect();
        let text = format!(
            "{} tailnet peer(s), {online} online:\n{}",
            peers.len(),
            lines.join("\n")
        );
        let data = serde_json::Value::Array(
            peers
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "name": p.name, "ip": p.ip, "os": p.os, "online": p.online,
                    })
                })
                .collect(),
        );
        Ok(Some(ProviderOutput { text, data }))
    }
}

/// Parse `tailscale status` text output into peers.
///
/// Lines look like: `100.x.y.z  hostname  owner@  os  active; relay "…"` (the
/// self line and idle peers show `-` as status). We treat a peer as online
/// unless its status column contains `offline`.
pub fn parse_tailscale(s: &str) -> Vec<TailnetPeer> {
    let mut peers = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        // First field must look like an IP (starts with a digit).
        if !f[0].chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let status = f[4..].join(" ").to_lowercase();
        peers.push(TailnetPeer {
            ip: f[0].to_string(),
            name: f[1].to_string(),
            os: f[3].to_string(),
            online: !status.contains("offline"),
        });
    }
    peers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_self_and_peers_with_online_state() {
        let raw = "\
100.64.0.1   my-laptop    me@   macOS   -
100.64.0.2   build-box    me@   linux   active; relay \"sea\", tx 1 rx 2
100.64.0.3   old-pi       me@   linux   offline
# a comment line is ignored
garbage line without ip
";
        let peers = parse_tailscale(raw);
        assert_eq!(peers.len(), 3);
        assert_eq!(peers[0].name, "my-laptop");
        assert!(peers[0].online); // "-" status → online
        assert_eq!(peers[1].ip, "100.64.0.2");
        assert_eq!(peers[1].os, "linux");
        assert!(peers[1].online); // "active" → online
        assert!(!peers[2].online); // "offline" → offline
    }

    #[test]
    fn empty_or_loggedout_yields_no_peers() {
        assert!(parse_tailscale("").is_empty());
        assert!(parse_tailscale("Logged out.").is_empty());
    }
}
