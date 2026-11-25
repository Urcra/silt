# 0018: Capture works without the daemon

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0008](0008-local-capture-architecture.md) (Daemon)
- Amended by: [ADR 0034](0034-performance-budgets.md): Performance budgets, their fixtures, and the gate
- Amended by: [ADR 0036](0036-a-hook-starts-the-daemon.md): A hook that finds no daemon starts one
- Amended by: [ADR 0089](0089-a-checkout-per-thread-in-a-shared-clone.md): A checkout per thread in a shared clone

## Context
ADR 0008 makes the daemon the only process that records anything. It owns the store, the checkout watchers and the session tailers. `silt hook` forwards each agent hook to it over the socket, and does nothing when the daemon can't be reached, so that a missing daemon can't break the agent session.

That makes capture only as reliable as one long-running process:
- **Hooks are lost while the daemon is down.** On restart, edits come back through the rescan, and transcript lines come back from their stored offsets. Turn marks from hooks that fired in the meantime are gone for good, so the edits from that time are credited to the user or to the wrong tool.
- **A hung daemon is worse.** Each hook waits up to 20 s for an answer, and then drops the mark.
- **The tool window lives in memory.** It is `Session::open_tool`, and so are the notices meant for the agent's next prompt (recovery forks, merged MRs).
- **Many M9 findings come from the process staying up.** B4.2 (memory changed before the store), B4.18 (a bad file aborts startup), B4.31 (pending sessions never expire), B4.32 (a purge during an ACP run crashes the daemon) and B4.33 (`Reload` clears in-flight flags).

The focus in ADR 0006 is handoff, forks, merge requests and history. None of these needs a resident process: they are operations on the log. Only the live features of ADR 0014 do: the web UI, ACP agents, presence and drafts, and remote edits written to disk as they arrive.

The daemon already has a capture path that needs no watcher: on startup it compares each checkout with its recorded disk state and resumes each transcript from its stored offset. That path is why "killing and restarting the daemon mid-session loses nothing" (M2) holds for edits and transcripts.

## Decision
**Invariant.** Recording a session is correct when no daemon runs. The daemon makes capture faster and more precise in time, and it hosts what has to keep running. It is no longer needed for capture to be correct.

**Two modes, one capture step.** In *resident* mode the daemon captures, as today. In *daemonless* mode, `silt hook` and the `silt <agent>` wrapper capture in their own process. Both modes run the same capture step for a session:
1. Catch the family up with the deltas other writers stored since this process last looked.
2. Tail the session's files from their stored offsets.
3. Compare the checkout with its recorded disk state. Files whose size and mtime match aren't read. After `Edit`, `Write`, `MultiEdit` and `NotebookEdit`, only the paths named in the tool's input are compared. After any other tool, and at `UserPromptSubmit` and `Stop`, the whole checkout is.
4. Append the hook's turn mark.
5. Store the deltas, offsets and disk state in one transaction.

A step is idempotent: running it twice records nothing the second time. Turn marks are keyed by `(session, tool_use_id, mark)`, so a mark the daemon recorded late isn't recorded again.

**Hook boundaries decide attribution, in both modes.**
- Edits found at `PostToolUse` belong to that tool call.
- Edits found at `UserPromptSubmit`, `PreToolUse` or `Stop` belong to the user, because they were made since the last hook.
- In resident mode the watcher records edits sooner, but it doesn't change who gets the credit.
- The open tool window is read from the log (the last `ToolStarted` without a matching `ToolFinished`), not from memory. Either mode can then close a window the other opened.

**The store is the only source of truth.**
- Whoever writes takes `capture.lock` in the data dir for one step or one command. The daemon too: it takes the lock per step, not for its whole life.
- `daemon.lock` stays, but it now only means "one resident daemon".
- Every writer keeps a per-family cursor (`lv`) and loads what others appended before it writes. The daemon's `ClientSync` push cursor learns about those deltas the same way.

**How `silt hook` picks a mode.**
- If `daemon.lock` is free, it captures in process.
- If the lock is held and the daemon answers within 2 s, it forwards the hook, as today.
- If the lock is held and the daemon doesn't answer, it captures in process under `capture.lock`. The daemon catches up when it comes back.
- `SILT_DAEMONLESS_CAPTURE=1` forces in-process capture, for tests and for anyone who doesn't want a resident process.

**Cold start.** In daemonless mode a hook loads only its own family: a projection snapshot plus the deltas after it. ADR 0008 already expected snapshots to be needed. Budget: p95 hook time of at most 200 ms on a family of 100k deltas in a checkout of 20k files.

**Agents without hooks** get their attribution from transcript marks (every adapter but Claude Code's). The `silt <agent>` wrapper lives exactly as long as the session. In daemonless mode it runs the capture step every second and once more when the agent exits. It also finds the session file when Silt can't choose the session id. `silt track` on an agent without hooks still needs the daemon.

**Notices for the agent are stored**, not held in memory, so either mode can deliver them at `UserPromptSubmit`. Teammates' live drafts (ADR 0014) stay a daemon feature.

**Sync without the daemon.**
- `Stop` and the end of the session start a detached `silt sync --once`, so the hook never waits on the network.
- That run pushes deltas and blobs, and checks for new commits for the git mirror and bundles (ADR 0009).
- `silt resume`, `fork` and `pull` already fetch what they need before acting.

**Out of scope here.**
- Commands other than capture (`fork`, `mr`, `resume`, `log`, `search` …) keep starting the daemon on demand. If it can't start they fail visibly, so nothing is lost silently.
- Running read-only commands straight from the store is possible later.
- So is a daemon that exits when idle.

**Daemon only:** the web UI, ACP agents, presence and live drafts, background pulls, writing other machines' changes into checkouts as they arrive, and `silt track` for agents without hooks.

## Consequences
- A daemon that is stopped, crashed, hung or mid-upgrade no longer costs turn marks or misattributes edits. Daemonless capture always runs the binary the hook calls, so version skew between the CLI and the daemon matters less.
- The M9 lifecycle bugs still need fixing, but they can no longer lose capture.
- Two modes have to stay equivalent. They share one capture step, and a test runs the same scripted session in both modes and compares the projections.
- The store gets more than one writer. Catching up before each write touches the daemon's in-memory families and its sync cursors. This is the riskiest part of the change.
- Hooks pay a cold start in daemonless mode. Snapshots are a prerequisite, and the store needs a versioned snapshot format (see B4.27).
- New stored state (snapshots, notices, turn-mark keys) needs migrations made with `sqlx migrate add -r`.
- In daemonless mode the other machine sees a session at `Stop` or at exit, not continuously. That is enough for async handoff. Anyone who wants continuous sync keeps the daemon running, which stays the default.
