# 0052: Commits tolerate a slower machine, and say so

- Status: accepted
- Date: 2026-10-29
- Amends: [ADR 0034](0034-performance-budgets.md) (how the gate enforces the budgets)
- Amended by: [ADR 0091](0091-a-faster-gate-and-a-pass-that-stands.md): A faster gate, and a pass that stands; [ADR 0131](0131-the-gate-runs-on-the-pull-request.md): The gate runs on the pull request

## Context
Most budgets are measured on the development machine, and that machine is shared with other work: clusters, databases, other agents' builds. On 2026-10-29, `sync.push` measured 1.04 to 1.08 s against its 1 s limit, run after run. The commit being tested changed only the web UI. Its Rust code was the same as a commit that had passed an hour before. Two k3d clusters had started on the same disk in the meantime.

ADR 0034's one extra run absorbs a blip, not a machine that stays busy. So commits that couldn't have changed a budget were blocked until the machine went quiet.

## Decision
- **`silt-bench check --tolerance <percent>`** passes an enforced budget that's over its limit by no more than that share of it. It prints a `slower than budget:` line for each one, with the measurement, the limit and the machine. Above the tolerance, a budget fails as before. The default is 0, so `silt-bench check` and `scripts/gate.sh` run alone stay strict.
- **`scripts/gate.sh`** passes `SILT_BENCH_TOLERANCE` on to it.
- **`scripts/commit.sh`** runs the gate with 50 %. When budgets pass only that way, it prints their lines with a warning to double-check whether the slowdown is expected, rerunning them on an idle machine, and fixing a real one.

The limits themselves don't change, and the report still marks such a budget `OVER`.

## Consequences
- A busy machine no longer blocks commits that don't touch performance.
- A real regression of up to 50 % can land when nobody heeds the warning. The warning names the budget and the numbers, so whoever commits, person or agent, can see it. A regression beyond 50 % still fails.
- Budgets measured as a ratio or a share of a core are loosened by the same share, though only time varies with load. `store.size` would pass up to 2.25×.
