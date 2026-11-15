#!/bin/sh
# One machine with no server: a local model, served by silt-fake-llama, a
# clone of a project in a Local only workspace, and the daemon's web UI.
# Prints the details as one JSON line.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
silt="$root/target/debug/silt"
work=$(mktemp -d /tmp/silt-local-XXXXXX)

git init -q -b main "$work/logs-seed"
printf 'ERROR disk full\n' > "$work/logs-seed/app.log"
git -C "$work/logs-seed" add .
git -C "$work/logs-seed" -c user.name=t -c user.email=t@example.com commit -q -m init
git clone -q --bare "$work/logs-seed" "$work/logs.git"

export SILT_HOME="$work/me/silt"
export SILT_VOICE_BIN="${SILT_VOICE_BIN:-/nonexistent}"
export SILT_LLAMA_SERVER="$root/target/debug/silt-fake-llama"
export CLAUDE_CONFIG_DIR="$work/me/claude"
export HOME="$work/me/home"
unset CODEX_HOME GEMINI_CLI_HOME COPILOT_HOME XDG_DATA_HOME XDG_CONFIG_HOME
mkdir -p "$HOME" "$CLAUDE_CONFIG_DIR" "$SILT_HOME"
git clone -q "$work/logs.git" "$work/me/incidents"
# A GGUF header is all the fake server needs of a model.
python3 - "$work/Tiny.gguf" <<'PY'
import struct, sys
entries = [("general.architecture", "llama"), ("general.name", "Tiny")]
data = b"GGUF" + struct.pack("<IQQ", 3, 0, len(entries))
for key, value in entries:
    data += struct.pack("<Q", len(key)) + key.encode() + struct.pack("<IQ", 8, len(value)) + value.encode()
open(sys.argv[1], "wb").write(data)
PY
cd "$work/me/incidents"
"$silt" model pull "$work/Tiny.gguf" > /dev/null
"$silt" workspace new Incidents --local-only --folder "$work/me/incidents" > /dev/null
url=$("$silt" open --no-browser)

printf '{"url":"%s","dir":"%s"}\n' "$url" "$work"
