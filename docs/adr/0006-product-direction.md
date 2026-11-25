# 0006: Async handoff, forking and search over live multiplayer

- Status: accepted
- Date: 2026-10-20

## Context
One way to build this is real-time multiplayer, where several people and agents work in one live thread. The user's actual pain points are different:

1. **Resume on another machine.** A Claude Code session is going on the laptop. The workday ends and the work should continue on the home PC. A WIP commit carries the code but loses the conversation, and the conversation is most of the context.
2. **Work stealing and forking.** A half-finished feature (failing tests, mid-session) gets parked when its owner switches tasks. A teammate should be able to **fork** it, getting both the code *and* the full chat, continue in their own agent session, and later send a **Silt merge request** back. The owner approves or declines it. Nothing merges automatically. If the request is declined, both lines of work carry on.
3. **History across compaction.** Humans and agents should be able to see and search what happened before a compaction point: chat and edits both.
4. **Searchability.** Finding in-progress work, past decisions and "who touched this and why".

## Decision
- **Silt is async-first.** Every thread streams continuously to the user's self-hosted server, so it can always be resumed, searched and forked. Other people see threads read-only, near-live. Real-time co-editing of one thread is deferred to a late, optional milestone.
- **People keep using their own agent CLI** (Claude Code first, then Codex and others). Silt runs underneath, capturing the agent's session and every file edit (ADR 0007). The web UI is for browsing, searching, reviewing merge requests and launching resumes and forks. Silt-driven agents over ACP and our own harness are later additions.
- **A thread has one writer at a time.** A *lease* records which device is driving it. Resuming on another device of your own moves the lease. Anyone else who wants to continue the work creates a **fork**.
- **A fork is a snapshot plus optional pull.** It starts from the source's code and conversation at fork time, and from then on is fully independent. `silt pull` can bring in the source's newer code (merged by the CRDT) and newer conversation (added as an "upstream progress" segment the agent can read). The source never changes because of a fork.
- **Merge requests:**
  - A merge request (MR) offers a fork's code changes to a target thread. The target's owner reviews the diff, the fork's conversation and any conflict hints, and approves or declines.
  - On approval, the code merges into the target worktree. The target thread gets a "merged from <fork>" event with a summary, and its agent is told about it on the next prompt. It can read the fork's full conversation on demand.
  - **Forks and their conversations are never deleted by a merge.** They stay readable and can be forked again.
- **Chat context matters as much as code.** Conversation, edits and tool calls live in one log per thread family. Fork, pull, MR and resume always carry the conversation, not only the diff.
- **Full history is kept.** Nothing is lost at compaction. The UI shows compaction boundaries but still lets you scroll the whole history. Agents get Silt tools (MCP) to search and read their own pre-compaction history, the history of forks and parents, and per-file history ("which prompt changed this line").
- **Visibility is set per project** (private or team), with a per-thread override.
- **Search** is server-side full-text first (SQLite FTS5) over messages, tool calls, file paths, diffs, commits and titles, with filters for repo, author, status, branch, file and date. Semantic search comes later.

## Consequences
- The CRDT stays: forks, pulls, MRs and multi-device resumes all need correct merging. The live sync path becomes simpler: one writer per thread, server-ordered, with no co-editing UI.
- **Turn running is simpler:** each thread has a single driver device, so there is no need to choose which machine runs a turn.
- The biggest new piece of work is **agent session capture and restore** for each agent CLI (ADR 0007).
- Live features such as shared drafts, presence/follow and shared terminals move to the late "live collaboration" milestone.
