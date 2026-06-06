# rosita docs

`rosita` injects global context into your AI coding agents: detect the
project/runtime context, select the **one** profile that fits, compose its
capabilities into a single agent-neutral overlay, and deliver it to each agent —
keeping it fresh and never leaking secrets.

## For consumers
- [Concepts](concepts.md) — the mental model (context, capabilities, profiles, the binding, agents, freshness, public/private).
- [Configuration](configuration.md) — the layered config and full schema reference.
- [Security](security.md) — secrets, redaction, the public/private split, and the command-execution trust model.

## For developers / extenders
- [Architecture](architecture.md) — modules, trait seams, and the render pipeline (reflects the current code).
- [Extending](extending.md) — add an agent, a capability, a provider, a detector, or a rule.

## Roadmap / execution
- [Implementation plan](implementation-plan.md) — the detailed, phased plan to build capabilities, the public/private layer, native environment providers, and dynamic capabilities on top of the current MVP.

## Status legend
Throughout these docs:
- **(implemented)** — shipped in the current binary.
- **(planned)** — designed and specified in the [implementation plan](implementation-plan.md), not yet built.
