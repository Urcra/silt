#!/bin/sh
# Builds the browser replica (crates/silt-wasm) into src/wasm, where the web
# UI imports it. The bindings come from xtask, which links the same
# wasm-bindgen as silt-wasm (ADR 0097).
set -eu
cd "$(dirname "$0")/../.."
cargo build -q --locked -p silt-wasm --target wasm32-unknown-unknown --release
target=${CARGO_TARGET_DIR:-target}
cargo run -q --locked -p xtask -- wasm-bindgen "$target/wasm32-unknown-unknown/release/silt_wasm.wasm" web/src/wasm
