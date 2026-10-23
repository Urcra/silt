#!/usr/bin/env bash
# Follows docs/guide/index.md and docs/guide/import.md on a fresh
# container: installs `silt` with scripts/install.sh from a release built
# here, then imports the Claude Code sessions already on disk and finds
# them with `silt search` (ADR 0043). Every command run from the guide must
# appear in it verbatim.
#
# Usage: scripts/install-walkthrough.sh [path/to/static/silt]
# Needs docker; builds a static x86_64 binary when none is given, as the
# release workflow does (needs musl-tools).
set -euo pipefail
cd "$(dirname "$0")/.."

GUIDES="docs/guide/index.md docs/guide/import.md"
IMAGE=${SILT_WALKTHROUGH_IMAGE:-debian:trixie-slim}
TARGET=x86_64-unknown-linux-musl
RUN=silt-install-$$
INSTALL_URL=https://raw.githubusercontent.com/Urcra/silt/main/scripts/install.sh

if [ $# -ge 1 ]; then
    BIN=$1
else
    CC=musl-gcc RUSTFLAGS="-C target-feature=+crt-static" cargo build --locked --release -q -p silt-cli --target "$TARGET"
    BIN=target/$TARGET/release/silt
fi

RELEASE=$(mktemp -d)
cleanup() {
    docker rm -f "$RUN" >/dev/null 2>&1 || true
    rm -rf "$RELEASE"
}
trap cleanup EXIT

say() { printf '\n== %s\n' "$*"; }

# Runs a command from the guide. The install script comes from this
# checkout and the release from the archive built here.
guide() {
    local command=$1
    # shellcheck disable=SC2086
    grep -qF -- "$command" $GUIDES || { echo "not in the guide: $command" >&2; exit 1; }
    command=${command//$INSTALL_URL/file:///src/scripts/install.sh}
    command=${command//<words>/farewell}
    printf '$ %s\n' "$command" >&2
    docker exec -i -e SILT_DOWNLOAD_URL=file:///release -w /root/src/app "$RUN" bash -lc "$command"
}

say "a release, as the workflow publishes it"
cp "$BIN" "$RELEASE/silt"
tar -czf "$RELEASE/silt-$TARGET.tar.gz" -C "$RELEASE" silt
rm "$RELEASE/silt"
(cd "$RELEASE" && sha256sum "silt-$TARGET.tar.gz" > SHA256SUMS)

say "a fresh machine with a clone and Claude Code sessions from before Silt"
docker pull -q "$IMAGE" >/dev/null
docker run -d --name "$RUN" -v "$PWD/scripts:/src/scripts:ro" -v "$RELEASE:/release:ro" "$IMAGE" sleep infinity >/dev/null
docker exec "$RUN" bash -c 'apt-get update -q >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y -q curl git >/dev/null'
docker exec "$RUN" bash -c '
    set -e
    git config --global init.defaultBranch main
    git init -q /root/src/app && cd /root/src/app
    mkdir src && printf "fn main() {}\n" > src/main.rs
    git add . && git -c user.name=me -c user.email=me@example.invalid commit -qm init
    git remote add origin https://github.com/acme/app.git
    dir="$HOME/.claude/projects/$(pwd | sed "s/[^A-Za-z0-9]/-/g")"
    mkdir -p "$dir"
    for n in 1 2; do
        printf "{\"type\":\"user\",\"cwd\":\"%s\",\"timestamp\":\"2026-03-0%sT10:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"Session %s: add a farewell line to main\"}}\n{\"type\":\"assistant\",\"timestamp\":\"2026-03-0%sT10:01:00.000Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Added fn farewell.\"}]}}\n" \
            "$(pwd)" "$n" "$n" "$n" > "$dir/6f1c2d3e-0000-4000-8000-00000000000$n.jsonl"
    done'

say "1. install"
guide "curl -fsSL $INSTALL_URL | sh"
docker exec "$RUN" bash -c 'printf "export PATH=\"\$HOME/.local/bin:\$PATH\"\n" >> ~/.profile'

say "2. import and search"
IMPORTED=$(guide "silt import")
printf '%s\n' "$IMPORTED"
grep -q "imported 2 sessions" <<<"$IMPORTED" || { echo "both sessions should be imported" >&2; exit 1; }
FOUND=$(guide 'silt search "<words>"')
printf '%s\n' "$FOUND"
[ "$(grep -c 'farewell' <<<"$FOUND")" -ge 2 ] || { echo "search should find both sessions" >&2; exit 1; }
AGAIN=$(guide "silt import")
grep -q "no sessions to import" <<<"$AGAIN" || { echo "importing twice should add nothing: $AGAIN" >&2; exit 1; }

say "the guide works from install to a searchable history"
