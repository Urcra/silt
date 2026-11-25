# 0091: A faster gate, and a pass that stands

- Status: accepted
- Date: 2026-11-16
- Amends: [ADR 0034](0034-performance-budgets.md) (where the gate runs `web.thread`), [ADR 0052](0052-commits-tolerate-a-slower-machine.md) (how `commit.sh` runs the gate)

## Context
Every commit waits for the gate. Timed in a cloud container with 4 cores, an incremental gate took about 16 minutes, and the first in a fresh clone over 20:

- `cargo test` took 4 minutes, as it runs the hundred test binaries one after another, mostly waiting on a few slow `silt-cli` ones;
- Playwright took 4.6 minutes on one worker, though each spec file sets up its own daemons in a temporary `SILT_HOME` on free ports;
- every web build rewrote `web/dist`, whose mtimes made cargo rebuild `silt-web` and relink every binary that embeds it, even when the UI hadn't changed;
- `target/debug` reached 21 GB, as each test binary carried its own copy of the workspace's debug info, which ran a cloud container out of disk;
- `fmt` and clippy ran after the web build and its tests, so a lint error waited on them;
- `commit.sh` ran the gate silently, then `git add -A`: an edit made while it ran was committed unchecked, and a commit right after running the gate by hand ran it a second time.

## Decision
- **Cheap checks first.** `fmt` (and `fmt` of `crates/silt-voice` and `crates/silt-desktop` where they're built), the web lint, the web build, clippy and the `wasm32` build run before any tests.
- **nextest.** The gate runs the tests with `cargo nextest run --workspace` when it's installed, and `cargo test --workspace --doc` for doctests, which nextest doesn't run; otherwise `cargo test`, as before. `scripts/cloud-setup.sh` installs it. Every test binary runs at once, with twice as many tests as there are cores, as most wait on the daemons and agents they start: 96 s for what took 240 s. Three times as many was faster still, but failed a test already flaky under load.
- **Two Playwright workers.** Spec files run two at a time, a file's tests in order as they share its setup. `perf.spec.ts`, which times `web.thread`, runs alone after the others, as a project that depends on them. `SILT_E2E_WORKERS` sets the count. 3.2 minutes for what took 4.6.
- **An unchanged web build leaves `web/dist` alone.** `pnpm build` builds into `node_modules/.dist-fresh`, and `scripts/sync-dist.mjs` writes into `dist` only the files whose bytes differ, `index.html` last. A gate on an unchanged UI then rebuilds nothing in Rust.
- **Less debug info, split out.** The `dev` profile keeps full debug info for the workspace's crates but only line tables for dependencies, and on Linux `-Csplit-debuginfo=unpacked` leaves it in `.dwo` files rather than linking it into each binary. Measured from clean: `target/debug` 21 → 13 GB, a cold build and test compile 251 → 234 s, and rebuilding after an edit to `silt-crdt` 36 → 21 s. gdb, lldb and backtraces read the `.dwo` files.
- **A cloud container's disk.** A full gate's builds take about 25 GB, and a cloud container has about 40 GB in all, so they ran it out of space. `scripts/cloud-setup.sh` gives the workspace's crates line tables only, in that machine's own cargo config, as nobody steps through code with a debugger there: `target/debug` 16 → 13 GB, and rebuilds a little faster. Turning incremental compilation off would have halved it, but rebuilt 3.5 times slower. The gate warns when under 10 GB is free.
- **Progress and a summary.** The gate names each step with the time so far, and ends with how long each took.
- **A pass stands for its tree.** `scripts/tree-id.sh` prints the tree `git add -A` would commit, using a copy of the index. A full gate that passes, on a tree that didn't change while it ran, writes it to `target/gate-passed`. `commit.sh` commits without running the gate when that matches; otherwise it runs the gate with its steps shown on the terminal and the rest in a log, and refuses to commit if the tree changed meanwhile.
- **`--quick`.** `scripts/gate.sh --quick` runs the formatting, lints, web build and the workspace's tests, for iterating. It records no pass.
- **`silt-bench` says how long each scenario took**, so its share of the gate can be worked on. It showed most of its three minutes went to starting a session on the 50,000-file checkout, twice, which built git's attribute lookup for every file; `silt_git::Repo::filters` now keeps it (36 s to 16 s, and the budgets 148 s to 87 s).
- **Fixtures clean up after a failure.** Playwright's `afterAll` hooks stopped at the first error, such as a daemon a test had stopped already, and left servers, daemons and their data behind, which once filled a cloud container's disk. `teardown` in `web/e2e/fixture.ts` runs every step.

## Consequences
- An incremental gate takes minutes less, and a lint error shows within about two.
- A commit still never skips the gate: it lands only a tree a full gate passed on, now including edits made while it ran, which it used to commit unchecked.
- A pass from `scripts/gate.sh` run alone held the budgets to their limits, stricter than `commit.sh`'s 50 % (ADR 0052), so `commit.sh` can take it as its own.
- nextest runs each test in its own process. A fake agent's write to a hook that exits without reading its input (`true`) raced, and failed with a broken pipe; the fakes now ignore that, as the agents do.
- Two Playwright workers load the machine more. In five runs, one screen reference failed once; `SILT_E2E_WORKERS=1` brings back the old order.
- The gate falls back to `cargo test` without nextest, so a machine without it is only slower.
- Stepping through a dependency with its variables needs `debug = true` for it under a `[profile.dev.package.<crate>]` of one's own.
