# 0008: Local capture architecture

- Status: accepted
- Date: 2026-10-20
- Amended by: [ADR 0018](0018-daemon-optional-capture.md): capture is correct without the daemon, which becomes an accelerator and the host for live features
- Amended by: [ADR 0035](0035-deltas-stored-once-compressed.md): A family's deltas stored once, compressed in chunks
- Amended by: [ADR 0047](0047-your-clone-between-agents.md): Your clone between agents
- Amended by: [ADR 0089](0089-a-checkout-per-thread-in-a-shared-clone.md): A checkout per thread in a shared clone

## Context
M2 must record a native agent session (Claude Code first) and every file edit on one machine. Each edit is attributed to the prompt and tool call that made it, and everything survives restarts. M3 then syncs the same data to a server, and M4 forks it. ADR 0007 decides *what* is captured. This ADR decides how the local pieces fit together.

## Decision

### One log per thread family
`silt-log` defines the family history: a `CausalGraph` plus `Delta { span, parents, author, time, op }`. Op variants:
- `Tree(TreeOp)`: creating, moving and deleting files. Inodes are created with their content source (empty, a git blob, or a binary blob).
- `Text { inode, op }`: edits to a text file's content, using the `silt-crdt` text CRDT.
- `Blob { inode, blob }`: last-writer-wins content for binary or oversized files.
- `Conv(ConvOp)`: conversation records.
- `Git(GitOp)`: the genesis import of a commit, and commits made on the thread.

`Family` is a pure projection of this log: the file tree, per-file documents, and the conversation per thread. Base-blob files stay lazy. The first text edit to one needs the blob's content, which the caller supplies, since the projection does no I/O.

### Conversation stored as native lines
- The verbatim lines of the agent's session files are the record: `ConvOp::Native { session, file, line }`. Here `file` is the main transcript or a subagent transcript, and large tool-result files are stored as blobs.
- The normalized view (messages, thinking, tool calls and results, compaction boundaries) is a pure function of those lines. It is computed when read and indexed for search.
- Nothing is stored twice, so normalization can improve without migrating data.
- Hook events are recorded as `ConvOp::Turn` markers (prompt submitted, tool started/finished with `tool_use_id`, stop).

### Store
- One SQLite database per user, with migrations created by `sqlx migrate add` and a table name set per crate. It holds: families, deltas (in local application order), threads, checkouts with per-file disk state, agent sessions with tail offsets, and an FTS5 search index.
- Blobs are stored on disk, addressed by BLAKE3.
- On start, the daemon rebuilds the projections by replaying deltas. Snapshots come later.

### Daemon
- `silt daemon` is one long-running process that owns the store, the checkout watchers and the session tailers.
- CLI commands and agent hooks talk to it over a Unix socket in the data dir, with one JSON request and one JSON response per line. CLI commands start it on demand.
- If the daemon crashes or restarts, it reloads state, resumes each tail from its stored byte offset, and rescans each watched checkout against its recorded disk state. Nothing is lost.

### Checkouts
- `silt claude` tracks the current checkout in place by default. `--isolated` creates a managed checkout (ADR 0002).
- Each checkout is its own actor for tree operations, which are based on the thread head.
- Text edits to a file come from an actor derived from the checkout and the file (`file_actor`). The graph requires each actor's deltas to form a causal chain. Edits to different files are based on different disk versions, so one actor per checkout would break that chain.
- The bridge between disk and the log compares new disk content against `text_at(disk_version)`. The resulting edits have `disk_version` as their parents, so concurrent edits from another checkout merge instead of one overwriting the other.
- Diffs are line-based (imara-diff) and then trimmed to the minimal character change within each hunk, so anchors and attribution survive small edits.
- File content in the log is git-canonical. The disk form is produced and parsed through gix filters, with a git fallback for the cases gix gets wrong (ADR 0002).

### Attribution
- `silt claude` starts Claude Code with a session id chosen by Silt and with hooks injected through `--settings`.
- `PreToolUse`/`PostToolUse` hooks open and close an attribution window for each `tool_use_id`. The `PostToolUse` hook waits until the watcher has flushed, so a tool's edits land inside its window.
- Disk edits outside any window are attributed to the user.

## Consequences
- The daemon is needed from M2 on. It is also where M3's sync loop will live.
- Replaying the whole log on start is fine for now. Snapshots are needed once logs get large.
- Normalization depends on Claude Code's JSONL format, which can change between versions. Unknown entry types are kept verbatim and shown raw.
