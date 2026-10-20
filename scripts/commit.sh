#!/bin/sh
# Runs the gate and only then commits everything staged and unstaged and
# pushes, so a commit can never skip the gate. Usage: scripts/commit.sh <message>
set -eu
cd "$(dirname "$0")/.."
log=$(mktemp)
if ! ./scripts/gate.sh >"$log" 2>&1; then
    grep -E '^(error|warning)|FAILED|panicked' -A6 "$log" | head -40
    echo "gate failed; nothing committed (full log: $log)" >&2
    exit 1
fi
rm -f "$log"
git add -A
git commit -q -m "$1"
git push -q
git log --oneline -1
