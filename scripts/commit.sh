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
    if ! ./scripts/gate.sh >"$log" 2>&1; then
        grep -E '^(error|warning)|FAILED|panicked' -A6 "$log" | head -40
        echo "gate failed; nothing committed (full log: $log)" >&2
        exit 1
    fi
    rm -f "$log"
fi
git add -A
git commit -q -m "$1"
git push -q
git log --oneline -1
