# 0034: Performance budgets, their fixtures, and the gate

- Status: accepted
- Date: 2026-10-22
- Amends: [ADR 0018](0018-daemon-optional-capture.md) (the hook budget)
- Amended by: [ADR 0052](0052-commits-tolerate-a-slower-machine.md) (a commit tolerates a slower machine), [ADR 0101](0101-the-ui-under-heavy-load.md) (scrolling a long thread, and what a change reads again)
- Amended by: [ADR 0036](0036-a-hook-starts-the-daemon.md): A hook that finds no daemon starts one
- Amended by: [ADR 0037](0037-cloud-machine-budgets.md): Budgets for a cloud session's machine
- Amended by: [ADR 0091](0091-a-faster-gate-and-a-pass-that-stands.md): A faster gate, and a pass that stands

## Context
Dogfooding Silt on its own repository showed it doesn't keep up with real-sized history:
- `silt track` took about five minutes on a 123 MB, 65,000-line Claude Code transcript.
- The session took 380 MB on disk.
- The daemon took 3.3 s to start.
- The thread page ran the browser out of memory.

The only performance check before this was `hook_latency`, a bench outside the gate, so nothing caught any of these.

Silt should feel instant at the size people actually use it. That means fixed budgets, measured on fixtures as big as real use, checked by the gate.

## Decision
**One anchor.** Importing a 100 MB transcript takes under a second: about 100 MB/s, with constant work per line. Every time budget is set at that scale.

| Budget | What's measured | Limit | Made to pass by |
|---|---|---|---|
| `record.import` | tracking a 100 MB transcript, the daemon already running | < 1 s | recording |
| `import.session` | `silt import` of a 100 MB transcript that ran before Silt, the daemon already running ([ADR 0043](0043-history-from-day-one.md)) | < 1 s | importing (M23) |
| `record.hook` | `silt hook` as agents run it, forwarded to the running daemon, process start included, at 100k deltas and 20k files | p95 ≤ 20 ms | recording |
| `record.hook_daemonless` | the hook that finds no daemon: its capture step in process, before it starts one ([ADR 0036](0036-a-hook-starts-the-daemon.md)), at 100k deltas and 20k files | p95 ≤ 50 ms | recording |
| `store.size` | the data dir (database, WAL, blobs) over the transcript bytes it records | ≤ 1.5× | storage |
| `daemon.startup` | from spawning the daemon to its first answer, holding every fixture | < 100 ms | startup |
| `daemon.ready` | from spawning the daemon to its first thread listing, holding every fixture (100 MB of transcript) | < 1 s, so 100 MB/s | startup |
| `checkout.rescan` | comparing a 50,000-file checkout with its recorded state | < 200 ms | big repositories |
| `daemon.idle` | CPU of a daemon with nothing to do, over 10 s | ≤ 0.1 % of a core | big repositories |
| `web.thread` | a 2,000-message thread in the browser | shown < 100 ms, heap < 200 MB | the web UI |
| `web.scroll` | long tasks over 100 wheel notches up that thread ([ADR 0101](0101-the-ui-under-heavy-load.md)) | < 500 ms | the web UI under load |
| `web.change` | history read again after one change to that thread, open ([ADR 0101](0101-the-ui-under-heavy-load.md)) | < 64 KB | the web UI under load |
| `sync.push` | `silt sync --once` pushing a family holding the 100 MB transcript to a server | < 1 s, so 100 MB/s | sync |
| `sync.pull` | `silt resume` pulling that family onto another machine, its session restored | < 1 s, so 100 MB/s | sync |
| `app.threads` | the server's hosted app listing threads, holding that family | < 100 ms | the server |
| `app.history` | the hosted app's first page of that session, the first time it's opened | < 100 ms | the server |
| `page.fold` | a page's data folded along its thread: 10,000 writes over 1,000 documents | < 10 ms | page data (ADR 0072) |
| `doc.replay` | a doc's text replayed from 20,000 edits | < 15 ms | docs (ADR 0077) |
| `sheet.eval` | a 1,000-row sheet folded from its cells and computed, a SUM on every row | < 20 ms | sheets (ADR 0077) |

**Hooks.** ADR 0018 gave a hook 200 ms, with no daemon running. Now there are two budgets:
- **20 ms for a hook as agents run it.** `silt claude` starts the daemon, so a hook is forwarded to it. This is what makes recording a live session add nothing an agent's hooks would notice.
- **50 ms for capture with no daemon.** A process starting cold must open the store, load the family and compare the checkout. Comparing 20,000 files alone takes about 15 ms on the reference machine (`git ls-files` and a `stat` of each), so 20 ms isn't in reach without an index kept between hooks.
- **Since [ADR 0036](0036-a-hook-starts-the-daemon.md):** a hook that finds no daemon starts one, so capture with no daemon happens once for each time the daemon isn't running. It keeps its 50 ms. Meeting that in a process starting cold means loading only what a capture uses:
  - **The log:** the deltas a family's snapshot covers are decoded only when something reads the log, which a capture doesn't.
  - **Alongside the load:** the checkout's recorded entries are read, its worktree is listed, and its attribute check runs, each on threads of their own.
  - **The snapshot:** its file tree and its files decode on threads of their own.

**Starting.** Answering at all takes under 100 ms whatever the daemon holds:
- the socket is open before anything is loaded;
- a ping is answered from the connection, without waiting on the daemon's work;
- other requests wait for the load, ahead of the catch-up rescans it queues.

A thread listing needs every family's history in memory, about 3 ms per MB on a fresh process. So it has the 100 MB/s scale instead: `daemon.ready`.

**Sync.** Handing a long session to another machine is two transfers of its history, each with the import's scale: the push to the server, and the pull with which `silt resume` takes it in. So there are two budgets, each under a second:
- **`sync.push`** is `silt sync --once` on the machine that recorded the session, with its daemon stopped.
- **`sync.pull`** is `silt resume` on another machine, once its daemon lists the thread, up to the restored session the agent starts from. Waiting for the listing isn't timed: it's when the machine hears of the thread, not a transfer.

Holding them means:
- **Batches:** a push carries up to 5,000 deltas or 16 MiB, and a pull answers with as many.
- **Overlap:** a resume fetches the next page while it applies the last, and starts from what the background sync already pulled.
- **The server's index:** written after the push is answered, in push order; search and purges wait for what is pending.

**The hosted app.** The server answers the app from its own copy of each family, so it gets the browser's scale: 100 ms for the thread listing and for a long thread's first page, measured on the server holding the 100 MB family, signed in as a browser is. It keeps two things so it doesn't read whole families on each request:
- **Each thread's message count and first prompt:** kept by its index as lines come in.
- **The deltas each reader may read, with each thread's history entries:** kept with the family's room until it changes, once for every reader who may read the same ones.

The size limits aren't throughput, so they stay as the roadmap first set them.

**`silt-bench` measures them.** It's a binary in its own crate:
- `silt-bench check` runs the enforced budgets, and exits non-zero naming each one that's over, its limit, what was measured and the machine (cores, filesystem).
- `silt-bench report` runs every budget and prints the table.

It drives a real `silt` binary as a subprocess, with its own `SILT_HOME` and `CLAUDE_CONFIG_DIR`, so what it measures is what people run. Only `record.hook_daemonless` and `checkout.rescan` call the capture step in process, as `silt hook` does without a daemon.

**Enforced or pending.**
- Each budget is *pending* until the task that makes it pass lands. That task turns it *enforced*.
- The gate runs only the enforced ones, so it stays green while the work is in flight, and every budget, once met, stays met.
- `web.thread` is checked by a Playwright spec in the e2e run, since only a browser can measure it. `silt-bench report` lists it with where it's checked.
  - The gate runs the e2e suite on the `perf` build, and the spec enforces the budget only there; on a debug build it reports.
  - It's timed in the page, from the click on the thread's link to its first entry being drawn. Playwright's own work before a click lands, about 45 ms, isn't something a person waits for.

**Fixtures.**
- **What they are:** a 100 MB Claude Code transcript, shaped like a real session (prompts, replies, tool calls, and tool results that carry most of the bytes); a git repository of 50,000 files; and a family of ten forks with merges and pulls.
- **How they're made:** they're generated by code, with no randomness, so every run measures the same thing.
- **Caching:** the transcript and the repository are written once into `target/silt-bench/`, under a fixture version that changes when the generator does.
- **Stores:** these are built afresh on every run, through the real recording path, because how they're written is part of what's measured.

**Keeping the gate fast and steady.**
- **The build:** `silt-bench` and `silt` are built in a `perf` profile (release, with incremental builds) into `target/perf`. A debug build's timings say nothing about what people see.
- **Gate time:** with warm fixtures, `check` should add at most a minute to the gate.
- **Noise:** budgets measured in milliseconds take the median of three runs. A budget that is over gets one more run before it fails, and the better of the two stands, as other work on the machine only ever adds to a measurement. Size budgets are exact.
- **Where it's measured:** the reference machine is the development machine, with the checkout on ext4. A cloud session's machine has its own limits ([ADR 0037](0037-cloud-machine-budgets.md)).

## Baseline
The first `silt-bench report` (2026-10-22, 32 cores, ext4) shows what each task starts from:

| Budget | Measured | Limit |
|---|---|---|
| `record.import` | 203 s | < 1 s |
| `record.hook_daemonless` | 172 ms (p95 of `Stop`) | ≤ 50 ms (≤ 20 ms until the split below) |
| `store.size` | 2.90× | ≤ 1.5× |
| `daemon.startup` | 457 ms | < 100 ms |
| `checkout.rescan` | 296 ms | < 200 ms |
| `daemon.idle` | 0.90 % of a core | ≤ 0.1 % |
| `sync.push` | 13.0 s | < 1 s |
| `sync.pull` | 1.43 s | < 1 s |
| `app.threads` | 99 ms | < 100 ms |
| `app.history` | 112 ms | < 100 ms |

The sync and hosted app budgets were measured later the same day, once their bench existed. None is met yet, so the gate's check enforces none of them. Each turns enforced with the task that meets it.

**The split.** The first draft had one hook budget, 20 ms for capture in process. It was split in two on the same day, after measuring:
- hooks forwarded to a running daemon took 2 to 5 ms, process start included;
- capture with no daemon spent about 15 ms just comparing the checkout.

## Consequences
- The gate catches a slowdown the day it lands, by name and with numbers, rather than months later while dogfooding.
- Holding 100 MB/s means designing for batches: one transaction per batch rather than per line, queries that run once per batch, and nothing that grows with the size of the family on the way in.
- The gate needs an optimized build of `silt`. The first run after a clean checkout takes some minutes more; later runs are incremental.
- A budget can't be loosened quietly. Changing one means changing this ADR's table, or a new ADR.
- **Durability:** the store runs SQLite at `synchronous=NORMAL` with its WAL. A crash can't corrupt it, but a power cut can lose the last commits. A transcript's offset is stored in the same commit as its lines, so recording takes those lines in again.
- **The server's durability:** the server runs at `synchronous=NORMAL` too. A power cut there can lose the last pushes it answered, but every start is a new generation, and clients push again what it lacks.
- **Checkpoints:** in the daemon and the running server, a commit no longer copies the WAL into the database itself, as SQLite does every 4 MB by default.
  - A background task copies it once commits pause.
  - A commit still copies it when the WAL passes 64 MB, so it can't grow without bound.
  - The daemon empties the WAL when it stops.
  - So a batch's pages are written once while it's written, rather than twice.
  - Short-lived processes, such as hooks without a daemon, keep SQLite's default. They never run long enough to copy the WAL in the background, and reading through a big WAL made their capture about 60 ms slower.
- **Disk:** the disk, not SQLite, sets the floor. On the reference machine, writing a family's 110 MB of deltas costs 2 to 4 s of fsync when the WAL is checkpointed, and about 0.35 s once they're compressed in chunks of a thousand (5.9 times smaller). So `record.import` depends on storing deltas once, compressed, as `store.size` does too.
- Budgets tied to a machine can fail on a slower one. When the gate runs on CI, this needs revisiting.
