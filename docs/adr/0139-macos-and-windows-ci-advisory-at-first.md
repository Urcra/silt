# 0139: macOS and Windows CI, advisory at first

- Status: accepted
- Date: 2026-11-27
- Amends: [ADR 0119](0119-no-windows-ci-or-releases.md) (Windows has a CI workflow again), [ADR 0131](0131-the-gate-runs-on-the-pull-request.md) (what `/claude merge` waits for)
- Builds on: [ADR 0045](0045-windows-natively-and-in-wsl.md), [ADR 0129](0129-nightly-releases-and-three-channels.md)

## Context
ADR 0119 removed Windows' CI workflow because Windows minutes cost money and every Windows job had failed before starting. ADR 0129 put Windows back in releases once the repository was public and those minutes were free, but left CI waiting for "a Windows workflow that passes". The nightly has built on `windows-2025` and `macos-15` since then, so the runners work.

Nothing has run the tests on Windows since 2026-11-14, and they have never run on macOS. A build that compiles there can still fail its tests. Only the nightly would show a break, a day late, and only a break in the build.

`/claude merge` waits for every check run on the head commit and merges only if each passed. A macOS or Windows job added there would block every merge until the code is fixed on both platforms.

## Decision
**`.github/workflows/platforms.yml` runs on every pull request and on `main`.**
- On `macos-15` and `windows-2025`: clippy with `-D warnings`, the workspace's tests, `silt-cli`'s integration tests (several machines on a real server, with the agent CLIs' stand-ins), and the desktop app's clippy and tests.
- On Windows, a WSL 1 distribution beside the native `silt`, as `windows.yml` had: `scripts/wsl-handoff.sh` hands a thread each way, and the desktop app starts both backends. The distribution's `silt` is built on Linux in the same run.
- Linux keeps the whole gate in `ci.yml`. Perf, the end-to-end tests, wasm, the licences and the docs site aren't repeated.

**Advisory at first.** Every job is named `... (advisory)`, and `/claude merge` (`.github/scripts/claude.sh`) neither waits for nor counts a check with that name. The jobs are in a workflow of their own, so a red one leaves `ci.yml`'s run green, and `/claude merge` can still reuse the gate that run passed.

**Required once green.** When the jobs pass on `main`, they drop the `(advisory)` suffix and `/claude merge` waits for them like any other check. That needs its own decision.

## Consequences
- A pull request shows whether it broke macOS or Windows before it's merged, even though nothing stops the merge yet.
- A pull request's checks take longer to finish, as Windows builds slowly. `/claude merge` doesn't wait for them.
- The gate's clippy cross-check for `x86_64-pc-windows-gnu` and Wine stay for checking locally.
- `claude.sh` comes from the default branch, so the advisory rule applies to pull requests only once this change is on `main`.
