# 0047: Your clone between agents

- Status: accepted
- Date: 2026-10-25
- Amends: [ADR 0008](0008-local-capture-architecture.md) (who an edit outside a tool call belongs to)
- Amended by: [ADR 0089](0089-a-checkout-per-thread-in-a-shared-clone.md): A checkout per thread in a shared clone

## Context
`silt claude` tracks the clone it runs in, in place (ADR 0008), and so does a Local thread started from the web UI (ADR 0044). Every edit the watcher notices there goes to the thread the clone is bound to, the person's own when no tool call made it.

The binding outlived the agent. Once the session ended, the clone stayed the thread's until another thread took it, so everything the person did there in the meantime landed in the old thread. On one machine a planning session's thread grew to 940 changed files over the days of work that followed it. A thread whose agent finished long ago showed as changed "now". Its diff, its merge request and its pull request carried work that wasn't its own.

Capture without a daemon (ADR 0018) never had this problem: a hook compares the disk only when the agent's next event comes.

## Decision
**A clone tracked in place is the thread's only while an agent works there**: a session of the thread is live, an ACP agent of the thread runs, or one is starting and its session is awaited. In between, the watcher's events for that clone are left unrecorded.

**What changed meanwhile is taken in when an agent works there again, as the person's.** The whole clone is rescanned before the agent's edits begin: when a session is registered, before a resumed session is awaited, when an ACP agent starts in the thread, and at each turn through the hooks' own scans. So the thread's tree is what the agent sees, and an edit made while it was away isn't credited to its first tool call.

**A managed checkout is the thread's alone**, under `.silt/checkouts/`, so every edit there is still recorded as it happens.

## Consequences
- Daemon and daemonless capture now agree about edits between sessions.
- A thread's diff stops at its agent's last turn, unless it's continued in the same clone.
- A restarted daemon doesn't record a clone left alone; the rescan waits for the next agent.
