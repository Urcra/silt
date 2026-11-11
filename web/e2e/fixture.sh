#!/bin/sh
# Records a thread with the fake Claude, forks it, opens a merge request
# from the fork and starts the web UI. Prints the details as one JSON line.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
silt="${SILT:-$root/target/debug/silt}"
work=$(mktemp -d /tmp/silt-web-XXXXXX)
export SILT_HOME="$work/silt"
export SILT_VOICE_BIN="${SILT_VOICE_BIN:-/nonexistent}"
export CLAUDE_CONFIG_DIR="$work/claude"
# Agent sessions on the machine running the tests would show in the import
# offer, so each machine here has a home of its own.
export HOME="$work/home"
unset CODEX_HOME GEMINI_CLI_HOME COPILOT_HOME XDG_DATA_HOME XDG_CONFIG_HOME
mkdir -p "$HOME"
export SILT_CLAUDE_BIN="$root/web/e2e/fake-claude.sh"
export SILT_HOOK="$silt hook"
mkdir -p "$CLAUDE_CONFIG_DIR" "$work/repo/src"
cd "$work/repo"
git init -q -b main
printf 'fn main() {}\n' > src/main.rs
git add .
git -c user.name=t -c user.email=t@example.com commit -q -m init

"$silt" claude > /dev/null 2>&1
thread=$("$silt" log | awk 'NR == 1 { print $1 }')
FAKE_SAY="Continuing in a fork" FAKE_LINE="fn forked() {}" "$silt" fork "$thread" > /dev/null 2>&1
fork=$("$silt" log | awk '/fork of/ { print $1 }')
"$silt" mr create --thread "$fork" --title "Add forked" > /dev/null
url=$("$silt" open --no-browser)
printf '{"url":"%s","siltHome":"%s","work":"%s","thread":"%s","fork":"%s"}\n' "$url" "$SILT_HOME" "$work" "$thread" "$fork"
