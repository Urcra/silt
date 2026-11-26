#!/bin/sh
# Installs what the gate needs in a Claude Code cloud container. The
# environment's setup script, which runs one directory above the checkout,
# either calls it as `silt/scripts/cloud-setup.sh` or holds a copy of it.
# Safe to run again.
set -eu

step() { echo "==> cloud-setup: $*"; }

# The checkout: the one this file is in, or, for a copy pasted into the
# setup script, `silt` beside where it runs.
for root in "$(dirname "$0")/.." ./silt .; do
    if [ -f "$root/rust-toolchain.toml" ] && [ -f "$root/web/package.json" ]; then
        cd "$root"
        break
    fi
    root=
done
if [ -z "$root" ]; then
    echo "cloud-setup: no Silt checkout here ($(pwd)) or in ./silt" >&2
    exit 1
fi
step "checkout at $(pwd)"

step "Rust toolchain from rust-toolchain.toml"
rustup show active-toolchain >/dev/null 2>&1 || rustup toolchain install
rustup show >/dev/null

# The gate runs the tests with nextest when it's installed, every test
# binary at once rather than one after another. The release's binary takes
# seconds; building it takes minutes, so that's only the fallback.
NEXTEST=0.9.152
if [ "$(cargo nextest --version 2>/dev/null | awk 'NR == 1 {print $2}')" != "$NEXTEST" ]; then
    step "cargo-nextest $NEXTEST"
    bin="${CARGO_HOME:-$HOME/.cargo}/bin"
    mkdir -p "$bin"
    if curl -sSfL "https://get.nexte.st/$NEXTEST/linux" | tar xz -C "$bin"; then
        :
    else
        step "no release binary; building cargo-nextest $NEXTEST"
        cargo install --locked cargo-nextest --version "$NEXTEST"
    fi
fi

# The gate checks every dependency's licence with cargo-deny.
DENY=0.22.1
if [ "$(cargo deny --version 2>/dev/null | awk 'NR == 1 {print $2}')" != "$DENY" ]; then
    step "cargo-deny $DENY"
    cargo install --locked cargo-deny --version "$DENY"
fi

# A full gate's builds outgrow a cloud container's disk with full debug
# info, and nobody steps through code with a debugger there, so the
# workspace's crates get only line tables, as dependencies do, still enough
# for backtraces. In this machine's own cargo config, not the repository's.
config="${CARGO_HOME:-$HOME/.cargo}/config.toml"
if ! grep -qs 'silt cloud-setup' "$config"; then
    step "line tables only for debug builds, in $config"
    printf '\n# silt cloud-setup: smaller debug builds, to fit the disk\n[profile.dev]\ndebug = "line-tables-only"\n' >>"$config"
fi
# Incremental builds keep several gigabytes beside each build, which a full
# gate ran out of disk on; nobody edits and rebuilds in a loop here.
if ! grep -qs 'silt cloud-setup: no incremental' "$config"; then
    step "no incremental builds, in $config"
    printf '\n# silt cloud-setup: no incremental builds, to fit the disk\n[build]\nincremental = false\n' >>"$config"
fi

# Only needed to add migrations or change `query!`s; slow to build, so opt in.
if [ "${SILT_SETUP_SQLX:-0}" = 1 ] && ! command -v sqlx >/dev/null; then
    step "sqlx-cli"
    cargo install --locked sqlx-cli
fi

step "web dependencies"
pnpm -C web install --frozen-lockfile

# The container's own Chromium is older than Playwright's, and draws text
# differently from the screen references, so install Playwright's beside it.
step "Playwright's Chromium"
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD= pnpm -C web exec playwright install chromium

# System packages, in one apt run: Tauri's libraries, for the desktop app
# (ADR 0041), and bubblewrap and pasta, which Silt's sandbox runs agents and
# builds third parties' plugins in (ADR 0093, ADR 0104); the tests that
# build a plugin from git fail without them.
packages=
pkg-config --exists webkit2gtk-4.1 2>/dev/null ||
    packages="pkg-config libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev"
command -v bwrap >/dev/null || packages="$packages bubblewrap"
command -v pasta >/dev/null || packages="$packages passt"
if [ -n "$packages" ]; then
    step "system packages:$packages"
    sudo=
    [ "$(id -u)" = 0 ] || sudo=sudo
    $sudo apt-get update -q
    # shellcheck disable=SC2086 # a list of package names
    DEBIAN_FRONTEND=noninteractive $sudo apt-get install -y -q $packages
fi

step "crates"
cargo fetch --locked
cargo fetch --locked --manifest-path crates/silt-desktop/Cargo.toml
cargo fetch --locked --manifest-path crates/silt-voice/Cargo.toml

# silt-voice's first build downloads the speech engine with webpki's roots,
# which don't trust a cloud container's proxy; curl does, and the build takes
# the archive from where it would have saved it (ADR 0061).
SHERPA=2.0.2
archive="sherpa-onnx-v$SHERPA-linux-x64-static-lib.tar.bz2"
prebuilt=crates/silt-voice/target/sherpa-onnx-prebuilt
if [ "$(uname -sm)" = "Linux x86_64" ] && [ ! -f "$prebuilt/$archive" ]; then
    step "the speech engine's library"
    mkdir -p "$prebuilt"
    curl -sSfL -o "$prebuilt/$archive.part" "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$SHERPA/$archive"
    mv "$prebuilt/$archive.part" "$prebuilt/$archive"
fi

step "done"
