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

# A release binary when one can be downloaded, else built from source. The
# script is fetched first: a failed download piped into `sh` would succeed.
if ! command -v silt >/dev/null; then
    if install=$(curl -fsSL https://raw.githubusercontent.com/Urcra/silt/main/scripts/install.sh) &&
        printf '%s\n' "$install" | SILT_INSTALL_DIR=/usr/local/bin sh; then
        :
    else
        # Recording needs no web UI, nor the tools that build it.
        SILT_SKIP_WEB_BUILD=1 cargo install --locked --git https://github.com/Urcra/silt silt-cli
    fi
fi

if [ -n "${SILT_SERVER:-}" ]; then
    printf '%s\n' "$SILT_PASSWORD" |
        silt auth login --server "$SILT_SERVER" --user "$SILT_USER" --password-stdin --name "cloud-$(hostname)"
fi
