#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

# The web UI is embedded in the binary, so it is built first.
if [ -f web/package.json ]; then
    pnpm -C web install --frozen-lockfile
    pnpm -C web lint
    # With the mocks, which the screen references draw as the boards do (ADR 0039).
    VITE_SILT_MOCKS=on pnpm -C web build
    pnpm -C web test
fi

cargo fmt --all -- --check
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Windows (ADR 0045), cross-checked when its target and MinGW are installed;
# CI's Windows job runs the tests there.
if rustup target list --installed 2>/dev/null | grep -qx x86_64-pc-windows-gnu && command -v x86_64-w64-mingw32-gcc >/dev/null; then
    cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
else
    echo "gate: skipping the Windows check, as the x86_64-pc-windows-gnu target or MinGW isn't installed" >&2
fi

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

# The speech helper is its own workspace, as it links the speech engine's
# native library, which its first build downloads (ADR 0061). Linux only for now.
if [ "$(uname -s)" = Linux ]; then
    (
        cd crates/silt-voice
        cargo fmt -- --check
        cargo build
        cargo clippy --all-targets -- -D warnings
        cargo test
    )
else
    echo "gate: skipping crates/silt-voice, which is built on Linux only for now" >&2
fi

# Performance budgets (ADR 0034), timed on an optimized build.
cargo build --profile perf -p silt-cli -p silt-bench
target/perf/silt-bench check --tolerance "${SILT_BENCH_TOLERANCE:-0}"
cargo build -p silt-types -p silt-sumtree -p silt-crdt -p silt-log -p silt-checkout -p silt-wasm --target wasm32-unknown-unknown

# Playwright drives the built UI against a real daemon, the optimized one,
# as `web.thread` is timed there.
if [ -f web/package.json ]; then
    SILT="$PWD/target/perf/silt" pnpm -C web e2e
fi
