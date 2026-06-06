# Configuration

rosita is configured by layered TOML. **(implemented)** unless marked
**(planned)**.

## Layers (precedence: later wins)

**Implemented:** built-in defaults ← global ← repo.

**Planned (full):** built-in ← global (public) ← **global-local (private)** ←
repo ← **repo-local (private)**.

| Layer | Path | Shareable? |
| --- | --- | --- |
| global | `$ROSITA_CONFIG_DIR` or `$XDG_CONFIG_HOME/rosita` or `~/.config/rosita`, file `config.toml` | yes (commit / open-source) |
| global-local *(planned)* | `<global>/local.toml` | **no** (gitignored / private repo) |
| repo | `<repo_base>/.rosita/config.toml` | yes (commit, team-shared) |
| repo-local *(planned)* | `<repo_base>/.rosita/local.toml` | **no** (gitignored) |

- `$ROSITA_CONFIG_DIR` overrides the global dir (used in tests / isolation).
- Templates resolve repo → global → embedded; profiles/agents/capabilities merge
  **by id/name** (later layers override).
- Lists like `env.allowlist` are **additive** across layers (union, deduped).
- The merge keeps "unset" distinct from "default": each layer parses into an
  all-optional `RawConfig`, layers fold, then defaults are applied.

Other directories under `<repo_base>/.rosita/`: `generated/` (overlays,
gitignored), `logs/events.jsonl` (audit, gitignored), `templates/` (overrides),
`cache/` *(planned, gitignored — provider caches)*.

## `[defaults]` (implemented)

```toml
[defaults]
agent = "claude"   # agent used when --agent is omitted
```

## `[env]` (implemented)

Allowlist-only environment exposure; a name denylist wins even if allowlisted.

```toml
[env]
allowlist = ["LANG", "TERM", "CI", "TZ", "EDITOR"]      # ONLY these names surface
deny_name_patterns = ["(?i)(secret|token|key|password|credential|auth)"]
```

## `[codex]` (implemented)

```toml
[codex]
write_override = true    # auto-write AGENTS.override.md (default; `--no-override` to skip)
max_output_kib = 32      # warn when generated output exceeds this
```

## `[[profiles]]` (implemented; `capabilities`/`exclude`/`exclusive` planned)

```toml
[[profiles]]
name = "infra"
priority = 50                                                   # higher wins (today: single-winner)
when = [{ field = "path", op = "starts_with", value = "infra/" }]   # all clauses AND-ed
guidance = "Infrastructure code — prefer plans over direct mutation."   # inline (implemented)
# template = "infra"            # optional body-template override
# capabilities = ["infra-caution", "no-prod"]   # (planned) compose capabilities
# exclude     = ["deploy"]                       # (planned) remove a capability a base profile added
# exclusive   = false                            # (planned) replace rather than add
```

- Fields: `stack` `language` `package_manager` `path` `branch` `repo`
  `host_class` `os` `arch`.
- Ops: `equals` `starts_with` `contains` `matches` (regex).
- Built-in profiles (`rust`, `nextjs`, `node`, `go`, `python`, `infra`,
  `experimental`, `default`) are a base layer, overridable by name.

## `[[agents]]` (implemented)

Built-in agents are a base layer; override by `id` or add new ones — no code
change. Required: `id`, `generated_filename`.

```toml
[[agents]]
id = "gemini"
generated_filename = "gemini.md"
launch = "gemini"                  # program for `rosita run gemini` (omit → render-only)
template = "overlay"              # body template name (repo/global override → embedded)
# importer = "GEMINI.local.md"             # auto-wire @import into a LOCAL file
# override_target = "AGENTS.override.md"   # auto-merge target, gitignored (default-on)
# override_base   = "AGENTS.md"            # file whose content seeds the override
# append_prompt_flag = "--append-system-prompt"   # run injects a freshness note via this flag
wire_hint = "include .rosita/generated/gemini.md from your agent config"
```

Built-in defaults:

| id | generated file | wiring | launch |
| --- | --- | --- | --- |
| `claude` | `claude.md` | import → `CLAUDE.local.md` | `claude` |
| `codex` | `agents.md` | auto → gitignored `AGENTS.override.md` (Codex prefers it); `--no-override` = emit-only | `codex` |
| `gemini` | `gemini.md` | auto → gitignored `GEMINI.local.md` (`@import`) + registers it in `~/.gemini/settings.json` `context.fileName` | `gemini` |
| `opencode` | `opencode.md` | emit-only | `opencode` |
| `copilot` | `copilot.md` | emit-only | — (render-only) |
| `generic` | `generic.md` | emit-only | — |

## `[host_classes]` (implemented; keep mappings private)

Maps hostname globs (`*`/`?`) to a class you reference in rules
(`host_class == "work"`). **The mappings contain real hostnames/domains — keep
them in the private layer**, even though the *reference* in a profile is public.

```toml
# put this in local.toml (private), not the shareable config:
[host_classes]
work     = ["*.corp.example.com", "work-*"]
personal = ["my-laptop", "*.tailnet.ts.net"]
```

## `[[capabilities]]` (planned)

```toml
[[capabilities]]
id          = "ssh-tailnet"
description = "SSH into machines on my tailnet to do work"
tags        = ["infra", "network"]
risk        = "caution"            # info | caution | dangerous (informational)
when        = [{ field = "host_class", op = "equals", value = "personal" }]   # self-gate
requires    = ["network-awareness"]
params      = { allowed_hosts = ["host-a", "host-b"] }   # value belongs in private layer
guidance    = """
You may SSH to {{ params.allowed_hosts | join(', ') }}.
Confirm before any destructive remote command.
"""

# dynamic capability (native provider):
[[capabilities]]
id       = "network-awareness"
provider = "tailnet"     # built-in provider (or: command = "..." for the generic, trust-gated one)
cache    = "60s"
guidance = "Live tailnet (as of {{ generated_at }}):\n{{ provider.output }}"
```

See the [implementation plan](implementation-plan.md) for the exact schema and
the trust rules governing `command` providers.

## Global flags (implemented)

`--cwd <DIR>` (operate as if there), `--verbose`/`-v`, `--dry-run` (write
nothing, not even the audit log).
