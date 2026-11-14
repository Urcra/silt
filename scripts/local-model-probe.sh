#!/bin/sh
# Probes this machine for local models (ADR 0066) and writes a Markdown
# report to paste back: the GPU and its memory, which llama.cpp builds run,
# how fast a model is, whether tools work on both APIs, and Claude Code on it.
#   scripts/local-model-probe.sh [model.gguf]
#
#   LLAMA_SERVER         a llama-server to probe first; else the one on PATH
#   PROBE_CTX            the context size to load the model with; 32768 by default
#   PROBE_PORT           the loopback port to serve it on; 18431 by default
#   PROBE_SKIP_DOWNLOAD  1 to skip downloading llama.cpp's latest release
#
# Only llama.cpp's release comes from the network (GitHub); the model is
# served on loopback, and Claude Code talks to it alone.
set -u

os=$(uname -s)
arch=$(uname -m)
out=$PWD/local-model-probe-$(echo "$os" | tr 'A-Z' 'a-z')-$arch.md
model=${1:-}
ctx=${PROBE_CTX:-32768}
port=${PROBE_PORT:-18431}
key=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
tmp=$(mktemp -d "${TMPDIR:-/tmp}/silt-probe.XXXXXX")
server_pid=

cleanup() {
    [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null
    rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

note() { echo "probe: $*" >&2; }
say() { printf '%s\n' "$*" >>"$out"; }
have() { command -v "$1" >/dev/null 2>&1; }

# Runs a command with a time limit, as macOS has no timeout(1).
limited() {
    secs=$1
    shift
    "$@" &
    pid=$!
    (sleep "$secs" && kill "$pid" 2>/dev/null) &
    watcher=$!
    wait "$pid"
    status=$?
    kill "$watcher" 2>/dev/null
    wait "$watcher" 2>/dev/null
    return $status
}

# Writes a command and its output into the report as a code block.
shown() {
    say '```'
    say "\$ $*"
    limited 120 "$@" >>"$out" 2>&1 || say "(exit $?)"
    say '```'
}

json() {
    # $1: a Python expression over `d`, the JSON on stdin.
    if have python3; then
        python3 -c "import json,sys
try:
    d=json.load(sys.stdin)
    print($1)
except Exception as e:
    print('unreadable:', e)"
    else
        echo "(python3 is missing; raw JSON follows)"
        cat
    fi
}

sha256() {
    if have sha256sum; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

: >"$out"
say "# Local model probe, $(date -u +%Y-%m-%d)"
say ""
say "## Machine"
shown uname -srm
case $os in
Linux)
    [ -r /etc/os-release ] && shown sh -c '. /etc/os-release; echo "$PRETTY_NAME"'
    shown sh -c "grep -m1 'model name' /proc/cpuinfo; grep MemTotal /proc/meminfo"
    have nvidia-smi && shown nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv
    have nvcc && shown nvcc --version
    have vulkaninfo && shown sh -c 'vulkaninfo --summary 2>/dev/null | head -60'
    if ls /sys/class/drm/card*/device/mem_info_gtt_total >/dev/null 2>&1; then
        shown sh -c 'for d in /sys/class/drm/card*/device; do [ -r $d/mem_info_vram_total ] && echo "$d vram=$(cat $d/mem_info_vram_total) gtt=$(cat $d/mem_info_gtt_total)"; done'
        shown sh -c 'for p in /sys/module/ttm/parameters/pages_limit /sys/module/ttm/parameters/page_pool_size /sys/module/amdgpu/parameters/gttsize; do [ -r $p ] && echo "$p=$(cat $p)"; done; tr " " "\n" </proc/cmdline | grep -E "amdgpu|ttm|iommu" || true'
    fi
    ;;
Darwin)
    shown sw_vers
    shown sysctl -n machdep.cpu.brand_string hw.memsize
    shown sh -c 'sysctl iogpu.wired_limit_mb 2>&1 || true'
    ;;
esac

# Every llama-server to try, one path per line.
candidates=$tmp/candidates
: >"$candidates"
[ -n "${LLAMA_SERVER:-}" ] && echo "$LLAMA_SERVER" >>"$candidates"
have llama-server && command -v llama-server >>"$candidates"

say ""
say "## llama.cpp's latest release"
if [ "${PROBE_SKIP_DOWNLOAD:-0}" = 1 ]; then
    say "Skipped (PROBE_SKIP_DOWNLOAD=1)."
elif ! have python3; then
    say "Skipped: python3 is needed to read the release's asset list."
else
    note "reading llama.cpp's latest release"
    curl -fsSL https://api.github.com/repos/ggml-org/llama.cpp/releases/latest -o "$tmp/release.json" || say "Couldn't read the release (curl exit $?)."
    if [ -s "$tmp/release.json" ]; then
        say "Tag: $(json 'd["tag_name"]' <"$tmp/release.json")"
        say ""
        say "Assets for this OS:"
        say '```'
        json '"\n".join(a["name"]+"  "+str(a.get("digest")) for a in d["assets"] if ("macos" if sys.platform=="darwin" else "ubuntu") in a["name"] or "linux" in a["name"])' <"$tmp/release.json" >>"$out"
        say '```'
        case $os-$arch in
        Darwin-arm64) wanted='bin-macos-arm64' ;;
        Linux-x86_64) wanted='bin-ubuntu-vulkan-x64 bin-ubuntu-x64 cuda' ;;
        Linux-aarch64) wanted='bin-ubuntu-arm64 bin-ubuntu-vulkan-arm64' ;;
        *) wanted= ;;
        esac
        for pattern in $wanted; do
            url=$(json "next((a['browser_download_url'] for a in d['assets'] if '$pattern' in a['name'] and ('ubuntu' in a['name'] or 'macos' in a['name'] or 'linux' in a['name']) and a['name'].endswith(('.zip','.tar.gz'))), '')" <"$tmp/release.json")
            [ -z "$url" ] && continue
            name=$(basename "$url")
            note "downloading $name"
            dir=$tmp/release/$pattern
            mkdir -p "$dir"
            if ! curl -fsSL --proto =https -o "$dir/$name" "$url"; then
                say "- \`$name\`: download failed"
                continue
            fi
            say "- \`$name\` sha256 $(sha256 "$dir/$name")"
            case $name in
            *.zip) if have unzip; then (cd "$dir" && unzip -q "$name"); else (cd "$dir" && python3 -m zipfile -e "$name" .); fi ;;
            *) (cd "$dir" && tar -xzf "$name") ;;
            esac
            found=$(find "$dir" -name llama-server -type f | head -1)
            if [ -n "$found" ]; then
                chmod +x "$found"
                echo "$found" >>"$candidates"
            else
                say "  (no llama-server in it)"
            fi
        done
    fi
fi

say ""
say "## Each llama-server"
while read -r srv <&3; do
    [ -z "$srv" ] && continue
    say ""
    say "### \`$srv\`"
    shown "$srv" --version
    shown "$srv" --list-devices
    if [ "$os" = Linux ] && have ldd; then
        shown sh -c "ldd '$srv' | grep 'not found' || echo 'every library found'"
    fi
done 3<"$candidates"

if [ -z "$model" ]; then
    say ""
    say "No model given, so nothing was loaded. Run again with a GGUF: \`scripts/local-model-probe.sh model.gguf\`."
    note "report written to $out"
    exit 0
fi

base=http://127.0.0.1:$port
ask() {
    # $1: path, $2: body file, $3: output file.
    curl -s -m 900 -H "Authorization: Bearer $key" -H "x-api-key: $key" \
        -H 'Content-Type: application/json' -H 'anthropic-version: 2023-06-01' \
        --data-binary "@$2" -o "$3" -w '%{http_code} %{time_total}s\n' "$base$1"
}

start_server() {
    "$1" -m "$model" --jinja -c "$ctx" -ngl 999 --host 127.0.0.1 --port "$port" --api-key "$key" >"$tmp/server.log" 2>&1 &
    server_pid=$!
    waited=0
    while [ $waited -lt 600 ]; do
        if ! kill -0 "$server_pid" 2>/dev/null; then
            server_pid=
            return 1
        fi
        if curl -s -o /dev/null -w '%{http_code}' "$base/health" | grep -q 200; then
            say "Loaded in about ${waited}s."
            return 0
        fi
        sleep 2
        waited=$((waited + 2))
    done
    return 1
}

stop_server() {
    [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null && wait "$server_pid" 2>/dev/null
    server_pid=
}

speed() {
    cat >"$tmp/speed.json" <<'EOF'
{"messages":[{"role":"user","content":"Write about 200 words on how rivers shape valleys."}],"max_tokens":300}
EOF
    say "Generation: $(ask /v1/chat/completions "$tmp/speed.json" "$tmp/speed.out")"
    say "$(json '"prompt %.1f tok/s, generation %.1f tok/s" % (d["timings"]["prompt_per_second"], d["timings"]["predicted_per_second"])' <"$tmp/speed.out")"
}

say ""
say "## The model"
say "\`$(basename "$model")\`, $(ls -l "$model" | awk '{print $5}') bytes, context $ctx."
first_done=
while read -r srv <&3; do
    [ -z "$srv" ] && continue
    say ""
    say "### On \`$srv\`"
    note "loading the model on $srv"
    if ! start_server "$srv"; then
        say "Didn't start. The end of its log:"
        say '```'
        tail -20 "$tmp/server.log" >>"$out"
        say '```'
        continue
    fi
    speed
    if [ -n "$first_done" ]; then
        stop_server
        continue
    fi
    first_done=$srv

    say ""
    say "Context and template (\`/props\`):"
    curl -s -H "Authorization: Bearer $key" "$base/props" -o "$tmp/props.json"
    say "$(json '"n_ctx=%s, chat template starts: %r" % (d.get("default_generation_settings",{}).get("n_ctx"), str(d.get("chat_template",""))[:80])' <"$tmp/props.json")"

    note "prompt processing and the prompt cache"
    if have python3; then
        python3 -c "
import json
words = ' '.join('the log line %d says the service restarted' % i for i in range(1800))
print(json.dumps({'messages':[{'role':'system','content':words},{'role':'user','content':'How many lines are there?'}],'max_tokens':20}))" >"$tmp/long.json"
        say ""
        say "A long prompt, twice (the second should reuse the cache):"
        for run in 1 2; do
            status=$(ask /v1/chat/completions "$tmp/long.json" "$tmp/long.out")
            say "- run $run: $status, $(json '"%d prompt tokens processed at %.1f tok/s" % (d["timings"]["prompt_n"], d["timings"]["prompt_per_second"])' <"$tmp/long.out")"
        done
    fi

    note "tool calls on the OpenAI API"
    cat >"$tmp/tools.json" <<'EOF'
{"messages":[{"role":"user","content":"What does notes.txt say? Use the tool."}],
 "tools":[{"type":"function","function":{"name":"read_file","description":"Read a file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}}],
 "max_tokens":300}
EOF
    say ""
    say "OpenAI tool call: $(ask /v1/chat/completions "$tmp/tools.json" "$tmp/tools.out")"
    say "$(json '"finish %s, calls %s" % (d["choices"][0]["finish_reason"], [(c["function"]["name"], c["function"]["arguments"]) for c in (d["choices"][0]["message"].get("tool_calls") or [])])' <"$tmp/tools.out")"

    note "tool calls on the Anthropic API"
    cat >"$tmp/messages.json" <<'EOF'
{"model":"local","max_tokens":300,
 "messages":[{"role":"user","content":"What does notes.txt say? Use the tool."}],
 "tools":[{"name":"read_file","description":"Read a file","input_schema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]}
EOF
    say "Anthropic tool use: $(ask /v1/messages "$tmp/messages.json" "$tmp/messages.out")"
    say "$(json '"stop %s, blocks %s" % (d.get("stop_reason"), [(b["type"], b.get("name"), b.get("input")) for b in d.get("content", [])])' <"$tmp/messages.out")"

    say ""
    say "## Claude Code on it"
    if ! have claude; then
        say "\`claude\` isn't on the PATH; skipped."
    else
        shown claude --version
        real=$(python3 -c 'import os,shutil;print(os.path.realpath(shutil.which("claude")))' 2>/dev/null || command -v claude)
        say "Variables it reads about models, context and compaction:"
        say '```'
        grep -aoE '(CLAUDE|ANTHROPIC)_[A-Z0-9_]*(COMPACT|CONTEXT|WINDOW|MODEL|TOKENS)[A-Z0-9_]*' "$real" 2>/dev/null | sort -u >>"$out"
        say '```'
        work=$tmp/work
        mkdir -p "$work"
        word=heron$(od -An -N2 -tu2 /dev/urandom | tr -d ' ')
        echo "The word is $word." >"$work/notes.txt"
        note "Claude Code reading a file through the local model (up to 15 minutes)"
        started=$(date +%s)
        (cd "$work" && CLAUDE_CONFIG_DIR=$tmp/claude ANTHROPIC_BASE_URL=$base ANTHROPIC_AUTH_TOKEN=$key \
            ANTHROPIC_MODEL=local ANTHROPIC_SMALL_FAST_MODEL=local ANTHROPIC_DEFAULT_HAIKU_MODEL=local \
            ANTHROPIC_DEFAULT_SONNET_MODEL=local ANTHROPIC_DEFAULT_OPUS_MODEL=local \
            CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
            limited 900 claude -p "What is the word in notes.txt? Answer with the word only." \
            --output-format json --allowedTools Read >"$tmp/claude.out" 2>"$tmp/claude.err")
        took=$(($(date +%s) - started))
        if grep -q "$word" "$tmp/claude.out"; then
            say "It read the file and answered, in ${took}s."
        else
            say "It didn't answer with the word (${took}s). Its output and errors:"
            say '```'
            head -c 3000 "$tmp/claude.out" >>"$out"
            head -c 2000 "$tmp/claude.err" >>"$out"
            say '```'
        fi
        say ""
        say "What it asked the server:"
        say '```'
        grep -oE '(GET|POST) /[a-z0-9/_]+' "$tmp/server.log" | sort | uniq -c >>"$out"
        say '```'
        say ""
        say "Still to check by hand: run \`claude\` interactively with the same variables and a fresh \`CLAUDE_CONFIG_DIR\`, and note any first-run prompts (theme, sign-in, trust)."
    fi
    stop_server
done 3<"$candidates"

[ -z "$first_done" ] && say "No llama-server could load the model."
# Paths name the person's home; the report shouldn't.
sed "s#$HOME#~#g" "$out" >"$tmp/report" && cat "$tmp/report" >"$out"
note "report written to $out; read it before pasting it anywhere"
