# 0126: Budgets for a GitHub Actions runner

- Status: accepted
- Date: 2026-11-25
- Amends: [ADR 0037](0037-cloud-machine-budgets.md) (a third set of limits, and how a machine is chosen), [ADR 0034](0034-performance-budgets.md) (`report` never fails)

## Context
`/claude merge` and `/claude gate` run the full gate on GitHub Actions, the perf part on its own runner (`ubuntu-24.04`: 4 vCPUs, 16 GB, ext4 reported as ext2/ext3). The runner isn't the development machine or a cloud session's, so the gate ran there with `SILT_BENCH_ENFORCE=0`, reporting the budgets rather than holding them. That held nothing: a regression showed only to someone who read the log.

It also failed anyway. `silt-bench report` exited with failure whenever an enforced budget was over its limit, so a pull request that only bumps a dependency failed at the budgets on a runner, and its fix rounds were spent on a slowdown nobody made.

Two runs on runners, from that pull request, on the same code; the second is `check`'s, the better of two where a budget was over:

| Budget | First runner | Second runner | Dev limit |
|---|---|---|---|
| `record.import` | 939 ms | 1.75 s | < 1 s |
| `import.session` | 1.05 s | 1.85 s | < 1 s |
| `record.hook` | 14.0 ms | 27.4 ms | < 20 ms |
| `record.hook_daemonless` | 101 ms | 169 ms | < 50 ms |
| `store.size` | 0.57× | 0.57× | ≤ 1.5× |
| `daemon.startup` | 5.8 ms | 7.0 ms | < 100 ms |
| `daemon.ready` | 317 ms | 432 ms | < 1 s |
| `checkout.rescan` | 210 ms | 397 ms | < 200 ms |
| `daemon.idle` | 0.07 % of a core | 0.08 % | ≤ 0.1 % |
| `sync.push` | 1.10 s | 2.28 s | < 1 s |
| `sync.pull` | 1.30 s | 1.92 s | < 1 s |
| `app.threads` | 76.5 ms | 88.4 ms | < 100 ms |
| `app.history` | 1.5 ms | 1.2 ms | < 100 ms |
| `page.fold` | 4.9 ms | 5.2 ms | < 10 ms |
| `doc.replay` | 5.6 ms | 6.3 ms | < 15 ms |
| `sheet.eval` | 11.5 ms | 11.7 ms | < 20 ms |

Runners differ: the second was up to twice as slow as the first on the import, the hooks, the rescan and the push, much as cloud containers differ (ADR 0037). Even the slower one is faster than a cloud session's machine on most budgets, so the cloud limits would let a large regression through.

## Decision
**A third set of limits, CI's.** Each budget has a CI limit, about 1.3 times the slower runner's measurement and about twice the faster one's, rounded, and never tighter than the dev limit:

| Budget | CI limit |
|---|---|
| `record.import`, `import.session` | < 2.4 s |
| `record.hook` | < 40 ms |
| `record.hook_daemonless` | < 230 ms |
| `checkout.rescan` | < 520 ms |
| `daemon.idle` | ≤ 0.2 % of a core |
| `sync.push` | < 3 s |
| `sync.pull` | < 2.6 s |
| `app.threads` | < 200 ms |
| `page.fold` | < 15 ms |
| `sheet.eval` | < 25 ms |
| `web.thread` | shown < 210 ms, as on a cloud machine |
| `web.scroll` | < 1 s blocked, as on a cloud machine |

The rest keep their dev limits, which the runner met with room to spare. The web budgets weren't measured on a runner yet, as the gate stopped before the end-to-end tests, so they take the cloud limits until they are.

**Choosing.** `SILT_BENCH_MACHINE` takes `ci` as well as `dev` and `cloud`. Unset, a GitHub Actions runner (`GITHUB_ACTIONS=true`) gets the CI limits, then a cloud session the cloud ones, and anything else the dev ones. `silt-bench` and `web/e2e/perf.spec.ts` choose alike, and the gate on a runner holds the budgets with no `SILT_BENCH_ENFORCE=0`.

**The dev limits stay the targets, and a deviation from one is said.** On any machine but the development one, a budget measured over its dev limit prints an `off target:` line. It doesn't fail. On a runner the line is a warning annotation on the job, `silt-bench` writes its table, with each budget's dev target beside its limit, to the job's summary, and the status comment of `/claude merge` and `/claude gate` counts the budgets off target beside the perf part.

**`report` only reports.** `silt-bench report` exits with success whatever it measured; only `check` fails.

## Consequences
- The gate on a runner catches a regression of a third or more on a slow runner, and of about twice on a fast one, and the run says which budgets are off their targets, so a slowdown is visible without failing an unrelated change.
- The limits were first set at twice the faster run alone; the slower runner then failed `sync.push` at 2.28 s against 2.2 s with no change to explain it, which set them as above. If a runner fails a budget again with no change to explain it, its limit is measured again and this table changes.
- GitHub may change its runners; the summary's table shows when the measurements move.
- `SILT_BENCH_ENFORCE=0` remains for any other slower machine.
