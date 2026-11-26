#!/bin/sh
# Checks that the pnpm the web UI pins runs here, and says how to make it
# run when it doesn't.
set -eu
cd "$(dirname "$0")/.."

command -v node > /dev/null || { echo "Building the web UI needs Node.js (see docs/contributing.md)" >&2; exit 1; }
command -v pnpm > /dev/null || { echo "Building the web UI needs pnpm: \`corepack enable pnpm\`, or \`npm install -g pnpm\` (see docs/contributing.md)" >&2; exit 1; }

# Corepack would ask before fetching it, where the question can't be seen.
COREPACK_ENABLE_DOWNLOAD_PROMPT=0 pnpm --version > /dev/null 2>&1 && exit 0

pinned=$(node -p 'require("./package.json").packageManager.split("@")[1]')
cache="${COREPACK_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/node/corepack}/v1/pnpm/$pinned"
# Corepack before 0.34.5 starts pnpm by bin/pnpm.cjs, which pnpm 12 no
# longer has, and records that path beside the pnpm it fetched.
if corepack=$(corepack --version 2> /dev/null) \
    && node -e 'const [a, b, c] = process.argv[1].split(".").map(Number); process.exit(a > 0 || b > 34 || (b === 34 && c >= 5) ? 1 : 0)' "$corepack"; then
    echo "pnpm $pinned needs corepack 0.34.5 or newer, and this one is $corepack:" >&2
    echo "    npm install -g corepack@0.34    # or corepack@latest on Node 22.22.2 or newer" >&2
    echo "    rm -rf $cache" >&2
    echo "    corepack enable pnpm" >&2
else
    echo "pnpm $pinned doesn't run: \`pnpm --version\` fails here." >&2
    echo "If an older corepack fetched it, \`rm -rf $cache\` has it fetched again (see docs/contributing.md)." >&2
fi
exit 1
