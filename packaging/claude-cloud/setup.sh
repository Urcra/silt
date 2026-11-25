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

# Built from source. Recording needs neither the web UI nor the plugins
# Silt ships, nor the tools that build them.
if ! command -v silt >/dev/null; then
    SILT_SKIP_WEB_BUILD=1 SILT_SKIP_PLUGIN_BUILD=1 cargo install --locked --git https://github.com/Urcra/silt silt-cli
fi

if [ -n "${SILT_SERVER:-}" ]; then
    printf '%s\n' "$SILT_PASSWORD" |
        silt auth login --server "$SILT_SERVER" --user "$SILT_USER" --password-stdin --name "cloud-$(hostname)"
fi
