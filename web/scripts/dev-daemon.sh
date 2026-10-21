#!/bin/sh
# Runs the Vite dev server with `/api` proxied to the running daemon, so UI
# work doesn't need `silt` rebuilt. `SILT` names the binary to ask, by
# default the one on PATH.
set -eu
cd "$(dirname "$0")/.."
url=$("${SILT:-silt}" open --no-browser)
SILT_DAEMON_URL="${url%%/#*}"
export SILT_DAEMON_URL
port="${PORT:-5173}"
echo "silt: open http://localhost:$port/#${url#*#}"
exec npx vite --port "$port" --strictPort
