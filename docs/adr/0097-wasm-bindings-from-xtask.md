# 0097: The browser replica's bindings come from xtask

- Status: accepted
- Date: 2026-11-17

## Context
The web UI imports `silt-wasm` through the JavaScript that wasm-bindgen generates. The generator has to be the exact version `silt-wasm` links, so building Silt from source meant `cargo install wasm-bindgen-cli --version 0.2.126` first. That version was written in five places: `silt-wasm`'s manifest, `silt-web`'s build script, `cloud-setup.sh`, the release workflow and the docs. It was the snag most often hit on a fresh `just install` or `cargo install --git`. Nothing caught a pin bumped in one place and not the others.

## Decision
**`cargo xtask wasm-bindgen <wasm> <out-dir>` generates the bindings,** with `wasm-bindgen-cli-support`, using the settings of `wasm-bindgen --target web`. `web/scripts/build-wasm.sh` runs it.
- **One pin.** `wasm-bindgen` and `wasm-bindgen-cli-support` are both `[workspace.dependencies]`, side by side, each at `=0.2.126`.
- **The same output.** The JavaScript and TypeScript are byte for byte what the CLI writes. The `.wasm` differs only in its producers section, which names the walrus version.

## Consequences
- Building the web UI needs only Node.js with pnpm and the `wasm32-unknown-unknown` target.
- The first web build compiles the generator, walrus and wasmparser among it, which takes about half a minute. The gate's later `cargo build` reuses that build.
- A wasm-bindgen upgrade is a single edit to the workspace manifest.
