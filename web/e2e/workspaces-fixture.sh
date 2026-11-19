#!/bin/sh
# Two servers, work and oss, and Ada's machine signed in to both, work
# first, with clones of two projects. Prints the details as one JSON line.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
silt="$root/target/debug/silt"
work=$(mktemp -d /tmp/silt-workspaces-XXXXXX)
# A failure partway leaves nothing running or on disk.
trap 'status=$?; [ "$status" -eq 0 ] || "$root/web/e2e/abandon.sh" "$work" "$silt"; exit "$status"' EXIT
free_port() {
    python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}
password="correct horse battery"

for project in app tools; do
    git init -q -b main "$work/$project-seed"
    mkdir -p "$work/$project-seed/src"
    printf 'fn %s() {}\n' "$project" > "$work/$project-seed/src/main.rs"
    git -C "$work/$project-seed" add .
    git -C "$work/$project-seed" -c user.name=t -c user.email=t@example.com commit -q -m init
    git clone -q --bare "$work/$project-seed" "$work/$project.git"
done

serve() {
    name=$1
    port=$(free_port)
    url="http://127.0.0.1:$port"
    printf '%s\n' "$password" | "$silt" server init --data "$work/$name" --listen "127.0.0.1:$port" --public-url "$url" --password-stdin > /dev/null
    printf '%s\n' "$password" | "$silt" server user add ada --password-stdin --data "$work/$name" > /dev/null
    "$silt" server run --data "$work/$name" > "$work/$name.log" 2>&1 &
    echo $! > "$work/$name.pid"
    for _ in $(seq 1 50); do
        python3 -c "import socket; socket.create_connection(('127.0.0.1', $port), 1)" 2>/dev/null && break
        sleep 0.1
    done
    echo "$url"
}
work_url=$(serve work)
oss_url=$(serve oss)

export SILT_HOME="$work/ada/silt"
export SILT_VOICE_BIN="${SILT_VOICE_BIN:-/nonexistent}"
export CLAUDE_CONFIG_DIR="$work/ada/claude"
# Agent sessions on the machine running the tests would show in the import
# offer, and agent CLIs installed in its home would count as installed here,
# so each machine here has a home of its own.
# The Rust toolchain stays, for plugins built from git, and comes first: a
# system cargo that isn't rustup's, as Arch's, doesn't take `cargo +<toolchain>`.
toolchain=$(dirname "$(command -v rustup || command -v cargo)")
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
PATH=$toolchain:$(printf '%s' "$PATH" | tr ':' '\n' | grep -v "^$HOME/" | paste -sd:)
export HOME="$work/ada/home"
unset CODEX_HOME GEMINI_CLI_HOME COPILOT_HOME XDG_DATA_HOME XDG_CONFIG_HOME
mkdir -p "$HOME"
mkdir -p "$CLAUDE_CONFIG_DIR" "$SILT_HOME"
fake="$root/target/debug/silt-fake-agent"
printf '[{"id": "fake", "name": "Fake agent", "command": "%s"}]\n' "$fake" > "$SILT_HOME/agents.json"
for project in app tools; do
    git clone -q "$work/$project.git" "$work/ada/$project"
done
cd "$work/ada/app"
for url in "$work_url" "$oss_url"; do
    printf '%s\n' "$password" | "$silt" auth login --server "$url" --user ada --name ada > /dev/null
done
(cd "$work/ada/tools" && "$silt" open --no-browser > /dev/null)
ada_url=$("$silt" open --no-browser)

printf '{"ada":"%s","work":"%s","oss":"%s","dir":"%s"}\n' "$ada_url" "$work_url" "$oss_url" "$work"
