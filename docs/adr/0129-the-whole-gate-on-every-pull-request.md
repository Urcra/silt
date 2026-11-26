# 0129: The whole gate on every pull request

- Status: accepted
- Date: 2026-11-26

## Context
CI (`.github/workflows/ci.yml`) ran part of `scripts/gate.sh` on each pull request: formatting, licences, lints, clippy, the wasm32 build, the web UI's and plugins' tests and the workspace's tests. The budgets and the end-to-end tests ran only when someone with write access asked for `/claude gate` or `/claude merge` (`.github/workflows/claude.yml`). Those refuse a pull request from a fork, as Claude can't push there. So an outside contributor's change was never gated on a runner, and a maintainer had to run the rest locally before merging it by hand.

GitHub also held each fork's runs until a maintainer approved them. CI needs neither secrets nor write access, so approving them guarded nothing.

## Decision
**CI runs the gate's three parts.** `ci.yml` is one job per part (`scripts/gate.sh --part checks|tests|perf`), set up as `claude.yml`'s gate jobs are, with `SILT_SKIP_VOICE=1` as there. CI and the gate can't drift apart, since CI runs the gate script itself rather than its own list of steps. The desktop app and the speech helper stay local checks, as on `/claude gate`.

**A fork's runs start without approval**, except for accounts that are new to GitHub, the least approval GitHub allows on a public repository. That's safe because `ci.yml` stays on `pull_request`: a fork's run gets a read-only token and no secrets, and its caches are kept on its own ref, where the default branch never reads them. `ci.yml` must never need a secret or a write permission, nor move to `pull_request_target` or `workflow_run`.

**`claude.yml` keeps its own gate, and skips it when CI already ran it.** Its gate jobs run only on branches of this repository, which only people with write access push, so they get no code that isn't as trusted as the default branch. `/claude merge` gates the head merged with the base as it is at that moment. CI gates the merge GitHub made at the push, which may be older. Each of CI's gate jobs notes the parents of the merge commit it tested (`::notice title=gated::head <sha>, base <sha>`). When the newest passing CI run on the PR's head notes the current head and base in all three parts, `merge` and `gate` take that as their first round and don't run it again. They don't when the PR changes `.github/`, as a pull request's CI runs its own `ci.yml`, which could then claim a gate it didn't run. The fix rounds and the gates after them still run in `claude.yml`, which keeps the failures Claude fixes from.

**On a fork's pull request, Claude reviews and answers with a token that can't push.** `claude.sh` already refused `fix`, `merge`, `gate` and `review fix` there. `claude-code-action` skipped the rest by default, so `/claude review` and `/claude ask` said nothing at all. They now run with `allow_fork_prs`. The Claude app's token is narrowed to `contents: read`, so it can post comments and reviews but not push or merge, and Claude doesn't get `gh issue edit`. The prompt says that the fork's diff and text are an outsider's data. A prompt injection in a fork could at worst post or edit comments. `/claude review dangerously full`, for the repository's owner only, gives Claude the token and tools of a review on a member's pull request. Neither builds or runs the fork's code.

## Consequences
- An outside contributor sees the whole gate's result on their pull request, and a maintainer can merge it on that, with Claude's review if asked.
- Every pull request takes about as long in CI as `/claude gate`, and each part takes a runner. GitHub's standard runners cost nothing on a public repository.
- `/claude merge` on a PR whose base hasn't moved since its last push merges as soon as CI has passed, without a gate of its own. When the base has moved, or a fix round pushed, the change is gated twice, in `claude.yml` and in CI.
- The budgets now run on every pull request, so the runner variance that ADR 0126 sets the CI limits by can fail a change that didn't cause the slowdown. If that happens, the limit is measured again, as ADR 0126 says.
- The checks are named `Gate (checks)`, `Gate (tests)` and `Gate (perf)`, not `lint` and `test`. A branch rule that requires the old names needs the new ones.
