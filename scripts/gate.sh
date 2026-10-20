#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p silt-types -p silt-sumtree -p silt-crdt --target wasm32-unknown-unknown

if [ -f web/package.json ]; then
    pnpm -C web install --frozen-lockfile
    pnpm -C web lint
    pnpm -C web build
    pnpm -C web test
fi
