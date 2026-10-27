#!/bin/sh
# A server with two people, Alice and Bob, each with a daemon and a clone.
# Alice records a thread with the fake Claude and shares it with Bob; both
# open the web UI. Both can start the scripted ACP agent from it. Alice has
# a clone of a second project, tools, too. Prints the details as one JSON line.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
silt="$root/target/debug/silt"
work=$(mktemp -d /tmp/silt-live-XXXXXX)
free_port() {
    python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}
port=$(free_port)
url="http://127.0.0.1:$port"
password="correct horse battery"

git init -q -b main "$work/seed"
mkdir -p "$work/seed/src"
printf 'fn main() {}\n' > "$work/seed/src/main.rs"
git -C "$work/seed" add .
git -C "$work/seed" -c user.name=t -c user.email=t@example.com commit -q -m init
git clone -q --bare "$work/seed" "$work/origin.git"
# A second project, which only Alice has a clone of.
git init -q -b main "$work/tools-seed"
mkdir -p "$work/tools-seed/src"
printf 'fn tool() {}\n' > "$work/tools-seed/src/main.rs"
git -C "$work/tools-seed" add .
git -C "$work/tools-seed" -c user.name=t -c user.email=t@example.com commit -q -m init
git clone -q --bare "$work/tools-seed" "$work/tools.git"
# A third that nobody has cloned yet.
git clone -q --bare "$work/tools-seed" "$work/docs.git"

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

# A machine of `user`, named and kept under `home` (the user's name unless given).
machine() {
    user=$1
    home=${2:-$1}
    export SILT_HOME="$work/$home/silt"
    export CLAUDE_CONFIG_DIR="$work/$home/claude"
    # Agent sessions on the machine running the tests would show in the import
    # offer, so each machine here has a home of its own.
    export HOME="$work/$home/home"
    unset CODEX_HOME GEMINI_CLI_HOME COPILOT_HOME XDG_DATA_HOME XDG_CONFIG_HOME
    mkdir -p "$HOME"
    export SILT_CLAUDE_BIN="$root/web/e2e/fake-claude.sh"
    export SILT_HOOK="$silt hook"
    mkdir -p "$CLAUDE_CONFIG_DIR" "$SILT_HOME"
    fake="$root/target/debug/silt-fake-agent"
    printf '[{"id": "fake", "name": "Fake agent", "command": "%s"}, {"id": "plain", "name": "Plain agent", "command": "%s", "env": {"FAKE_ACP_NO_MODES": "1"}}]\n' \
        "$fake" "$fake" > "$SILT_HOME/agents.json"
    git clone -q "$work/origin.git" "$work/$home/repo"
    cd "$work/$home/repo"
    printf '%s\n' "$password" | "$silt" auth login --server "$url" --user "$user" --name "$home" > /dev/null
}

machine alice
"$silt" claude > /dev/null 2>&1
thread=$("$silt" log | awk 'NR == 1 { print $1 }')
"$silt" thread share "$thread" bob > /dev/null
# New threads of this project start with the plain agent; setting that
# takes an org owner or a server admin.
"$silt" server user admin alice --data "$work/server" > /dev/null
"$silt" project set --agent plain > /dev/null
"$silt" server user admin alice --revoke --data "$work/server" > /dev/null
alice_url=$("$silt" open --no-browser)
git clone -q "$work/tools.git" "$work/alice/tools"
(cd "$work/alice/tools" && "$silt" open --no-browser > /dev/null)

machine bob
for _ in $(seq 1 100); do
    "$silt" log | grep -q "$thread" && break
    sleep 0.2
done
bob_url=$("$silt" open --no-browser)

# Alice's second machine, which runs nothing of its own.
machine alice alice-laptop
laptop_url=$("$silt" open --no-browser)

printf '{"alice":"%s","bob":"%s","laptop":"%s","server":"%s","thread":"%s","work":"%s"}\n' \
    "$alice_url" "$bob_url" "$laptop_url" "$url" "$thread" "$work"
