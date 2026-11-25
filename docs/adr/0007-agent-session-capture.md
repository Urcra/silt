# 0007: Capturing and restoring native agent sessions

- Status: accepted
- Date: 2026-10-20
- Amended by: [ADR 0038](0038-cloud-sessions.md): Recording Claude Code cloud sessions

## Context
Under ADR 0006, users keep running their own agent CLI (Claude Code, Codex, …), and Silt has to move those sessions between machines and people. M0 probes showed the following:
- Claude Code stores each session as append-only JSONL: `~/.claude/projects/<slug(cwd)>/<id>.jsonl`, plus `subagents/` and `tool-results/` folders next to it.
- Entries form a `uuid`/`parentUuid` tree. Compaction deletes nothing.
- A session copied to another path, with its paths rewritten and a new id, resumes correctly with `claude --resume`.
- Hooks expose turn and tool boundaries along with `transcript_path` and `tool_use_id`.

## Decision
**Agent adapters.** `silt-agents` defines an `AgentAdapter` trait, with one implementation per CLI. Each adapter can:
- `discover`: find the session for a checkout (by cwd and session id, or by hook).
- `capture`: tail the native transcript and turn new entries into Silt deltas.
- `restore`: write a native session for a target checkout, user and session id, then return the command to launch, e.g. `claude --resume <id>`.
- `translate_from`: optionally turn another agent's conversation into this agent's native format.
- `render`: produce a plain transcript for any agent.

**Storage.** Native transcript entries are kept **verbatim** as content-addressed blobs. Silt also writes normalized `Conv` ops: user and assistant messages, thinking, tool calls and results, compaction boundaries, subagent sidechains. Search and the UI work off the normalized form; restore uses the verbatim form, so nothing is lost.

**Capture paths.**
- The checkout watcher captures file edits.
- Agent hooks (a Silt plugin for Claude Code) mark turn and tool boundaries, so each edit is attributed to the right `tool_use_id`, prompt and turn.
- The transcript tail captures the conversation.

**Starting.** Two entry points:
- `silt claude [args]`: a wrapper that creates or attaches the thread, installs hooks for the session, and runs `claude`.
- `silt track`: attaches a thread to a session that is already running.

**Restore on resume.**
1. Materialize the checkout at the thread head (ADR 0006: by default a managed checkout; `--here` uses the current clone).
2. Rewrite the absolute paths in the verbatim entries from the old checkout root to the new one.
3. Set the session id: keep it when resuming your own thread, or mint a new one for a fork.
4. Write the file under the target slug directory.
5. Launch `claude --resume <id>`.

**Handing off to another person, or to a different agent:**
- **Same agent, same account:** native restore, thinking blocks included.
- **Same agent, different account:** native restore with **thinking blocks stripped**. AI providers protect thinking content, so signed thinking won't validate across accounts. Decided 2026-10-20: cross-account restore is out of scope for now (single-account use only), and stripping is how it will work once added.
- **Different agent with translation coverage** (e.g. Claude Code → Codex): `translate_from` produces a native session for the target agent.
- **Otherwise:** start a new session primed with a rendered transcript and a pointer to Silt's history tools.

**History tools.** Silt runs an MCP server for agents in tracked sessions, with tools such as `silt_history_search`, `silt_history_read`, `silt_file_history`, `silt_thread_search`, `silt_fork_read`. A `SessionStart` / post-compaction hook tells the agent that these tools exist.

## Consequences
- Silt depends on each CLI's on-disk format, which can change between versions. Each adapter records the agent version it captured, the verbatim storage keeps things recoverable, and every adapter needs per-version fixture tests.
- Claude Code comes first. Codex (`~/.codex/sessions`, `codex resume`) comes next, then the others.
- ACP-driven agents (ADR 0005) become one more adapter, used for agents started from the web UI.
