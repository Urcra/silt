#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

# The web UI is embedded in the binary, so it is built first.
if [ -f web/package.json ]; then
    pnpm -C web install --frozen-lockfile
    pnpm -C web lint
    pnpm -C web build
    pnpm -C web test
fi

cargo fmt --all -- --check
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
# The sandbox's tests skip where namespaces are off; here they must run.
SILT_REQUIRE_SANDBOX="${SILT_REQUIRE_SANDBOX:-1}" cargo test --workspace
cargo build -p silt-types -p silt-sumtree -p silt-crdt -p silt-log -p silt-checkout -p silt-wasm --target wasm32-unknown-unknown

# Playwright drives the built UI against a real daemon.
if [ -f web/package.json ]; then
    pnpm -C web e2e
fi
