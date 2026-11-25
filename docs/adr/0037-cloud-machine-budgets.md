# 0037: Budgets for a cloud session's machine

- Status: accepted
- Date: 2026-10-22
- Amends: [ADR 0034](0034-performance-budgets.md) (where budgets are measured)
- Amended by: [ADR 0126](0126-budgets-for-a-github-actions-runner.md) (a GitHub Actions runner's limits)

## Context
ADR 0034 set every budget on the development machine (32 cores, ext4) and left them unscaled for slower machines. Silt is also worked on from Claude Code cloud sessions: a KVM guest with 4 vCPUs at 2.1 GHz and 15 GiB. There, `silt-bench check` fails on ten of twelve budgets with no regression, so the gate can never pass and nothing can be committed through `scripts/commit.sh`.

The slowdown isn't one factor. Over three runs on that machine:

| Budget | Measured (three runs) | Dev limit |
|---|---|---|
| `record.import` | 2.09 s, 5.99 s, 2.51 s | < 1 s |
| `record.hook` | 46.0, 50.9, 50.7 ms | < 20 ms |
| `record.hook_daemonless` | 593, 574, 566 ms | < 200 ms |
| `daemon.ready` | 0.999, 1.22, 1.12 s | < 1 s |
| `checkout.rescan` | 606, 616, 653 ms | < 200 ms |
| `daemon.idle` | 0.13 % of a core, each run | ≤ 0.1 % |
| `sync.push` | 1.73, 1.92, 2.01 s | < 1 s |
| `sync.pull` | 2.54, 5.37, 5.10 s | < 1 s |
| `app.threads` | 157, 105, 146 ms | < 100 ms |
| `app.history` | 141, 178, 148 ms | < 100 ms |
| `web.thread` shown | 160, 141, 128 ms | < 100 ms |

The first run is `check`'s, the better of two; the others are single `report` runs. The import and the pull vary nearly threefold between runs, which looks like other guests on the host.

## Decision
**Two sets of limits.** Each budget has a dev limit, ADR 0034's, and a cloud limit. A cloud limit is about 1.3 times the worst of the three runs above, rounded, and is never tighter than the dev limit:

| Budget | Cloud limit |
|---|---|
| `record.import` | < 4 s |
| `import.session` | < 4 s, as `record.import` |
| `record.hook` | < 70 ms |
| `record.hook_daemonless` | < 230 ms |
| `daemon.ready` | < 1.6 s |
| `checkout.rescan` | < 850 ms |
| `daemon.idle` | ≤ 0.2 % of a core |
| `sync.push` | < 2.6 s |
| `sync.pull` | < 7 s |
| `app.threads` | < 200 ms |
| `app.history` | < 230 ms |
| `page.fold` | < 25 ms |
| `doc.replay` | < 40 ms |
| `sheet.eval` | < 50 ms |
| `web.thread` | shown < 210 ms |

`record.hook_daemonless` was measured again once capture with no daemon got faster (ADR 0036): 117.7, 120.9 and 109.1 ms against a dev limit of 50 ms, so its cloud limit was set to 160 ms, not the 800 ms the runs above gave. Another cloud container then measured 167.2, 170.1 and 176.3 ms on the same code: containers differ by half as much again, so the limit is 230 ms, about 1.3 times that container's worst.

The import's limit sits under its worst run: `check` takes the better of two runs, and the other two were near 2 s. `store.size`, `daemon.startup` and the `web.thread` heap keep their dev limits, which hold on the cloud machine with room to spare.

`page.fold`, `doc.replay` and `sheet.eval` came later, with page data, docs and sheets. A cloud container measured 3.8–6.0, 10.4–12.0 and 14.1–15.2 ms over three runs, and their limits are about three times that: they run in process while the other scenarios' servers and daemons wind down.

**Choosing.** `SILT_BENCH_MACHINE=dev|cloud` picks the limits. Unset, a cloud session (`CLAUDE_CODE_REMOTE=true`) gets the cloud ones and every other machine the dev ones, so the development machine needs no setup. `silt-bench` and `web/e2e/perf.spec.ts` choose the same way, and a failure names the limits it was held to.

**The dev limits stay the targets.** The cloud limits catch a regression made from a cloud session. They don't say Silt is fast enough: a task meets a budget on the development machine.

## Consequences
- The gate can pass in a cloud session, so `scripts/commit.sh` works there.
- The cloud limits catch large regressions in the import and the pull, and smaller ones in the hooks, rescans and idle CPU, which were steady.
- A change to a cloud limit is a change to this table, as for the dev ones.
- If cloud machines change, the limits are measured again: `silt-bench report` run three times there.
- `store.size` read 0.56×, 0.65× and 0.60× over the three runs, though ADR 0034 treats sizes as exact. It is within its limit either way; why it varies is open.
