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
cargo test --workspace

# The desktop app is its own workspace, as it needs WebKitGTK (ADR 0041).
if pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    (
        cd crates/silt-desktop
        cargo fmt -- --check
        cargo build
        cargo clippy --all-targets -- -D warnings
        cargo test
    )
else
    echo "gate: skipping crates/silt-desktop, as WebKitGTK 4.1 isn't installed (scripts/cloud-setup.sh installs it)" >&2
fi

# Performance budgets (ADR 0034), timed on an optimized build.
cargo build --profile perf -p silt-cli -p silt-bench
target/perf/silt-bench check
cargo build -p silt-types -p silt-sumtree -p silt-crdt -p silt-log -p silt-checkout -p silt-wasm --target wasm32-unknown-unknown

# Playwright drives the built UI against a real daemon, the optimized one,
# as `web.thread` is timed there.
if [ -f web/package.json ]; then
    SILT="$PWD/target/perf/silt" pnpm -C web e2e
fi
