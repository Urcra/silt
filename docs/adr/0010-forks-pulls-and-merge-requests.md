# 0010: Forks, pulls, merge requests and visibility

- Status: accepted (implements the fork and merge-request parts of ADR 0006)
- Date: 2026-10-20
- Amended by: [ADR 0029](0029-fork-from-and-revert-to-a-turn.md): Forking from and reverting to a turn

## Context
ADR 0006 asks for scenario 2: a teammate forks someone's in-progress thread, getting both code and conversation, continues in their own agent, and later offers the work back with a merge request (MR) that the owner approves or declines. ADR 0009 already gives threads a family log, per-thread views, leases, sync and `silt resume`. This ADR says how forks, pulls and MRs are represented in that log and who may see what.

## Decision
**A fork is a thread in its source's family.** `silt fork <thread>` appends `Op::Fork { from }` on a new thread, with the source's head as parents. The fork's view is the family as of its own head, so it starts with the source's files and sees none of the source's later work (the per-thread views of ADR 0009). Forking works on any device that can read the family: the forker catches the family up, makes sure the base commit is present (origin or bundles), creates the fork, takes the fork's lease and materializes a managed checkout (or the current clone with `--here`), like `silt resume`.

**The fork carries the conversation.** A thread's conversation is its own conversation deltas plus, for a fork, the conversation of its source chain up to the fork point. The agent session is restored natively (same agent, single account; ADR 0007) under a **new session id**: every occurrence of the old id is rewritten, along with the working and session directories. The source's session is never touched, so both can continue.

**Pull.** `silt pull` on a fork appends `Op::Merge { from, mr: None }` on the fork with parents `fork head ∪ source head`. The CRDT merges the source's newer edits into the fork's view, and the fork's checkout is reconciled. The source's conversation since the previous fork or pull point becomes an *upstream progress* segment: `silt show` lists it, and the fork's agent is told about it on its next prompt.

**Merge requests.**
- `silt mr create` on a fork appends `Op::Mr(MrOp::Opened { target, head, title, summary })`, where `target` is the source thread and `head` is the fork's head at that moment. The MR's id is that delta's id. Newer work on the fork needs a new MR.
- `silt mr list` and `silt mr show <mr>` work on any device that has the family. `show` gives the fork's changes since the merge base, the fork's conversation and conflict hints: line ranges that both the target and the fork changed since the merge base. The CRDT merges them anyway; the hints say where to look.
- `silt mr approve <mr>` must run on the device that holds the target's lease. It appends `Op::Merge { from: fork, mr: Some(id) }` on the target with parents `target head ∪ MR head`, then `Op::Mr(MrOp::Approved { mr })` and a `ConvOp::Merged { from, mr, summary }` conversation event. Target checkouts are reconciled, so the merged code lands on disk.
- `silt mr decline <mr> [--reason]` appends `Op::Mr(MrOp::Declined { mr, reason })`. Nothing else changes.
- A merge never deletes or closes the fork. It stays readable and can be pulled into, continued, forked again or offered again.

**Telling the agent.** Claude Code's `UserPromptSubmit` hook may print `additionalContext`. The daemon keeps notices per thread: a merge into it, or upstream progress pulled into it. The next prompt's hook prints them once. A notice names the other thread and says how to read its full conversation (`silt show <thread>`).

**Users and visibility.**
- Every device belongs to a user. `silt server pair --user <name>` creates a pairing code for that user (the user is created if missing). Devices paired before users existed belong to the user `owner`.
- A thread's owner is the user whose device registered it. Only the owner's devices can take its lease. Anyone else continues the work in a fork.
- Visibility is `private` (owner only) or `team` (every user of the server), and it is **per thread**. A thread's own setting (`silt visibility private|team --thread <id>`, owner only) wins. A fork without one follows its source. Otherwise the project's setting applies (`silt visibility private|team`, keyed by normalized origin URL, default `team`). Any user can change a project's setting, since a server is shared by a team that trusts each other.
- Forks share their family's log, so the server filters what it sends. A user reads a delta when its thread is visible to them, or when a visible thread builds on it (a pull or a merge), or when it is offered to a visible thread in a merge request. That is how private work gets shared, and only then.
- Pull cursors move past withheld deltas. The listing therefore gives each family a fingerprint of the visible threads and the shared deltas. When it changes, the client reads the family again from the start, and the server dedupes as usual.
- Hidden threads don't appear in listings, search or in-flight queries. Taking a hidden thread's lease is refused with `404`, so its existence isn't revealed. Deltas of threads the server doesn't know yet go only to the family's owner.

## Consequences
- Forks, pulls and MRs need no new sync machinery: they are deltas in the family log, so every device that can read the family sees MRs and their outcomes.
- Merging is always CRDT merging inside one family. Work from another family can't be merged.
- New ops are appended at the end of `Op` and `ConvOp`, so stored and synced logs stay readable.
- Each thread gets its own daemon actor (derived from the device's actor and the thread id). Threads of a family grow side by side once forks exist, and an actor's deltas must form one causal chain.
- Restoring a forked session rewrites every former id, working directory, checkout root and session directory in one pass, longest match first, so new paths that contain old ones are never rewritten again.
- The recovery fork for a diverged family (ADR 0009) can reuse `silt fork` later.
