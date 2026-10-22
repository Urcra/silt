#!/bin/sh
# The screen references' world (ADR 0036): live-fixture.sh's server, Alice,
# Bob and Alice's laptop, plus a fork of Alice's thread with a merge request
# from it. Prints live-fixture.sh's JSON line with the fork added.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
silt="$root/target/debug/silt"
line=$("$here/live-fixture.sh" | tail -n 1)
work=$(printf '%s' "$line" | sed 's/.*"work":"\([^"]*\)".*/\1/')
thread=$(printf '%s' "$line" | sed 's/.*"thread":"\([^"]*\)".*/\1/')

export SILT_HOME="$work/alice/silt"
export CLAUDE_CONFIG_DIR="$work/alice/claude"
export SILT_CLAUDE_BIN="$here/fake-claude.sh"
export SILT_HOOK="$silt hook"
cd "$work/alice/repo"
FAKE_SAY="Retry blob uploads on a 5xx" FAKE_LINE="fn retry() {}" "$silt" fork "$thread" > /dev/null 2>&1
fork=$("$silt" log | awk '/fork of/ { print $1; exit }')
"$silt" mr create --thread "$fork" --title "Retry blob uploads on 5xx" > /dev/null

printf '%s,"fork":"%s"}\n' "${line%\}}" "$fork"
