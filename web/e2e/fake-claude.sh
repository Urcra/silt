#!/bin/sh
# A scripted stand-in for Claude Code, shared with the Rust end-to-end
# tests (crates/silt-cli/tests/common/mod.rs), plus a compaction.
set -eu
session=""
resume=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --resume) session="$2"; resume=1; shift 2 ;;
        --settings) shift 2 ;;
        *) shift ;;
    esac
done
slug=$(pwd | sed 's/[^A-Za-z0-9]/-/g')
dir="$CLAUDE_CONFIG_DIR/projects/$slug"
transcript="$dir/$session.jsonl"
hook() {
    printf '{"session_id":"%s","hook_event_name":"%s","tool_name":"Edit","tool_use_id":"%s"}' "$session" "$1" "$2" | $SILT_HOOK "$1"
}
say() {
    printf '%s\n' "$1" >> "$transcript"
}
if [ -n "$resume" ]; then
    grep -q "\"cwd\":\"$(pwd)\"" "$transcript" || { echo "transcript not restored for $(pwd)" >&2; exit 3; }
    grep -q "Please add a farewell line" "$transcript" || { echo "conversation missing" >&2; exit 3; }
    saved=$(sed -n 's/.*saved to \([^"]*\)".*/\1/p' "$transcript" | head -n 1)
    [ "$saved" = "$dir/$session/tool-results/t1.txt" ] || { echo "tool result path not rewritten: $saved" >&2; exit 4; }
    grep -q "big output" "$saved" || { echo "tool result missing" >&2; exit 4; }
    grep -q "\"sessionId\":\"$session\"" "$transcript" || { echo "session id not rewritten" >&2; exit 5; }
    for needle in ${FAKE_EXPECT:-}; do
        needle=$(printf '%s' "$needle" | tr '-' ' ')
        grep -q "$needle" "$transcript" || { echo "transcript lacks $needle" >&2; cat "$transcript" >&2; exit 6; }
    done
    printf '%s' "$session" > "$CLAUDE_CONFIG_DIR/last-session"
    say "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"${FAKE_SAY:-Continuing at home: add a home line}\"}}"
    hook UserPromptSubmit toolu_2 > "$CLAUDE_CONFIG_DIR/last-hook-output"
    say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_2","name":"Edit","input":{"file_path":"src/main.rs"}}]}}'
    hook PreToolUse toolu_2
    printf '%s\n' "${FAKE_LINE:-fn home() {\}}" >> src/main.rs
    hook PostToolUse toolu_2
    say '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"edited","is_error":false}]}}'
    hook Stop toolu_2
    exit 0
fi
mkdir -p "$dir"
say '{"type":"ai-title","aiTitle":"Add a farewell","sessionId":"x"}'
say "{\"type\":\"user\",\"cwd\":\"$(pwd)\",\"sessionId\":\"$session\",\"message\":{\"role\":\"user\",\"content\":\"Please add a farewell line to main\"}}"
hook UserPromptSubmit toolu_1
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"src/main.rs"}}]}}'
hook PreToolUse toolu_1
printf 'fn farewell() {}\n' >> src/main.rs
hook PostToolUse toolu_1
mkdir -p "$dir/$session/tool-results"
printf 'big output\n' > "$dir/$session/tool-results/t1.txt"
say "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"saved to $dir/$session/tool-results/t1.txt\"}]}}"
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done: farewell added. I used ghp_abcdefghijklmnopqrstuvwxyz0123 to fetch it."}]}}'
say '{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}'
say '{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"Summary: farewell was added to main."}}'
hook Stop toolu_1
