#!/bin/sh
# Installs what the gate needs in a Claude Code cloud container. The
# environment's setup script, which runs one directory above the checkout,
# calls it as `silt/scripts/cloud-setup.sh`. Safe to run again.
set -eu
cd "$(dirname "$0")/.."

# Rust as rust-toolchain.toml pins it, with clippy, rustfmt and wasm32.
rustup show active-toolchain >/dev/null 2>&1 || rustup toolchain install
rustup show >/dev/null

# The browser replica's bindings, pinned to silt-wasm's wasm-bindgen.
WASM_BINDGEN=0.2.126
if [ "$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')" != "$WASM_BINDGEN" ]; then
    cargo install --locked wasm-bindgen-cli --version "$WASM_BINDGEN"
fi

# Only needed to add migrations or change `query!`s; slow to build, so opt in.
if [ "${SILT_SETUP_SQLX:-0}" = 1 ] && ! command -v sqlx >/dev/null; then
    cargo install --locked sqlx-cli
fi

pnpm -C web install --frozen-lockfile

# The container's own Chromium is older than Playwright's, and draws text
# differently from the screen references, so install Playwright's beside it.
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD= pnpm -C web exec playwright install chromium

# The crates, so the first build starts at once.
cargo fetch --locked
