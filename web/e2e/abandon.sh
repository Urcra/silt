#!/bin/sh
# Called by a fixture script that failed partway, with its folder and its
# silt: stops the daemons and servers it started there and removes the
# folder, as the tests that would tear it down never run.
# Usage: abandon.sh <work> <silt>
work=$1
silt=$2
[ -d "$work" ] || exit 0
find "$work" -name silt.sqlite 2>/dev/null | while read -r store; do
    SILT_HOME=$(dirname "$store") "$silt" daemon stop >/dev/null 2>&1 || true
done
find "$work" -maxdepth 1 -name '*.pid' 2>/dev/null | while read -r pid; do
    kill "$(cat "$pid")" 2>/dev/null || true
done
# What was just stopped can still be writing there for a moment.
for _ in 1 2 3 4 5; do
    rm -rf "$work" 2>/dev/null && exit 0
    sleep 1
done
echo "abandon.sh: couldn't remove $work" >&2
