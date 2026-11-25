# 0005: Agents are discovered through the public ACP registry

- Status: accepted. Scope reduced by ADR 0006/0007: ACP is now used only for agents started from the web UI (M8), while native CLI adapters are primary.
- Date: 2026-10-20

## Context
The plan hard-coded an agent list (Claude, Codex, Gemini). The Agent Client Protocol project publishes a registry at `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`. That registry lists 42 agents, each with `distribution` set to `npx {package, args}` or to per-platform `binary {archive, cmd, args, sha256}`. The Claude adapter is `@agentclientprotocol/claude-agent-acp`.

## Decision
- `silt-acp` reads the ACP registry (cached, with an offline fallback to a bundled snapshot) to list and install agents.
- npx agents run through the user's node.
- Binary agents are downloaded into the data dir and checked against their sha256.
- Users can add custom agent commands in settings.

## Consequences
- New agents appear without a Silt release.
- We depend on the registry's availability and format. The bundled snapshot covers both.
