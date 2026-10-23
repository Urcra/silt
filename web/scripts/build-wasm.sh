#!/bin/sh
# Builds the browser replica (crates/silt-wasm) into src/wasm, where the web
# UI imports it. Needs `cargo install wasm-bindgen-cli --version 0.2.126`.
set -eu
cd "$(dirname "$0")/../.."
cargo build -q -p silt-wasm --target wasm32-unknown-unknown --release
target=${CARGO_TARGET_DIR:-target}
wasm-bindgen --target web --out-dir web/src/wasm "$target/wasm32-unknown-unknown/release/silt_wasm.wasm"
