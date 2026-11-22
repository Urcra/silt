#!/bin/sh
# One machine with no server and one thread with a lot in it, recorded with
# fake-claude-busy.sh. Prints the web UI's URL, the thread and the work dir
# as one JSON line.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
silt="$root/target/debug/silt"
# As screens-fixture.sh: no background answer may land in a screen.
export SILT_ASK_AGENTS=0
work=$(mktemp -d /tmp/silt-busy-XXXXXX)
# A failure partway leaves nothing running or on disk.
trap 'status=$?; [ "$status" -eq 0 ] || "$root/web/e2e/abandon.sh" "$work" "$silt"; exit "$status"' EXIT

git init -q -b main "$work/seed"
mkdir -p "$work/seed/src"
printf 'pub fn upload(data: &[u8]) {\n    let read = data.len();\n    send(data);\n    progress(read);\n}\n' > "$work/seed/src/upload.rs"
git -C "$work/seed" add .
git -C "$work/seed" -c user.name=t -c user.email=t@example.com commit -q -m init
git clone -q --bare "$work/seed" "$work/origin.git"
git clone -q "$work/origin.git" "$work/repo"

export SILT_HOME="$work/silt"
export SILT_VOICE_BIN="${SILT_VOICE_BIN:-/nonexistent}"
export CLAUDE_CONFIG_DIR="$work/claude"
# Agent sessions on the machine running the tests would show in the import
# offer, so each machine here has a home of its own.
export HOME="$work/home"
unset CODEX_HOME GEMINI_CLI_HOME COPILOT_HOME XDG_DATA_HOME XDG_CONFIG_HOME
mkdir -p "$HOME"
export SILT_CLAUDE_BIN="$root/web/e2e/fake-claude-busy.sh"
export SILT_HOOK="$silt hook"
mkdir -p "$SILT_HOME" "$CLAUDE_CONFIG_DIR"
cd "$work/repo"
"$silt" claude > /dev/null 2>&1
thread=$("$silt" log | awk 'NR == 1 { print $1 }')
url=$("$silt" open --no-browser | tail -n 1)
printf '{"url":"%s","thread":"%s","work":"%s"}\n' "$url" "$thread" "$work"
