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

# Pinned to silt-wasm's wasm-bindgen. The release's binary takes seconds;
# building it takes minutes, so that's only the fallback.
WASM_BINDGEN=0.2.126
if [ "$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')" != "$WASM_BINDGEN" ]; then
    step "wasm-bindgen $WASM_BINDGEN"
    release="wasm-bindgen-$WASM_BINDGEN-x86_64-unknown-linux-musl"
    bin="${CARGO_HOME:-$HOME/.cargo}/bin"
    mkdir -p "$bin"
    if curl -sSfL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$WASM_BINDGEN/$release.tar.gz" |
        tar xz -C "$bin" --strip-components=1 "$release/wasm-bindgen" "$release/wasm-bindgen-test-runner" "$release/wasm2es6js"; then
        :
    else
        step "no release binary; building wasm-bindgen $WASM_BINDGEN"
        cargo install --locked wasm-bindgen-cli --version "$WASM_BINDGEN"
    fi
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

# Tauri's system libraries, for the desktop app (ADR 0041).
if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    step "WebKitGTK for the desktop app"
    sudo=
    [ "$(id -u)" = 0 ] || sudo=sudo
    $sudo apt-get update -q
    DEBIAN_FRONTEND=noninteractive $sudo apt-get install -y -q pkg-config libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
fi

step "crates"
cargo fetch --locked
cargo fetch --locked --manifest-path crates/silt-desktop/Cargo.toml

step "done"
