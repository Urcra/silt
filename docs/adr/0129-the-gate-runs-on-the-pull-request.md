# 0129: The gate runs on the pull request

- Status: accepted
- Date: 2026-11-26
- Amends: [ADR 0091](0091-a-faster-gate-and-a-pass-that-stands.md) (a pass no longer stands for a commit), [ADR 0052](0052-commits-tolerate-a-slower-machine.md) (no commit runs the gate)

## Context
Every commit by a maintainer, or by an agent working for one, went through `scripts/commit.sh`, which ran the full gate first unless it had already passed on that exact tree. That took 20 to 30 minutes per commit in a cloud session, and an edit made meanwhile meant running it again.

Pull requests now get the same checks elsewhere. CI (`.github/workflows/ci.yml`) runs formatting, licences, lints, the `wasm32` build and the tests on every push. `/claude gate` and `/claude merge` run the full gate, its three parts on their own runners, on the head merged with its base, with budgets set for a runner (ADR 0126), and `.github/scripts/claude.sh` merges only a head that passed it and CI. Outside contributors already worked this way: they committed with git and the pull request was gated. The local gate held maintainers to a second, slower copy of a check their pull requests get anyway.

## Decision
- **Nobody has to run the gate locally.** Maintainers and their agents commit with git and push, like everyone else. CI runs on the pull request, and the full gate before it merges, whoever wrote it.
- **`scripts/commit.sh` is gone**, and with it `scripts/tree-id.sh` and the pass `scripts/gate.sh` recorded in `target/gate-passed`, which only `commit.sh` read.
- **`scripts/gate.sh` stays**, for running locally when a change is likely to fail a part CI doesn't run, such as the end-to-end tests or the budgets, and as what `/claude gate` runs. `--quick` covers about what CI does in a few minutes.
- A task is done when its pull request's CI and full gate pass.

## Consequences
- A commit is a commit again: seconds rather than half an hour.
- A branch can now hold commits that fail the gate. Only what merges into `main` is held to it, as it already was for outside contributors.
- A failure in a part CI doesn't run, the end-to-end tests, the budgets, the desktop app or the speech helper, shows up at `/claude gate` or `/claude merge` rather than before the push. Running `scripts/gate.sh` locally still catches it first, for those who want to.
- `SILT_BENCH_TOLERANCE` stays in the gate, but nothing sets it for a local run any more; a busy machine sets it itself (ADR 0052).
