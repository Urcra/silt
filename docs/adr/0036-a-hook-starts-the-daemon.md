# 0036: A hook that finds no daemon starts one

- Status: accepted
- Date: 2026-10-22
- Amends: [ADR 0018](0018-daemon-optional-capture.md) (daemonless capture), [ADR 0034](0034-performance-budgets.md) (`record.hook_daemonless`)

## Context
ADR 0018 lets a hook capture in its own process when no daemon runs, so recording never depends on one. ADR 0034 gave that capture 50 ms at 100,000 deltas and 20,000 files.

It measures about 96 ms, 111 to 125 ms at p95, because each hook process starts cold:

| Step | Time |
|---|---|
| reading and decoding the family's 100,000 deltas | 24 ms |
| decoding its 9 MB snapshot | 17 ms |
| loading the checkout's 20,000 recorded entries | 22 ms |
| comparing the checkout | 22 ms |
| sessions | 5 ms |

A family loaded lazily, without decoding the deltas its snapshot covers, would still leave about 55 to 65 ms. Getting under 50 would also need a compact stored form of the entries, a cheaper snapshot decode and a cache of git's untracked walk. That is days of work for a path that, by then, should only be taken once.

A daemon holds all of this in memory, and a hook forwarded to it takes 17 ms (`record.hook`).

## Decision
**A hook that finds no daemon starts one.**
- It still captures in its own process first, so the hook is recorded at once and nothing waits on the daemon's load.
- Then, if Silt records the session, it starts `silt daemon run` in the background, as `silt claude` does. The session's later hooks are forwarded to it.
- A session Silt doesn't record starts nothing, so hooks installed for every Claude Code session don't start a daemon for sessions nobody tracks.
- `SILT_DAEMONLESS_CAPTURE=1`, which asks for capture in process, starts none either.

ADR 0018's invariant stands: recording is correct with no daemon. If the daemon can't start, or stops, hooks keep capturing in their own process.

**The budget.** `record.hook_daemonless` is now the hook that finds no daemon, which happens once for each time the daemon isn't running. Every other hook is `record.hook`, forwarded, at 20 ms.
- **As first decided:** ADR 0018's original 200 ms at p95, at 100,000 deltas and 20,000 files.
- **Revised the same day:** ADR 0034's 50 ms again. Capture in process was made to meet it after all, as ADR 0034 records, and measures 43 to 44 ms at p95.

## Consequences
- A session started with `claude` rather than `silt claude` gets a daemon after its first hook, with the web UI, watchers and sync that come with it.
- The daemon inherits the hook's environment, which is the agent's, so it finds the agent's sessions where the agent writes them.
