#!/bin/sh
# The setup script of a Claude Code cloud environment that records its
# sessions with Silt (docs/guide/cloud.md). It runs once per container, as
# root, before Claude Code starts.
#
# The environment's variables give it:
#   SILT_SERVER    the server's URL, which the network policy must allow
#   SILT_USER      the account the container signs in as
#   SILT_PASSWORD  that account's password
set -eu

if ! command -v silt >/dev/null; then
    cargo install --locked --git https://github.com/Urcra/silt silt-cli
fi

if [ -n "${SILT_SERVER:-}" ]; then
    printf '%s\n' "$SILT_PASSWORD" |
        silt auth login --server "$SILT_SERVER" --user "$SILT_USER" --password-stdin --name "cloud-$(hostname)"
fi
