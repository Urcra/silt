#!/bin/sh
# Prints the id of the tree `git add -A` would commit, without touching the
# index: what the gate checked, so scripts/commit.sh can tell whether the
# tree changed since.
set -eu
cd "$(dirname "$0")/.."
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
# A copy of the real index keeps its stat cache, so unchanged files aren't
# hashed again.
cp "$(git rev-parse --git-path index)" "$scratch/index" 2>/dev/null || true
GIT_INDEX_FILE="$scratch/index" git add -A
GIT_INDEX_FILE="$scratch/index" git write-tree
