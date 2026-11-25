# 0125: Antigravity CLI resumes from its own store, and starts primed elsewhere

- Status: accepted
- Date: 2026-11-13
- Numbered 0063 until 2026-11-26, a number another ADR also had.
- Amends: [ADR 0056](0056-signing-an-agent-in-from-the-web-ui.md) (an agent that answered v2 in v1 is asked its sign-in methods again in v1)

## Context
Google moved most people from Gemini CLI to Antigravity CLI (`agy`) on 2026-06-18, so Silt needs to record and continue it as it does the other agent CLIs (ADR 0007, ADR 0012). Gemini CLI stays, for the paid API keys and Gemini Code Assist licences that still reach it.

agy logs each conversation's steps as JSON lines (`brain/<id>/.system_generated/logs/transcript_full.jsonl`), appended as each step ends and written before a tool runs, which is enough to record a conversation and credit edits to their tool calls. But it resumes only from `conversations/<id>.db`, a SQLite database of protobuf steps; given only the transcript, it starts a new conversation (docs/research/agents.md). Writing that database would mean following an undocumented protobuf schema. Carrying it from machine to machine as an attachment would sync the conversation unredacted, as a binary, and have agy parse a peer's file (ADR 0016, ADR 0032).

agy picks its conversation ids and writes nothing that says where a conversation runs, except its index `conversation_summaries.db`, which lists each conversation's workspace as it starts.

## Decision
- **Capture:** from the transcript, as for Gemini and Codex. A reply's calls are paired with the steps after it, in order, and named `step-<index>-<n>`, as agy gives them no ids. A reply that calls no tool ends the turn, and a reply after that is a turn the agent opened itself.
- **Finding a run:** Silt reads agy's index read-only, with `rusqlite` on the same SQLite as `sqlx`, for the newest conversation of the cwd that isn't a subagent's. Started primed (`-i`) in a folder it doesn't trust yet, as a new checkout is, agy lists the conversation with no workspace, so a new one with none is taken when none names the cwd. Silt doesn't trust the folder for agy: a checkout holds what others wrote, and agy asks. The index also gives import each conversation's folder, and each conversation's subagents.
- **Restore:** `Agent::can_restore` tells the daemon whether an agent can take a recorded session back. agy can when it keeps the conversation's `.db` itself, on the machine that ran it, and resumes it with `--conversation`. Otherwise, on another machine, in a fork or from another agent, it starts primed with the conversation, as OpenCode does from another agent. Silt writes nothing into agy's store.
- **Per-run setup:** agy reads its MCP servers only from files, so it runs with `--gemini_dir` pointing at a `.gemini` of the run's own, linking everything to the real one but `config/mcp_config.json`, which adds Silt's server, `GEMINI.md`, which adds the session's instructions, and `skills`, which adds the given skills.
- **Web UI:** the registry's `antigravity-acp` runs `agy acp` when `agy` is on `PATH`, signed in as the person signed agy in, rather than downloading the registry's separate build.
- **Signing in from the web UI:** `agy acp` answers Silt's v2 `initialize` in v1 without reading it, and lists its terminal sign-in only to a v1 client that asks. The ACP library keeps that answer rather than send Silt's v1 `initialize`, so once such an agent needs a sign-in and listed no terminal method, Silt sends its v1 `initialize` on the same connection and adds the methods it then lists. The card runs `agy acp --login` as it runs Claude's `auth login` (ADR 0056).

## Consequences
- A thread continued with agy on another machine starts a new conversation, primed with the end of the old one; the agent reads the rest with Silt's history tools. Continuing on the machine that ran it is native.
- `--gemini_dir` is hidden from `agy --help`. If agy drops it, Silt's MCP server and instructions need another way in, and the overlay test fails first.
- Silt depends on the columns it reads in agy's index. When they change, finding runs, import and subagents stop until the reader follows.
