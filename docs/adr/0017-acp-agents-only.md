# 0017: ACP agents only, no native harness

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0014](0014-live-collaboration-and-silt-driven-agents.md) (the native harness, the sandbox), [ADR 0015](0015-secret-overlay.md) (the harness and model APIs), [ADR 0016](0016-untrusted-peer-content.md) (the sandbox, the harness's file tools)
- Amended by: [ADR 0092](0092-local-models.md): Local models, run by Silt, and workspaces that stay on the machine
- Amended by: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md): Silt sandboxes the agents it starts, and production is reached through plugins

## Context
ADR 0014 added `silt harness`, Silt's own agent on a model API, and `silt sandbox` for the commands it runs. Since then:
- **Others already cover what it offers.** ADR 0006 and 0007 have people bring their own agent. The ACP registry offers Claude Code, Codex, Gemini CLI and OpenCode. Between them they cover bring-your-own-key providers and local models.
- **Owning an agent means owning everything it executes.** That includes:
  - a sandbox;
  - file tools that resist symlinks and races;
  - redacting what goes to the model.

  The 2026-10-20 review found two critical findings and several high ones there (B2.1–B2.4), all in code that exists only for the harness.
- **It is Linux only,** and fails on kernels without unprivileged user namespaces.

## Decision
**Silt captures and syncs. Agents bring their own execution and their own trust model.**
- Agents started from the web UI are ACP agents: the registry's, and custom ones from `agents.json`.
- `silt harness`, `silt sandbox` and the `silt-harness` and `silt-sandbox` crates are removed, along with `SILT_HARNESS_MODEL` and `SILT_HARNESS_UNSANDBOXED`.
- **What a command may do** is up to the agent: its permission prompts, and its own sandbox where it has one, such as Codex's workspace sandbox or Claude Code's sandbox. Silt passes permission requests to the person, as before.
- **What stays from ADR 0016:**
  - agents get an allow-listed environment, now built in `silt-acp`;
  - registry downloads are checked against their checksum;
  - Silt's own git commands run hardened;
  - `.agents/prepare` runs only when the person allowed it.
- **ADR 0015's section on model APIs** goes with the harness. Redaction on push and the overlay are unchanged.

## Consequences
- **Bring-your-own-key and local models** come through an ACP agent that supports them, such as OpenCode.
- **No agent works out of the box.** On a machine without `npx` or network access, the web UI has no agent until one is installed or listed in `agents.json`.
- **Edits on top of a collaborator's** are the agent's responsibility. Edit-based tools, like Claude Code's and Codex's, fail when the text they change has moved.
- **Prompt injection from shared threads.** Commands an agent runs after reading someone else's content are contained only as far as that agent contains them. Turning on the agent's own sandbox is the advice for shared threads.
- **Tests.** Tests of the ACP host use `silt-fake-agent`.
- **Old sessions.** Threads recorded with the harness keep their history and resume into agent CLIs. Starting the harness again reports that there is no such agent.
