# 0002: Git: cheap checkouts via alternates, gix filters, git CLI for writes

- Status: accepted
- Date: 2026-10-20. Revised the same day after the gix filter and checkout probes (docs/research/probes.md).

## Context
Every thread needs its own checkout. That covers resume targets (`<repo>/.silt/checkouts/<thread>`), forks, and MR review. Checkouts must:
- be cheap to create, even for large or shallow repos;
- behave exactly like the user's own git (credentials, LFS, eol and filters);
- survive whatever the user does to their source clone, including gc.

A separate git dir that borrows the source's objects through alternates makes checkouts cheap.

## Decision
**Cheap checkout (strategy B).** Everything is created with the git CLI:
1. `git init --separate-git-dir=$SRC/.git/silt/clones/<id>.git <checkout>`. The checkout gets a `.git` *file*. Keeping the git dir inside the source's `.git` means the source can never stage it.
2. Write `objects/info/alternates` pointing at `$SRC/.git/objects`. Copy `.git/shallow` when the source is shallow.
3. Set `core.alternateRefsCommand = : "$@"` and `core.worktree = <checkout>`.
4. **Do not copy the source's refs.** Only HEAD plus `refs/silt/*`.
5. Remotes:
   - `origin`: the source's origin URL.
   - `local`: `$SRC/.git`, with `remote.local.receivepack = git -c receive.denyCurrentBranch=updateInstead receive-pack`, so `git push local <branch>` updates a clean checked-out branch in place.
6. Detach HEAD at the base, then materialize with `git -c checkout.workers=8 reset --hard <base>`.
7. Add `/.silt/` to `$SRC/.git/info/exclude`. Managed checkouts live in `<repo>/.silt/checkouts/<thread>`.

**Gc safety.**
- For every live thread, the source keeps `refs/silt/threads/<id>`, pointing at its base and advanced by pushing thread commits. This was verified to survive `branch -D`, reflog expiry, `gc --prune=now` and `repack -ad`.
- The ref is deleted when the thread's checkout is discarded.
- Health checks use `git fsck --connectivity-only` or `rev-list`, never `git status` alone.

**Fallbacks.**
- No local source clone (e.g. the first resume on a new machine): clone from the Silt server's git mirror, or from `origin` plus the mirror for local-only commits, then use B against that clone.
- If the user opts out of `refs/silt/*`: `git clone --local` (strategy A).

**Clean and smudge.**
- `gix` 0.89 `Pipeline` (`attributes` feature), one per worker and reused. It matched git on 244 of 256 cases, including process filters such as git-lfs.
- Paths whose attributes set a non-UTF-8 `working-tree-encoding`, or `ident`, are sent to git instead:
  - smudge: `git cat-file --filters --path`
  - clean: `git hash-object --path --stdin`
- The CRDT keeps clean blob bytes as the source of truth and never re-cleans its own smudged output.

**Other operations.**
- Writes (fetch, push, commit) use the `git` CLI.
- Hot-path reads (trees, blobs, merge-base) use gix.
- The server holds a bare mirror per project with `refs/silt/<thread>/{base,head,checkpoint}`.

## Consequences
- Creating a checkout takes about 10 ms plus materialization. Each git dir is a few hundred KB, and it immediately sees objects the source fetches later.
- Correctness depends on the `refs/silt/threads/*` refs staying in the source. The daemon re-asserts them on start and before any destructive operation, and `silt doctor` checks connectivity.
- Two code paths for filters (gix plus the git fallback). Both are covered by the gix filter probe's test matrix, which is turned into fixtures.
