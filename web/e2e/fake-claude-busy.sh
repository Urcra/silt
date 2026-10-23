#!/bin/sh
# A scripted Claude Code session with a lot in it, for the busy thread's
# screen reference: a pasted screenshot, thinking, searches and reads,
# commands with their output, a multiple-choice question answered, a plan
# kept up to date, edits, and a screenshot back from the agent.
set -eu
session=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --resume) session="$2"; shift 2 ;;
        --settings) shift 2 ;;
        *) shift ;;
    esac
done
slug=$(pwd | sed 's/[^A-Za-z0-9]/-/g')
dir="$CLAUDE_CONFIG_DIR/projects/$slug"
transcript="$dir/$session.jsonl"
mkdir -p "$dir"
# The screenshot pasted with the prompt, and the one the agent took.
fixtures=$(cd "$(dirname "$0")/fixtures" && pwd)
pasted=$(base64 -w0 "$fixtures/upload-100.png")
taken=$(base64 -w0 "$fixtures/upload-40.png")
say() {
    printf '%s\n' "$1" >> "$transcript"
}
hook() {
    printf '{"session_id":"%s","hook_event_name":"%s","tool_name":"%s","tool_use_id":"%s"}' "$session" "$1" "${2:-}" "${3:-}" | $SILT_HOOK "$1"
}
# Each of the agent's messages names itself and what it cost, as Claude Code records them.
tool() {
    say "{\"type\":\"assistant\",\"message\":{\"id\":\"msg_$1\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":1200,\"cache_read_input_tokens\":14000,\"output_tokens\":340},\"content\":[{\"type\":\"tool_use\",\"id\":\"$1\",\"name\":\"$2\",\"input\":$3}]}}"
    hook PreToolUse "$2" "$1"
}
result() {
    hook PostToolUse "$2" "$1"
    say "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"$1\",\"content\":$3,\"is_error\":${4:-false}}]}}"
}

say '{"type":"ai-title","aiTitle":"Upload progress jumps to 100%","sessionId":"x"}'
say "{\"type\":\"user\",\"cwd\":\"$(pwd)\",\"sessionId\":\"$session\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"The upload progress bar jumps to 100% before the upload finishes. Screenshot attached.\"},{\"type\":\"image\",\"source\":{\"type\":\"base64\",\"media_type\":\"image/png\",\"data\":\"$pasted\"}}]}}"
hook UserPromptSubmit
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"The bar reaches 100% early, so progress probably counts bytes read from disk, not bytes the server has taken. Find where progress is reported."}]}}'

tool toolu_1 Grep '{"pattern":"progress","path":"src"}'
result toolu_1 Grep '"src/upload.rs:4:    progress(read);\nsrc/upload.rs:9:fn progress(bytes: usize) {}"'
tool toolu_2 Read '{"file_path":"src/upload.rs"}'
result toolu_2 Read '"1\tpub fn upload(data: &[u8]) {\n2\t    let read = data.len();\n3\t    send(data);\n4\t    progress(read);\n5\t}"'
tool toolu_3 Bash '{"command":"cargo test upload","description":"Run the upload tests"}'
result toolu_3 Bash '"Exit code 101\nrunning 2 tests\ntest upload::reports_bytes_sent ... FAILED\ntest upload::sends_in_chunks ... ok\n\nfailures:\n    upload::reports_bytes_sent\n\ntest result: FAILED. 1 passed; 1 failed"' true

tool toolu_4 AskUserQuestion '{"questions":[{"question":"What should the progress bar count?","header":"Progress","multiSelect":false,"options":[{"label":"Bytes the server acknowledged","description":"Accurate, but moves in steps of one chunk"},{"label":"Bytes sent on the socket","description":"Smooth, but can run ahead of the server"},{"label":"Chunks completed","description":"Simplest, coarse for small files"}]}]}'
result toolu_4 AskUserQuestion '"User has answered your questions: \"What should the progress bar count?\"=\"Bytes the server acknowledged\". You can now continue with the user'"'"'s answers in mind."'
tool toolu_9 TodoWrite '{"todos":[{"content":"Send the upload in chunks","status":"in_progress","activeForm":"Sending the upload in chunks"},{"content":"Report each acknowledged chunk","status":"pending","activeForm":"Reporting each acknowledged chunk"},{"content":"Check the bar mid-upload","status":"pending","activeForm":"Checking the bar mid-upload"}]}'
result toolu_9 TodoWrite '"Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress."'

tool toolu_5 Edit '{"file_path":"src/upload.rs","old_string":"    send(data);\n    progress(read);","new_string":"    for chunk in data.chunks(CHUNK) {\n        let acked = send(chunk);\n        progress(acked);\n    }"}'
cat > src/upload.rs <<'EOF'
pub fn upload(data: &[u8]) {
    for chunk in data.chunks(CHUNK) {
        let acked = send(chunk);
        progress(acked);
    }
}
EOF
result toolu_5 Edit '"The file src/upload.rs has been updated."'
tool toolu_6 Write '{"file_path":"src/chunk.rs","content":"/// How much goes in one request, and so how often progress moves.\npub const CHUNK: usize = 256 * 1024;\n"}'
printf '/// How much goes in one request, and so how often progress moves.\npub const CHUNK: usize = 256 * 1024;\n' > src/chunk.rs
result toolu_6 Write '"File created successfully at: src/chunk.rs"'
tool toolu_7 Bash '{"command":"cargo test upload","description":"Run the upload tests again"}'
result toolu_7 Bash '"running 2 tests\ntest upload::reports_bytes_sent ... ok\ntest upload::sends_in_chunks ... ok\n\ntest result: ok. 2 passed; 0 failed"'
tool toolu_8 Bash '{"command":"npx playwright screenshot http://localhost:5173/upload upload.png","description":"Screenshot the upload page mid-upload"}'
result toolu_8 Bash "[{\"type\":\"text\",\"text\":\"Capturing screenshot into upload.png\"},{\"type\":\"image\",\"source\":{\"type\":\"base64\",\"media_type\":\"image/png\",\"data\":\"$taken\"}}]"
tool toolu_10 TodoWrite '{"todos":[{"content":"Send the upload in chunks","status":"completed","activeForm":"Sending the upload in chunks"},{"content":"Report each acknowledged chunk","status":"completed","activeForm":"Reporting each acknowledged chunk"},{"content":"Check the bar mid-upload","status":"completed","activeForm":"Checking the bar mid-upload"}]}'
result toolu_10 TodoWrite '"Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress."'

say '{"type":"assistant","message":{"id":"msg_summary","role":"assistant","usage":{"input_tokens":900,"cache_read_input_tokens":16000,"output_tokens":420},"content":[{"type":"text","text":"Fixed: progress now counts the bytes the server acknowledged, one 256 KiB chunk at a time.\n\n- `src/upload.rs` sends in chunks and reports each acknowledged one\n- `src/chunk.rs` holds the chunk size\n\nBoth upload tests pass, and the screenshot shows the bar at 40% mid-upload."}]}}'
hook Stop
