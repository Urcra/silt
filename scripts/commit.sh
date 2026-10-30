#!/bin/sh
# Runs the gate and only then commits everything staged and unstaged and
# pushes, so a commit can never skip the gate. A change to Markdown alone
# skips it: no build or test reads those, save the generated CLI reference.
# Usage: scripts/commit.sh <message>
set -eu
cd "$(dirname "$0")/.."
changed=$(git status --porcelain --untracked-files=all | cut -c4- | sed 's/.* -> //')
if [ -n "$changed" ] && ! printf '%s\n' "$changed" | grep -qv -e '\.md$' && ! printf '%s\n' "$changed" | grep -qx 'docs/reference/cli\.md'; then
    echo "only Markdown changed; skipping the gate"
else
    log=$(mktemp)
    # A busy machine slows the budgets down, so a commit passes one up to
    # half over its limit, and says so (ADR 0052).
    if ! SILT_BENCH_TOLERANCE=50 ./scripts/gate.sh >"$log" 2>&1; then
        grep -E '^(error|warning)|FAILED|panicked|^over budget' -A6 "$log" | head -40
        echo "gate failed; nothing committed (full log: $log)" >&2
        exit 1
    fi
    if grep -q '^slower than budget' "$log"; then
        grep '^slower than budget' "$log" >&2
        echo "warning: the gate passed, but the budgets above ran over their limits." >&2
        echo "Double-check whether that increase in runtime is expected: rerun them with" >&2
        echo "\`target/perf/silt-bench check --only <budget>\` on an idle machine, and fix a real slowdown." >&2
    fi
    rm -f "$log"
fi
git add -A
git commit -q -m "$1"
git push -q
git log --oneline -1
