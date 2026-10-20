#!/bin/sh
# A server with two people, Alice and Bob, each with a daemon and a clone.
# Alice records a thread with the fake Claude and shares it with Bob; both
# open the web UI. Both can start the scripted ACP agent from it. Prints the details as one JSON line.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
silt="$root/target/debug/silt"
work=$(mktemp -d /tmp/silt-live-XXXXXX)
port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
url="http://127.0.0.1:$port"
password="correct horse battery"

git init -q -b main "$work/seed"
mkdir -p "$work/seed/src"
printf 'fn main() {}\n' > "$work/seed/src/main.rs"
git -C "$work/seed" add .
git -C "$work/seed" -c user.name=t -c user.email=t@example.com commit -q -m init
git clone -q --bare "$work/seed" "$work/origin.git"

printf '%s\n' "$password" | "$silt" server init --data "$work/server" --listen "127.0.0.1:$port" --public-url "$url" --password-stdin > /dev/null
for user in alice bob; do
    printf '%s\n' "$password" | "$silt" server user add "$user" --password-stdin --data "$work/server" > /dev/null
done
"$silt" server run --data "$work/server" > "$work/server.log" 2>&1 &
echo $! > "$work/server.pid"
for _ in $(seq 1 50); do
    python3 -c "import socket; socket.create_connection(('127.0.0.1', $port), 1)" 2>/dev/null && break
    sleep 0.1
done

machine() {
    user=$1
    export SILT_HOME="$work/$user/silt"
    export CLAUDE_CONFIG_DIR="$work/$user/claude"
    export SILT_CLAUDE_BIN="$root/web/e2e/fake-claude.sh"
    export SILT_HOOK="$silt hook"
    mkdir -p "$CLAUDE_CONFIG_DIR" "$SILT_HOME"
    printf '[{"id": "fake", "name": "Fake agent", "command": "%s"}]\n' "$root/target/debug/silt-fake-agent" > "$SILT_HOME/agents.json"
    git clone -q "$work/origin.git" "$work/$user/repo"
    cd "$work/$user/repo"
    printf '%s\n' "$password" | "$silt" auth login --server "$url" --user "$user" > /dev/null
}

machine alice
"$silt" claude > /dev/null 2>&1
thread=$("$silt" log | awk 'NR == 1 { print $1 }')
"$silt" thread share "$thread" bob > /dev/null
alice_url=$("$silt" open --no-browser)

machine bob
for _ in $(seq 1 100); do
    "$silt" log | grep -q "$thread" && break
    sleep 0.2
done
bob_url=$("$silt" open --no-browser)

printf '{"alice":"%s","bob":"%s","thread":"%s","work":"%s"}\n' "$alice_url" "$bob_url" "$thread" "$work"
