#!/bin/sh
# Decides whether a change is done (docs/contributing.md). The cheap checks
# come first, so most mistakes fail in a minute rather than after the tests.
# It runs on every pull request before a merge (.github/workflows/claude.yml);
# running it locally is optional (ADR 0131).
#
# Usage: scripts/gate.sh [--quick | --part checks|tests|perf]
#   --quick  formatting, lints and the workspace's tests only: for iterating.
#   --part   one third of the gate, for running the thirds on separate
#            machines at once (.github/workflows/claude.yml): checks is
#            formatting, licences, lints, clippy and the wasm, web, desktop
#            and speech checks; tests is the build and the tests; perf is the
#            perf build, the budgets and the end-to-end tests.
set -eu
cd "$(dirname "$0")/.."

usage() {
    echo "usage: scripts/gate.sh [--quick | --part checks|tests|perf]" >&2
    exit 2
}
quick=false
part=
case "${1:-}" in
    "") ;;
    --quick) quick=true ;;
    --part)
        case "${2:-}" in
            checks | tests | perf) part=$2 ;;
            *) usage ;;
        esac
        ;;
    *) usage ;;
esac
# Whether this run does a step of the given part: all of them without --part.
in_part() {
    [ -z "$part" ] || [ "$part" = "$1" ]
}

started=$(date +%s)

# Each step is named on stdout. The summary at the end says where the time
# went.
summary=
current=
since=$started
elapsed() {
    s=$(($(date +%s) - $1))
    printf '%d:%02d' $((s / 60)) $((s % 60))
}
done_step() {
    if [ -n "$current" ]; then
        summary="$summary$(printf '%8s  %s' "$(elapsed "$since")" "$current")
"
    fi
}
step() {
    done_step
    current=$1
    since=$(date +%s)
    echo "gate: [$(elapsed "$started")] $1"
}
skip() {
    echo "gate: skipping $1" >&2
}

# A full gate's builds take about 25 GB under target/, its end-to-end tests
# a few more in /tmp, and a cloud container has about 40 GB in all. Running
# out fails a step ten minutes in, with an error that doesn't say why.
free_gb=$(df -Pk . | awk 'NR == 2 {print int($4 / 1048576)}')
if [ "${free_gb:-10}" -lt 10 ]; then
    echo "gate: only $free_gb GB of disk free, which a full gate may run out of; deleting target/debug/incremental and target/perf, which rebuild, frees the most" >&2
fi

has_web=false
[ -f web/package.json ] && has_web=true

if $has_web; then
    step "web dependencies"
    sh scripts/check-pnpm.sh
    # Corepack reads the root's, which `pnpm -C web` starts from; pnpm reads web's.
    node -e 'const [r, w] = ["./package.json", "./web/package.json"].map((p) => require(p).packageManager);
        if (r !== w) { console.error(`package.json names ${r} but web/package.json ${w}; make them the same`); process.exit(1); }'
    pnpm -C web install --frozen-lockfile
fi

# The desktop app and the speech helper are workspaces of their own, checked
# further down where they can be built.
has_desktop=false
pkg-config --exists webkit2gtk-4.1 2>/dev/null && has_desktop=true
has_voice=false
[ "$(uname -s)" = Linux ] && [ "${SILT_SKIP_VOICE:-0}" = 0 ] && has_voice=true

if in_part checks; then
step "formatting"
cargo fmt --all -- --check
if $has_voice; then (cd crates/silt-voice && cargo fmt -- --check); fi
if $has_desktop; then (cd crates/silt-desktop && cargo fmt -- --check); fi
(cd plugins && cargo fmt --all -- --check)
if $has_web; then pnpm -C web fmt:check; fi

# Every dependency's licence is one deny.toml allows, in each workspace;
# the web UI's build checks its own.
if command -v cargo-deny >/dev/null; then
    step "licences"
    cargo deny --log-level error check licenses
    (cd plugins && cargo deny --log-level error --config ../deny.toml check licenses)
    if $has_voice; then (cd crates/silt-voice && cargo deny --log-level error --config ../../deny.toml check licenses); fi
    if $has_desktop; then (cd crates/silt-desktop && cargo deny --log-level error --config ../../deny.toml check licenses); fi
else
    skip "the licence check, as cargo-deny isn't installed (cargo install --locked cargo-deny)"
fi

if $has_web; then
    step "web lint"
    pnpm -C web lint
fi
fi

# The UI is embedded in the binary, so it's built before cargo builds
# silt-web, by every part. With the mocks, which the screen references draw
# as the boards do (ADR 0039).
if $has_web; then
    step "web build"
    VITE_SILT_MOCKS=on pnpm -C web build
fi

if in_part checks; then
step "clippy"
cargo clippy --workspace --all-targets -- -D warnings
# The plugins Silt ships, and the probe its host is tested with (ADR 0104),
# a workspace of their own built for wasm32-wasip2.
(cd plugins && cargo clippy --workspace --target wasm32-wasip2 --target-dir ../target/plugins -- -D warnings)
(cd plugins && cargo test --workspace --target-dir ../target/plugins --quiet)

# Windows (ADR 0045), cross-checked when its target and MinGW are installed;
# nothing else checks it before a merge (ADR 0119).
if rustup target list --installed 2>/dev/null | grep -qx x86_64-pc-windows-gnu && command -v x86_64-w64-mingw32-gcc >/dev/null; then
    step "clippy for Windows"
    cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
else
    skip "the Windows check, as the x86_64-pc-windows-gnu target or MinGW isn't installed"
fi

if ! $quick; then
    # The pure crates build for the browser (ADR 0004).
    step "wasm32 build of the pure crates"
    cargo build -p silt-types -p silt-sumtree -p silt-crdt -p silt-log -p silt-checkout -p silt-wasm -p silt-workspace -p silt-design -p silt-theme -p silt-sheet -p silt-access -p silt-plugin -p silt-forge --target wasm32-unknown-unknown
fi

if $has_web; then
    step "web unit tests"
    # `pnpm test` would build the wasm again; the web build just did.
    pnpm -C web exec vitest run
fi
fi

if in_part tests; then
if ! $quick; then
    # Every binary, the ones no test runs too.
    step "build"
    cargo build --workspace
fi

step "tests"
# nextest runs every test binary at once rather than one after another,
# which saves minutes; it has no doctests, so those run apart.
if cargo nextest --version >/dev/null 2>&1; then
    # Most tests wait on the daemons and agents they start rather than
    # compute, so twice as many as there are cores keep the cores busy.
    # More than that made the tests already flaky under load fail.
    cargo nextest run --workspace --test-threads "$(($(getconf _NPROCESSORS_ONLN) * 2))"
    cargo test --workspace --doc
else
    skip "nextest, which isn't installed (cargo install --locked cargo-nextest); cargo test runs the tests one binary at a time"
    cargo test --workspace
fi
fi

if $quick; then
    done_step
    printf '%s' "$summary"
    echo "gate: quick checks passed in $(elapsed "$started"); the full gate is still to run"
    exit 0
fi

if in_part checks; then
# The desktop app is its own workspace, as it needs WebKitGTK (ADR 0041).
if $has_desktop; then
    step "desktop app"
    (
        cd crates/silt-desktop
        cargo build
        cargo clippy --all-targets -- -D warnings
        cargo test
    )
else
    skip "crates/silt-desktop, as WebKitGTK 4.1 isn't installed (scripts/cloud-setup.sh installs it)"
fi

# The speech helper is its own workspace, as it links the speech engine's
# native library, which its first build downloads (ADR 0061). Linux only for
# now.
if $has_voice; then
    step "speech helper"
    (
        cd crates/silt-voice
        cargo build
        cargo clippy --all-targets -- -D warnings
        cargo test
    )
else
    skip "crates/silt-voice, which is built on Linux only for now, and not with SILT_SKIP_VOICE=1"
fi
fi

# Performance budgets (ADR 0034), timed on an optimized build. Their limits
# are set for the machines Silt is developed on, with looser ones for a
# cloud session's (ADR 0037) and a GitHub Actions runner's (ADR 0126). Any
# other slower machine reports them with SILT_BENCH_ENFORCE=0.
enforce_budgets=true
[ "${SILT_BENCH_ENFORCE:-1}" = 0 ] && enforce_budgets=false
if in_part perf; then
step "perf build"
cargo build --profile perf -p silt-cli -p silt-bench
step "performance budgets"
if $enforce_budgets; then
    target/perf/silt-bench check --tolerance "${SILT_BENCH_TOLERANCE:-0}"
else
    target/perf/silt-bench report
fi

# Playwright drives the built UI against a real daemon, the optimized one,
# as `web.thread` is timed there.
if $has_web; then
    # The fixtures run the debug build and the fake agents, which the tests
    # part builds; on its own runner, the perf part builds them itself.
    if [ "$part" = perf ]; then
        step "debug build for the end-to-end fixtures"
        cargo build -p silt-cli -p silt-fake-agent
    fi
    step "end-to-end tests"
    SILT="$PWD/target/perf/silt" pnpm -C web e2e
fi
fi

done_step
printf '%s' "$summary"
if [ -n "$part" ]; then
    echo "gate: the $part part passed in $(elapsed "$started")"
elif ! $enforce_budgets; then
    echo "gate: passed in $(elapsed "$started") with the budgets only reported"
else
    echo "gate: passed in $(elapsed "$started")"
fi
