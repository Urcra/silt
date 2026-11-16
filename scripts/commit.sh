#!/bin/sh
# Commits everything, staged and unstaged, and pushes, once the gate has
# passed on exactly that tree, so a commit can never skip the gate: when
# scripts/gate.sh already passed on it, that pass stands; otherwise the gate
# runs now, showing each step, and a tree edited while it ran isn't
# committed. A change to Markdown alone skips the gate: no build or test
# reads those, save the generated CLI reference.
# Usage: scripts/commit.sh <message>
set -eu
cd "$(dirname "$0")/.."
tree=$(scripts/tree-id.sh)
changed=$(git status --porcelain --untracked-files=all | cut -c4- | sed 's/.* -> //')
if [ -n "$changed" ] && ! printf '%s\n' "$changed" | grep -qv -e '\.md$' && ! printf '%s\n' "$changed" | grep -qx 'docs/reference/cli\.md'; then
    echo "only Markdown changed; skipping the gate"
elif [ "$(cat target/gate-passed 2>/dev/null)" = "$tree" ]; then
    echo "the gate already passed on this tree; committing it"
else
    log=$(mktemp)
    # A busy machine slows the budgets down, so a commit passes one up to
    # half over its limit, and says so (ADR 0052). The gate names each step
    # on fd 3, which goes to the terminal.
    if ! SILT_BENCH_TOLERANCE=50 ./scripts/gate.sh 3>&2 >"$log" 2>&1; then
        # Not warnings: clippy's are errors already, and the rest are noise.
        grep -E '^error|FAILED|FAIL |panicked|^over budget|✘|No space left' -A6 "$log" | head -40
        echo "gate failed at \"$(grep '^gate: \[' "$log" | tail -1 | sed 's/^gate: \[[^]]*\] //')\"; nothing committed (full log: $log)" >&2
        exit 1
    fi
    if grep -q '^slower than budget' "$log"; then
        grep '^slower than budget' "$log" >&2
        echo "warning: the gate passed, but the budgets above ran over their limits." >&2
        echo "Double-check whether that increase in runtime is expected: rerun them with" >&2
        echo "\`target/perf/silt-bench check --only <budget>\` on an idle machine, and fix a real slowdown." >&2
    fi
    if [ "$(cat target/gate-passed 2>/dev/null)" != "$tree" ]; then
        echo "the tree changed while the gate ran, so what it checked isn't what would be committed; nothing committed" >&2
        echo "run scripts/commit.sh again (full log: $log)" >&2
        exit 1
    fi
    rm -f "$log"
fi
git add -A
if [ "$(git write-tree)" != "$tree" ]; then
    echo "the tree changed just now, after the gate; nothing committed (it's staged: run scripts/commit.sh again)" >&2
    exit 1
fi
git commit -q -m "$1"
git push -q
git log --oneline -1
