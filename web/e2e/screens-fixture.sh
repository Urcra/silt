#!/bin/sh
# The screen references' world (ADR 0039): live-fixture.sh's server, Alice,
# Bob and Alice's laptop, plus a fork of Alice's thread with a merge request
# from it. Prints live-fixture.sh's JSON line with the fork added.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
silt="$root/target/debug/silt"
# What an agent offers comes from its runs here, not from asking it in the background, whose answer could land
# before or after a screen is taken.
export SILT_ASK_AGENTS=0
line=$("$here/live-fixture.sh" | tail -n 1)
work=$(printf '%s' "$line" | sed 's/.*"work":"\([^"]*\)".*/\1/')
thread=$(printf '%s' "$line" | sed 's/.*"thread":"\([^"]*\)".*/\1/')

export SILT_HOME="$work/alice/silt"
export SILT_VOICE_BIN="${SILT_VOICE_BIN:-/nonexistent}"
export CLAUDE_CONFIG_DIR="$work/alice/claude"
# Agent sessions on the machine running the tests would show in the import
# offer, so each machine here has a home of its own.
export HOME="$work/alice/home"
unset CODEX_HOME GEMINI_CLI_HOME COPILOT_HOME XDG_DATA_HOME XDG_CONFIG_HOME
mkdir -p "$HOME"
export SILT_CLAUDE_BIN="$here/fake-claude.sh"
export SILT_HOOK="$silt hook"
cd "$work/alice/repo"
FAKE_SAY="Retry blob uploads on a 5xx" FAKE_LINE="fn retry() {}" "$silt" fork "$thread" > /dev/null 2>&1
fork=$("$silt" log | awk '/fork of/ { print $1; exit }')
"$silt" mr create --thread "$fork" --title "Retry blob uploads on 5xx" > /dev/null

printf '%s,"fork":"%s"}\n' "${line%\}}" "$fork"
