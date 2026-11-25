# 0011: History tools for agents, search, comments and the web UI

- Status: accepted (implements scenario 3 and the browsing parts of ADR 0006)
- Date: 2026-10-20
- Amended by: [ADR 0020](0020-next-generation-web-ui.md): The next-generation web UI
- Amended by: [ADR 0026](0026-hosted-mode-and-remote-control.md): Hosted mode and remote control
- Amended by: [ADR 0055](0055-reviewing-a-threads-changes.md): Reviewing a thread's changes
- Amended by: [ADR 0065](0065-workspaces.md): Workspaces: a server, defaults, rules and history apart for each kind of work
- Amended by: [ADR 0068](0068-artifacts.md): Artifacts: what an agent shows the person, beside the thread
- Amended by: [ADR 0112](0112-phones-on-the-local-network.md): Phones on the local network

## Context
Silt keeps every conversation line and edit across compactions, forks and machines (ADRs 0007–0010). M5 makes that history useful:
- Agents should be able to look back past their own compactions and into related threads.
- People should be able to search across threads and see who else is working on a file.
- Threads and merge requests should be browsable and reviewable in a web UI, with comments that stay attached to their code or message.

## Decision
**History tools over MCP.**
- `silt mcp --thread <id>` is an MCP server on stdio. JSON-RPC is written by hand: the protocol needs only `initialize`, `tools/list`, `tools/call` and `ping`, so no SDK is pulled in.
- It answers by asking the daemon over its socket. Tools:
  - `silt_history_search`: this thread and the threads it was forked from.
  - `silt_history_read`: a page of the full conversation, including what was compacted away.
  - `silt_file_history`: the prompts and tool calls that changed a file.
  - `silt_thread_search`: every thread this device can see.
  - `silt_fork_read`: another thread's conversation, usually a fork or a source.
  - `silt_in_flight`: other threads changing a path.
- `silt claude`, `silt resume` and `silt fork` pass the server to Claude Code with `--mcp-config`.
- After a compaction, Claude Code fires `SessionStart` with `source: "compact"`. The hook answers with `additionalContext` naming the tools, so the agent knows its earlier history is still within reach.

**Search with filters on the server.**
- The server already sees every delta. It keeps a search index next to the log:
  - FTS5 over conversation text.
  - Per thread: repository, owner, branch (from git ops), last activity and status.
  - The paths each thread touched. The server can't project text (it has no git blobs), so it follows only tree operations: create and move give an inode's parent and name, and later stamps win.
- Status is `merged` for a fork whose merge request was approved, `active` for a thread written in the last 24 hours, and `idle` otherwise.
- `GET /v1/search` filters by repo, author, status, branch, file and date, and only ever returns visible families (ADR 0010).
- `silt search` uses the server when the device is connected and falls back to the local index.
- `GET /v1/in-flight?repo=&path=` lists threads that touched a path, with their owner, status and last activity. `silt in-flight <path>` shows it for the current repository.

**Comments.**
- Comments are deltas, so they sync and are versioned like everything else.
- `Op::Comment(CommentOp::Added { target, text })` and `CommentOp::Resolved { comment }`.
- A target is a conversation delta (`Message`), or a line range of a file held by CRDT anchors (`Code { inode, start, end }`).
- Anchors name characters, not positions, so a comment follows its code through edits. When the anchored text is deleted, the comment is shown as outdated at its last position.
- Git-lazy files are turned into text documents when commented on, like on their first edit.

**Web UI served by the daemon.**
- `silt open` starts, or reuses, an HTTP listener on 127.0.0.1 with a random token, and opens the browser at `/#token=…`.
- The UI is the React app in `web/`, embedded in the binary. The page reads the token from the fragment and sends it as a bearer token. The listener also rejects foreign `Host` headers.
- The HTTP API is a thin JSON layer over the same requests as the socket, so the UI and the CLI can't drift apart. It covers threads, show, diff by turn, search, merge requests (show, approve, decline), comments, and the commands to resume or fork a thread.
- The daemon has the user's own threads and every visible family it synced. That is enough for browsing, search and review, without a second copy of the logic in the server. A server-hosted UI for users without a daemon is left for later.
- Playwright tests drive the built UI against a daemon whose threads were recorded with the fake Claude used by the CLI tests.

## Consequences
- Agents get history access without Silt being their harness, through the MCP support their CLI already has.
- The server learns more about the threads (paths, branches, text) to answer search. It already stored all of it, so nothing new is exposed. Redaction still happens before upload.
- New ops are appended at the end of `Op`, so stored logs stay readable.
- The web UI is local-first. Opening it needs `silt` on the machine; a hosted read-only UI can reuse the same API shape later.
