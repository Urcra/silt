#!/usr/bin/env bash
# Follows docs/self-hosting.md on fresh containers: a server and two
# people's machines that start with nothing but the silt binary. Alice
# records a thread, Bob forks it and offers his work back, and Alice merges
# it. Every command run from the guide must appear in it verbatim, and the
# whole run must take under ten minutes.
#
# Usage: scripts/selfhost-walkthrough.sh [path/to/silt]
# Needs docker; builds a release binary when none is given.
set -euo pipefail
cd "$(dirname "$0")/.."

GUIDE=docs/self-hosting.md
IMAGE=${SILT_WALKTHROUGH_IMAGE:-debian:trixie-slim}
LIMIT_SECONDS=600
URL=http://server:7450
RUN=silt-walkthrough-$$

if [ $# -ge 1 ]; then
    BIN=$1
else
    cargo build --release -q -p silt-cli
    BIN=target/release/silt
fi

cleanup() {
    docker rm -f "$RUN-server" "$RUN-alice" "$RUN-bob" >/dev/null 2>&1 || true
    docker network rm "$RUN" >/dev/null 2>&1 || true
}
trap cleanup EXIT

say() { printf '\n== %s\n' "$*"; }

THREAD="" FORK="" MR=""

# Runs a command from the guide on a machine. The guide's placeholder URL
# becomes the server container, `<thread>`, `<fork>` and `<mr>` the ids
# seen so far, and `sudo` is dropped: containers run as root.
guide() {
    local machine=$1 command=$2
    grep -qF -- "$command" "$GUIDE" || { echo "not in $GUIDE: $command" >&2; exit 1; }
    command=${command//https:\/\/silt.example.com/$URL}
    command=${command//<thread>/$THREAD}
    command=${command//<fork>/$FORK}
    command=${command//<mr>/$MR}
    command=${command//sudo /}
    printf '%s$ %s\n' "$machine" "$command" >&2
    docker exec -i -e DEBIAN_FRONTEND=noninteractive -w /root/src/app "$RUN-$machine" bash -lc "$command"
}

# Like `guide`, retrying until the output contains `expect`.
eventually() {
    local machine=$1 command=$2 expect=$3 output=""
    for _ in $(seq 1 60); do
        output=$(guide "$machine" "$command" 2>/dev/null || true)
        if grep -qF -- "$expect" <<<"$output"; then
            printf '%s\n' "$output"
            return 0
        fi
        sleep 1
    done
    echo "$machine never showed '$expect' for: $command" >&2
    printf '%s\n' "$output" >&2
    exit 1
}

say "pulling $IMAGE"
docker pull -q "$IMAGE" >/dev/null
START=$(date +%s)

say "starting three fresh machines"
docker network create "$RUN" >/dev/null
for machine in server alice bob; do
    docker run -d --name "$RUN-$machine" --hostname "$machine" --network "$RUN" --network-alias "$machine" \
        "$IMAGE" sleep infinity >/dev/null
    docker exec "$RUN-$machine" mkdir -p /root/src/app
    docker cp "$BIN" "$RUN-$machine:/usr/local/bin/silt"
done

say "1. install (the binary is copied in; git comes from the guide)"
for machine in server alice bob; do
    guide "$machine" "sudo apt-get update && sudo apt-get install -y git" >/dev/null
done

say "the team's repository, cloned on both people's machines"
SEED=$(mktemp -d)
git -C "$SEED" init -q -b main
mkdir -p "$SEED/src"
printf 'fn main() {}\n' > "$SEED/src/main.rs"
git -C "$SEED" add .
git -C "$SEED" -c user.name=seed -c user.email=seed@example.invalid commit -qm init
git -C "$SEED" bundle create -q "$SEED/app.bundle" HEAD main
for machine in alice bob; do
    docker cp "$SEED/app.bundle" "$RUN-$machine:/tmp/app.bundle"
    docker exec "$RUN-$machine" bash -c 'git config --global init.defaultBranch main && rm -rf /root/src/app && git clone -q --bare /tmp/app.bundle /srv/git/app.git && git clone -q /srv/git/app.git /root/src/app && git config --global user.name "$(hostname)" && git config --global user.email "$(hostname)@example.invalid"'
    # A stand-in for Claude Code: records a session and edits main.rs.
    docker exec -i "$RUN-$machine" bash -c 'cat > /usr/local/bin/claude && chmod +x /usr/local/bin/claude' <<'FAKE'
#!/bin/sh
set -eu
session="" resume=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --resume) session="$2"; resume=1; shift 2 ;;
        *) shift ;;
    esac
done
dir="$HOME/.claude/projects/$(pwd | sed 's/[^A-Za-z0-9]/-/g')"
mkdir -p "$dir"
me=$(hostname)
printf '{"type":"user","cwd":"%s","sessionId":"%s","message":{"role":"user","content":"%s: add a line to main"}}\n' "$(pwd)" "$session" "$me" >> "$dir/$session.jsonl"
printf 'fn %s() {}\n' "$me" >> src/main.rs
sleep 1
printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Added fn %s."}]}}\n' "$me" >> "$dir/$session.jsonl"
FAKE
done
rm -rf "$SEED"

say "2. set up the server"
guide server "silt server init --data /var/lib/silt --listen 0.0.0.0:7450 --public-url https://silt.example.com" >/dev/null
docker exec -d "$RUN-server" bash -lc "silt server run --data /var/lib/silt > /var/log/silt-server.log 2>&1"
grep -qF -- "silt server run --data /var/lib/silt &" "$GUIDE"

say "3. add the team"
password() { sed -n 's/.*password: \([^ ]*\).*/\1/p'; }
for _ in $(seq 1 20); do docker exec "$RUN-server" bash -c 'exec 3<>/dev/tcp/127.0.0.1/7450' 2>/dev/null && break; sleep 0.5; done
ALICE_PASSWORD=$(guide server "silt server user add alice --data /var/lib/silt" | password)
BOB_PASSWORD=$(guide server "silt server user add bob --data /var/lib/silt" | password)

say "4. sign in from each machine"
printf '%s\n' "$ALICE_PASSWORD" | guide alice "silt auth login --server https://silt.example.com --user alice"
printf '%s\n' "$BOB_PASSWORD" | guide bob "silt auth login --server https://silt.example.com --user bob"

say "5. work together"
guide alice "silt claude"
grep -qF -- "cd ~/src/app" "$GUIDE"
LOG=$(eventually bob "silt log" "2 msgs")
THREAD=$(awk '{print $1; exit}' <<<"$LOG")
guide bob "silt fork <thread>"
FORK=$(docker exec "$RUN-bob" bash -lc "silt log" | awk -v thread="$THREAD" '$0 ~ "fork of " thread {print $1; exit}')
[ -n "$FORK" ] || { echo "Bob's fork is not listed" >&2; exit 1; }
OPENED=$(guide bob "silt mr create --thread <fork> --title \"Add Bob's change\"")
printf '%s\n' "$OPENED"
MR=$(awk '/opened merge request/ {sub(":", "", $4); print $4; exit}' <<<"$OPENED")
[ -n "$MR" ] || { echo "no merge request was opened" >&2; exit 1; }
eventually alice "silt inbox" "asks to merge"
guide alice "silt mr show <mr>"
guide alice "silt mr approve <mr>"
for _ in $(seq 1 30); do
    if docker exec "$RUN-alice" grep -q 'fn bob() {}' /root/src/app/src/main.rs; then
        break
    fi
    sleep 1
done
docker exec "$RUN-alice" cat /root/src/app/src/main.rs
docker exec "$RUN-alice" grep -q 'fn bob() {}' /root/src/app/src/main.rs || { echo "Bob's change never reached Alice" >&2; exit 1; }

ELAPSED=$(( $(date +%s) - START ))
say "done in ${ELAPSED}s (limit ${LIMIT_SECONDS}s)"
[ "$ELAPSED" -lt "$LIMIT_SECONDS" ]
