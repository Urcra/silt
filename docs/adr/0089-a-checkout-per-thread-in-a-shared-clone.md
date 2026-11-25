# 0089: A checkout per thread in a shared clone

- Status: accepted
- Date: 2026-11-17
- Amends: [ADR 0008](0008-local-capture-architecture.md) (a clone tracked in place), [ADR 0018](0018-daemon-optional-capture.md) (what a hook loads), [ADR 0047](0047-your-clone-between-agents.md) (whose a clone is while agents work there), [ADR 0069](0069-git-rewrites-can-be-left-out-of-a-diff.md) (what a diff can leave out)

## Context
A clone tracked in place had one checkout row, bound to one thread. A second `silt claude` in the same clone rebound that row to its new thread. From then on, every edit in the clone went into the second thread's family:
- With a daemon, the first agent's edits landed in the second agent's thread, and its own thread recorded none.
- Without one, the first agent's next hook loaded its own family and a checkout that now belonged to another, and panicked.

People run two agents in one clone on purpose, for a quick question beside a long task or two halves of one change. Agents that know about each other (docs/research/agents-in-one-clone.md) need each one's work to be in its own thread first.

Two agents in one working tree also share every file. What agent B writes is on disk under agent A, and A's next edit of that file starts from B's text.

## Decision
**A clone tracked in place has a checkout row per thread working there.**
- A row is `(path, thread)`, unique together. Each row has its own actor, bridge and recorded disk state, tied to its own thread's family.
- A thread starting in a clone where another thread's agent works **adds** a row. It takes over an existing row, as before, only when that row's thread has no agent at work. "At work" means a live session, an ACP agent or an awaited session in the daemon, or a wrapper recording the thread on its own (its recording lock, ADR 0018). The row of a thread whose agent is gone can be taken over. So a clone has at most one row more than it has agents working there.
- Resuming a thread in a clone reuses that thread's own row if it has one there.
- ADR 0047's rule holds per row: a row records only while its thread's agent is at work.

**An edit on disk is offered to every recording row at that path**, and each row takes it into its own thread's family. Otherwise a thread's tree drifts from the disk, and its next edit of a file would carry the other agent's change as its own.
- With a daemon, the watcher reports a changed path to every row whose root it is, not only the last one watched.
- A hook ingests what its scan finds into its own session's row and into the other recording rows at that path.
- Without a daemon, a hook's process loads the other recording rows at its checkout's path, with their families and sessions. Only then can it offer them the edit and see their open tools.

**Who made an edit, row by row.** The candidates are the sessions of every row at the path, so a row sees the other agents' open tools as well as its own thread's.
- **A file tool names its paths.** A PostToolUse hook for Edit, Write, MultiEdit or NotebookEdit decides: the edits to the paths it names are that session's.
- **Otherwise, exactly one candidate** has an open tool, or one that just closed, or, without hooks, is in a turn. The edit is that one's, as before.
- **Otherwise, several candidates** qualify, for example a watcher event or a Bash tool's rescan while both agents have a tool open. Silt can't tell which made it, and says so rather than guess. The edit's author lists them all.

**How the author reads in each row.** The same edit is recorded differently in the thread whose agent made it and in the others:
- **`Author::Agent`** is unchanged: this thread's agent made it, credited to its tool call when known.
- **`Author::Others { sessions }`** is new: an agent of another thread working in the same clone made it. It lists one session when known and several when it's unclear which. It never names a tool, as a tool call belongs to the other thread's conversation.
- **`Author::Agents { sessions }`** is new: it's unclear which of several agents made it, and this thread's own is among them.

The two variants go last in `Author`, as stored deltas name variants by position. They bump the protocol to 18.

The meaning of each variant:
- Neither new variant is the person's, so git run by hand (`GitMade::persons_edits_since`) doesn't take them.
- A turn's late edits count `Agents` as the turn's, and never `Others`.
- `silt show` lists another agent's edits as "✎ by another agent" and an unclear edit as "✎ by one of these agents". Neither is ever placed under a tool call.

**A diff can leave other agents' edits out.** This works like git's (ADR 0069):
- `silt_log::OthersMade` is the set of edits authored `Others`.
- `silt diff --hide-others`, `silt mr show --hide-others`, `silt comment` and `silt comments` take it, and so does `others=hide` beside `git=hide` on the daemon's and the server's endpoints. Both sides are compared without those edits, through the same reads that need no causal closure.
- In the web UI, **Hide other agents' changes** sits beside **Hide git's changes**, and is kept the same way. It shows only on a thread that records other agents' edits, as a thread summary's `shared` says, so the many threads with one agent don't carry a box that changes nothing.
- `Agents` edits always show, as they may be this thread's own.
- Because the set is by author, a fork leaves out its parent's `Others` edits too, with no need to know which sessions were whose.

## Consequences
- A pull request is git commits from the shared working tree, so it carries both agents' work. Neither filter reaches it. Each agent, or the person, commits only their own files, or works in an isolated checkout (`--isolated`) instead.
- A thread recorded before this change keeps the other agent's edits as its own, as they were recorded.
- A daemonless hook in a shared clone loads every other recording thread's family, so it takes longer. A clone with one agent is unchanged.
- A row that writes to disk, such as a pull applied in place, a merge or a revert, changes files under the other rows. They take those writes in as the person's, as they would anyone's outside a tool call.
- Two rows record each edit, so the store holds it once per thread working in the clone.
- Without hooks, an agent's own edits while another agent is also in a turn are unclear, as nothing names who wrote what.
