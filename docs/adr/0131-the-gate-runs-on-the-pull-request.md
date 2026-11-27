# 0131: The gate runs on the pull request

- Status: accepted
- Date: 2026-11-26
- Amends: [ADR 0091](0091-a-faster-gate-and-a-pass-that-stands.md) (a pass no longer stands for a commit), [ADR 0052](0052-commits-tolerate-a-slower-machine.md) (no commit runs the gate)
- Builds on: [ADR 0130](0130-the-whole-gate-on-every-pull-request.md)
- Amended by: [ADR 0138](0138-macos-and-windows-ci-advisory-at-first.md): macOS and Windows CI, advisory at first

## Context
Every commit by a maintainer, or by an agent working for one, went through `scripts/commit.sh`, which ran the full gate first unless it had already passed on that exact tree. That took 20 to 30 minutes per commit in a cloud session, and an edit made meanwhile meant running it again.

Pull requests now get the same gate elsewhere. Since ADR 0130, CI runs the gate's three parts on every push to a pull request, and `.github/scripts/claude.sh` merges only a head that passed it. Outside contributors already worked this way: they committed with git and the pull request was gated. The local gate held maintainers to a second, slower copy of a check their pull requests get anyway.

## Decision
- **Nobody has to run the gate locally.** Maintainers and their agents commit with git and push, like everyone else. The pull request's gate decides whether a change is done, whoever wrote it.
- **`scripts/commit.sh` is gone**, and with it `scripts/tree-id.sh` and the pass `scripts/gate.sh` recorded in `target/gate-passed`, which only `commit.sh` read.
- **`scripts/gate.sh` stays**, as what CI and `/claude gate` run, and for running locally before pushing a change likely to fail it. `--quick` covers formatting, lints and the tests in a few minutes.
- A task is done when its pull request's gate passes.

## Consequences
- A commit is a commit again: seconds rather than half an hour.
- A branch can now hold commits that fail the gate. Only what merges into `main` is held to it, as it already was for outside contributors.
- A failure shows up on the pull request rather than before the push. Running `scripts/gate.sh` locally still catches it first, for those who want to. The desktop app and the speech helper stay local checks (ADR 0130), so a change to them is still worth a local gate.
- `SILT_BENCH_TOLERANCE` stays in the gate, but nothing sets it for a local run any more; a busy machine sets it itself (ADR 0052).
