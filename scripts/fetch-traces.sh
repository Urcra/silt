#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
dir=target/editing-traces
mkdir -p "$dir"
for trace in automerge-paper seph-blog1; do
    [ -f "$dir/$trace.json.gz" ] || curl -fsSL -o "$dir/$trace.json.gz" \
        "https://raw.githubusercontent.com/josephg/editing-traces/master/sequential_traces/$trace.json.gz"
done
ls -la "$dir"
