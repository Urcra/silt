//! Scripted stand-ins for agent CLIs that write each agent's own session
//! format (docs/research/agents.md). A first run records a prompt and an
//! edit of `src/main.rs`; a resumed run checks the restored session names
//! the current directory, mentions every `FAKE_EXPECT` word (dashes for
//! spaces), and adds `FAKE_SAY` and the line `FAKE_LINE`.

/// Codex: picks its own id and writes a rollout under `$CODEX_HOME`.
pub const FAKE_CODEX: &str = r#"#!/bin/sh
set -eu
resume=""
id=""
while [ $# -gt 0 ]; do
    case "$1" in
        resume) resume=1; shift ;;
        -c) printf '%s\n' "$2" >> "$CODEX_HOME/last-config"; shift 2 ;;
        *) id="$1"; shift ;;
    esac
done
now=$(date -u +%Y-%m-%dT%H:%M:%S.000Z)
line() { printf '{"timestamp":"%s","type":"%s","payload":%s}\n' "$now" "$1" "$2" >> "$rollout"; }
turn() {
    line event_msg '{"type":"task_started","turn_id":"t"}'
    line response_item "{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"$1\"}]}"
    line event_msg "{\"type\":\"user_message\",\"message\":\"$1\"}"
    line response_item "{\"type\":\"function_call\",\"name\":\"apply_patch\",\"arguments\":\"{\\\"path\\\":\\\"$(pwd)/src/main.rs\\\"}\",\"call_id\":\"$2\"}"
    printf '%s\n' "$3" >> src/main.rs
    sleep 0.5
    line response_item "{\"type\":\"function_call_output\",\"call_id\":\"$2\",\"output\":\"patched\"}"
    line response_item '{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Codex says done."}]}'
    line event_msg '{"type":"task_complete","turn_id":"t","last_agent_message":"Codex says done."}'
}
if [ -n "$resume" ]; then
    rollout=$(find "$CODEX_HOME/sessions" -name "rollout-*-$id.jsonl" | head -n 1)
    [ -n "$rollout" ] || { echo "no rollout for $id" >&2; exit 3; }
    grep -q "\"cwd\":\"$(pwd)\"" "$rollout" || { echo "rollout not restored for $(pwd)" >&2; exit 3; }
    for needle in ${FAKE_EXPECT:-}; do
        needle=$(printf '%s' "$needle" | tr '-' ' ')
        grep -q "$needle" "$rollout" || { echo "rollout lacks $needle" >&2; cat "$rollout" >&2; exit 6; }
    done
    printf '%s' "$id" > "$CODEX_HOME/last-session"
    turn "${FAKE_SAY:-Continuing in codex}" call_2 "${FAKE_LINE:-fn codex() {\}}"
    exit 0
fi
id=$(cat /proc/sys/kernel/random/uuid)
dir="$CODEX_HOME/sessions/$(date -u +%Y/%m/%d)"
mkdir -p "$dir"
rollout="$dir/rollout-$(date -u +%Y-%m-%dT%H-%M-%S)-$id.jsonl"
line session_meta "{\"id\":\"$id\",\"timestamp\":\"$now\",\"cwd\":\"$(pwd)\",\"originator\":\"codex_cli_rs\",\"cli_version\":\"0.163.0\",\"source\":\"cli\"}"
line turn_context "{\"cwd\":\"$(pwd)\",\"approval_policy\":\"on-request\",\"sandbox_policy\":{\"type\":\"workspace-write\"},\"model\":\"gpt-5.5\"}"
turn "Please add a farewell line to main" call_1 "fn farewell() {}"
"#;

/// Gemini CLI: takes `--session-id`, keeps chats under the project's slug.
pub const FAKE_GEMINI: &str = r#"#!/bin/sh
set -eu
session=""
resume=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --resume) session="$2"; resume=1; shift 2 ;;
        --prompt-interactive) printf '%s' "$2" > "$FAKE_LOG_DIR/gemini-prompt"; shift 2 ;;
        *) shift ;;
    esac
done
gemini="$GEMINI_CLI_HOME/.gemini"
grep -q '"silt"' "$gemini/settings.json" || { echo "no silt MCP server in settings" >&2; exit 7; }
slug=$(basename "$(pwd)" | tr 'A-Z' 'a-z' | sed 's/[^a-z0-9]\{1,\}/-/g')
chats="$gemini/tmp/$slug/chats"
now=$(date -u +%Y-%m-%dT%H:%M:%S.000Z)
say() { printf '%s\n' "$1" >> "$file"; }
turn() {
    say "{\"id\":\"u-$2\",\"timestamp\":\"$now\",\"type\":\"user\",\"content\":[{\"text\":\"$1\"}]}"
    say "{\"id\":\"g-$2\",\"timestamp\":\"$now\",\"type\":\"gemini\",\"content\":\"\",\"thoughts\":[]}"
    # Like Gemini, the tool call is only written once the tool finished.
    printf '%s\n' "$3" >> src/main.rs
    say "{\"id\":\"g-$2\",\"timestamp\":\"$now\",\"type\":\"gemini\",\"content\":\"\",\"thoughts\":[],\"toolCalls\":[{\"id\":\"edit-$2\",\"name\":\"replace\",\"args\":{\"file_path\":\"$(pwd)/src/main.rs\"},\"result\":[{\"functionResponse\":{\"id\":\"edit-$2\",\"name\":\"replace\",\"response\":{\"output\":\"replaced\"}}}],\"status\":\"success\"}]}"
    say "{\"id\":\"a-$2\",\"timestamp\":\"$now\",\"type\":\"gemini\",\"content\":\"Gemini says done.\",\"thoughts\":[]}"
    # Written again with its tokens once the reply finished streaming.
    say "{\"id\":\"a-$2\",\"timestamp\":\"$now\",\"type\":\"gemini\",\"content\":\"Gemini says done.\",\"thoughts\":[],\"tokens\":{\"total\":1}}"
    say '{"$set":{"lastUpdated":"now"}}'
}
if [ -n "$resume" ]; then
    file=$(grep -l "\"sessionId\":\"$session\"" "$chats"/session-*.jsonl | head -n 1)
    [ -n "$file" ] || { echo "no session $session in $chats" >&2; exit 3; }
    grep -q "$(pwd)/src/main.rs" "$file" || { echo "session not restored for $(pwd)" >&2; exit 3; }
    for needle in ${FAKE_EXPECT:-}; do
        needle=$(printf '%s' "$needle" | tr '-' ' ')
        grep -q "$needle" "$file" || { echo "session lacks $needle" >&2; cat "$file" >&2; exit 6; }
    done
    turn "${FAKE_SAY:-Continuing in gemini}" 2 "${FAKE_LINE:-fn gemini() {\}}"
    exit 0
fi
mkdir -p "$chats"
printf '%s' "$(pwd)" > "$gemini/tmp/$slug/.project_root"
printf '{"projects":{"%s":"%s"}}' "$(pwd)" "$slug" > "$gemini/projects.json"
file="$chats/session-$(date -u +%Y-%m-%dT%H-%M)-$(printf '%s' "$session" | cut -c1-8).jsonl"
say "{\"sessionId\":\"$session\",\"projectHash\":\"x\",\"startTime\":\"$now\",\"lastUpdated\":\"$now\",\"kind\":\"main\"}"
say "{\"id\":\"ctx\",\"timestamp\":\"$now\",\"type\":\"user\",\"content\":[{\"text\":\"<session_context>Gemini CLI</session_context>\"}]}"
turn "Please add a farewell line to main" 1 "fn farewell() {}"
"#;

/// GitHub Copilot CLI: a session directory with `events.jsonl`.
pub const FAKE_COPILOT: &str = r#"#!/bin/sh
set -eu
session=""
resume=""
plugin=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --resume) session="$2"; resume=1; shift 2 ;;
        --plugin-dir) plugin="$2"; shift 2 ;;
        --additional-mcp-config) printf '%s' "$2" > "$COPILOT_HOME/last-mcp"; shift 2 ;;
        --interactive) printf '%s' "$2" > "$COPILOT_HOME/last-prompt"; shift 2 ;;
        *) shift ;;
    esac
done
[ -f "$plugin/hooks/hooks.json" ] || { echo "no Silt plugin" >&2; exit 7; }
dir="$COPILOT_HOME/session-state/$session"
events="$dir/events.jsonl"
now=$(date -u +%Y-%m-%dT%H:%M:%S.000Z)
n=0
event() { n=$((n + 1)); printf '{"type":"%s","data":%s,"id":"%s-%s","timestamp":"%s","parentId":null}\n' "$1" "$2" "$session" "$n" "$now" >> "$events"; }
turn() {
    event user.message "{\"content\":\"$1\",\"transformedContent\":\"$1\"}"
    event assistant.message "{\"content\":\"\",\"toolRequests\":[{\"toolCallId\":\"$2\",\"name\":\"edit\",\"arguments\":{\"path\":\"$(pwd)/src/main.rs\"}}]}"
    # Like Copilot, events reach the file a little after they happen.
    printf '%s\n' "$3" >> src/main.rs
    sleep 0.1
    event tool.execution_start "{\"toolCallId\":\"$2\",\"toolName\":\"edit\"}"
    event tool.execution_complete "{\"toolCallId\":\"$2\",\"success\":true,\"result\":{\"content\":\"edited\"}}"
    event assistant.message '{"content":"Copilot says done.","toolRequests":[]}'
}
if [ -n "$resume" ]; then
    [ -f "$events" ] || { echo "no session $session" >&2; exit 3; }
    grep -q "\"cwd\":\"$(pwd)\"" "$events" || { echo "events not restored for $(pwd)" >&2; exit 3; }
    grep -q "$(pwd)" "$dir/workspace.yaml" || { echo "workspace not restored" >&2; exit 3; }
    for needle in ${FAKE_EXPECT:-}; do
        needle=$(printf '%s' "$needle" | tr '-' ' ')
        grep -q "$needle" "$events" || { echo "session lacks $needle" >&2; cat "$events" >&2; exit 6; }
    done
    n=1000
    turn "${FAKE_SAY:-Continuing in copilot}" call_2 "${FAKE_LINE:-fn copilot() {\}}"
    exit 0
fi
mkdir -p "$dir"
printf 'id: %s\ncwd: %s\n' "$session" "$(pwd)" > "$dir/workspace.yaml"
event session.start "{\"sessionId\":\"$session\",\"context\":{\"cwd\":\"$(pwd)\",\"gitRoot\":\"$(pwd)\"}}"
turn "Please add a farewell line to main" call_1 "fn farewell() {}"
"#;

/// OpenCode: sessions live in a store reached only through `opencode
/// session list`, `export` and `import`; here the store is
/// `$FAKE_OPENCODE_DIR`, one single-line export per session.
pub const FAKE_OPENCODE: &str = r#"#!/bin/sh
set -eu
store="$FAKE_OPENCODE_DIR"
mkdir -p "$store"
case "${1:-}" in
    session)
        printf '['
        first=1
        for meta in "$store"/*.meta; do
            [ -f "$meta" ] || continue
            [ -n "$first" ] || printf ','
            first=""
            read -r id created directory < "$meta"
            printf '{"id":"%s","created":%s,"updated":%s,"directory":"%s"}' "$id" "$created" "$created" "$directory"
        done
        printf ']\n'
        exit 0 ;;
    export)
        printf 'Exporting session: %s\n' "$2" >&2
        cat "$store/$2.json"
        exit 0 ;;
    import)
        id=$(grep -o '"id":"ses_[A-Za-z0-9]*"' "$2" | head -n 1 | cut -d'"' -f4)
        cp "$2" "$store/$id.json"
        printf '%s %s %s\n' "$id" "$(date +%s%3N)" "$(grep -o '"directory":"[^"]*"' "$2" | head -n 1 | cut -d'"' -f4)" > "$store/$id.meta"
        printf 'Imported session: %s\n' "$id"
        exit 0 ;;
esac
session=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session) session="$2"; shift 2 ;;
        --prompt) printf '%s' "$2" > "$store/last-prompt"; shift 2 ;;
        *) shift ;;
    esac
done
printf '%s' "${OPENCODE_CONFIG_CONTENT:-}" > "$store/last-config"
message() {
    printf '{"info":{"id":"msg_%s%s","role":"user","sessionID":"%s"},"parts":[{"id":"prt_%s%sa","messageID":"msg_%s%s","type":"text","text":"%s"}]},' "$2" "u" "$session" "$2" "u" "$2" "u" "$1"
    printf '{"info":{"id":"msg_%s%s","role":"assistant","sessionID":"%s","path":{"cwd":"%s"}},"parts":[' "$2" "a" "$session" "$(pwd)"
    printf '{"id":"prt_%sb","messageID":"msg_%sa","type":"tool","callID":"call_%s","tool":"edit","state":{"status":"completed","input":{"filePath":"%s/src/main.rs"},"output":"edited"}},' "$2" "$2" "$2" "$(pwd)"
    printf '{"id":"prt_%sc","messageID":"msg_%sa","type":"text","text":"OpenCode says done.","time":{"start":1,"end":2}},' "$2" "$2"
    printf '{"id":"prt_%sd","messageID":"msg_%sa","type":"step-finish","reason":"stop"}]}' "$2" "$2"
}
if [ -n "$session" ]; then
    file="$store/$session.json"
    [ -f "$file" ] || { echo "no session $session" >&2; exit 3; }
    grep -q "$(pwd)" "$file" || { echo "session not restored for $(pwd)" >&2; exit 3; }
    for needle in ${FAKE_EXPECT:-}; do
        needle=$(printf '%s' "$needle" | tr '-' ' ')
        grep -q "$needle" "$file" || { echo "session lacks $needle" >&2; cat "$file" >&2; exit 6; }
    done
    printf '%s\n' "${FAKE_LINE:-fn opencode() {\}}" >> src/main.rs
    turn=$(message "${FAKE_SAY:-Continuing in opencode}" 0000000002ZZZZZZZZZZZZZZZZ | sed 's/,$//')
    sed -i "s|]}\$|,$turn]}|" "$file"
    sleep 4
    exit 0
fi
session="ses_$(date +%s%3N | cut -c1-12)$(cat /proc/sys/kernel/random/uuid | tr -d '-' | cut -c1-14)"
created=$(date +%s%3N)
printf '%s %s %s\n' "$session" "$created" "$(pwd)" > "$store/$session.meta"
printf '%s\n' "fn farewell() {}" >> src/main.rs
first=$(message "Please add a farewell line to main" 0000000001AAAAAAAAAAAAAAAA | sed 's/,$//')
printf '{"info":{"id":"%s","directory":"%s","title":"Farewell","time":{"created":%s}},"messages":[%s]}\n' "$session" "$(pwd)" "$created" "$first" > "$store/$session.json"
sleep 4
"#;
