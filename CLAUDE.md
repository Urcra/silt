# Silt

Silt is an open-source (AGPL-3.0), self-hostable workspace for coding with agents whose sessions (code + chat) can be resumed anywhere, forked and merged back, built on a CRDT version-control layer that interoperates with git.

**Focus (ADR 0006):** async handoff between machines, forking in-progress work (code + chat), merge requests with approval, and full history and search across compaction. People keep using their own agent CLI (Claude Code first) and Silt captures underneath (ADR 0007). Live multiplayer comes last.

- How it's built: docs/architecture.md (crate by crate, with the ADR behind each part)
- How to work on it: docs/contributing.md (building, the gate, tests, dev loops, generated files)
- Research: docs/research/
- Decisions: docs/adr/

## Architecture map
- **Pure crates** (no I/O; the gate builds them for wasm32):
  - `silt-types`: ids;
  - `silt-sumtree`;
  - `silt-crdt`: the text CRDT;
  - `silt-log`: deltas, and the family as a projection of its log;
  - `silt-checkout`: disk ↔ tree, the write policy;
  - `silt-sync`: Sans-I/O `ClientSync`/`Room`;
  - `silt-proto`: the sync API types, `PROTOCOL`, and the views both serving modes answer with;
  - `silt-workspace`: workspaces, the projects they match, and the rules for agents started in them (ADR 0065);
  - `silt-design`: design systems' tokens, compiled to the CSS an artifact's frame gets (ADR 0071);
  - `silt-theme`: themes for Silt's UI: the format, derivation, the CSS a light and a dark theme compile to, contrast checks, and imports from VS Code and Zed (ADR 0098);
  - `silt-sheet`: sheets' formulas, evaluated alike on every surface (ADR 0077);
  - `silt-forge`: the forges' APIs as requests and answers, for the forge bridge and GitHub's plugin (ADR 0103);
  - `silt-access`: plugin calls classified, approvals bound to their request's hash, and data rules (ADR 0093);
  - `silt-plugin`: plugins' manifests, cards, effects, prepared calls and the rules that decide them (ADR 0104);
  - `silt-wasm`: the browser's replica.
- **Shell crates:**
  - `silt-store`: the SQLite log and blobs, with its own migrations;
  - `silt-git`: gix reads, `git` CLI writes;
  - `silt-agents`: one adapter per agent CLI;
  - `silt-acp`, `silt-mcp`, `silt-redact`;
  - `silt-gateway`: the proxy a sandboxed agent's HTTP(S) leaves through, adding credentials it never holds (ADR 0093);
  - `silt-llm`: local models found on Hugging Face, their checked downloads, fetching llama.cpp, and running its `llama-server` (ADR 0092);
  - `silt-local`: Silt's own agent, `local`, on a local model through the daemon's gateway (ADR 0092);
  - `silt-plugin-host`: the WebAssembly host plugins run in, on wasmtime, with only what each was granted (ADR 0104);
  - `silt-plugin-api`: the SDK plugins are written with, built for `wasm32-wasip2`; `silt-plugin-test`: a stand-in host for a plugin's tests;
  - `silt-shipped`: the plugins shipped with Silt (`plugins/`), built for `wasm32-wasip2` by its build script and embedded (`SILT_SKIP_PLUGIN_BUILD=1` skips them, ADR 0104);
  - `silt-vendor`: the CDN files artifacts load through Silt, fetched and kept (ADR 0071);
  - `silt-media`: what an image or a video is, images' previews, and a blob served by range (ADR 0121);
  - `silt-update`: which build this is (commit, clone, and its channel: stable, nightly or dev), whether a newer one is out, and an update's steps and state (ADR 0100, ADR 0130);
  - `silt-os`: what each OS does its own way (Windows' pipe and DACLs, detaching, links, WSL paths). The only crate with `unsafe`; platform code goes here rather than as `cfg`s in the others (ADR 0045).
- **Wiring:**
  - `silt-web`: the embedded web UI; its `build.rs` builds `web/dist` when missing (`SILT_SKIP_WEB_BUILD=1` skips it);
  - `silt-daemon`: one machine's capture, watchers, sync, control channel, ACP agents and web API (`/api`);
  - `silt-server`: sync, accounts, search, inbox, git mirror, the hosted app's `/app/api`, the forge bridge and the Agent SDK session store (ADR 0042). It never touches checkouts or agents;
  - `silt-cli`: the `silt` binary, linking both;
  - `silt-desktop`: the Tauri 2 app, its own workspace as it needs WebKitGTK. It runs `silt open --no-browser` and shows the UI (ADR 0041);
  - `silt-voice`: the speech helper, its own workspace as it links sherpa-onnx's native library. The daemon runs it per dictation (ADR 0061).
- **From capture to sync:**
  1. `silt claude` passes hooks that run `silt hook <event>`.
  2. Each hook runs one capture step (`capture.rs`). The daemon runs it if one runs, else the hook does in its own process; it's the same step either way (ADR 0018).
  3. The step goes to the store (`$SILT_HOME/silt.sqlite` + `blobs/`).
  4. `ClientSync` pushes it, redacted, to the family's home server (ADR 0032).
  5. The server keeps it in a `Room`, and others pull it.
- **Where state lives:**
  - a machine's data dir is `$SILT_HOME`, else `~/.local/share/silt`;
  - the server's is `--data`, by default `$SILT_HOME/server`;
  - managed checkouts are under `<repo>/.silt/checkouts/`.

  architecture.md has the full layout and the `SILT_*` variables.
- **`web/`:** React + Vite + TypeScript. It's embedded in the binary from `web/dist`, and served by the daemon at `/` and by the server at `/app/`.
- **`docs/`:** the site's pages (`guide/`, `self-hosting.md`, `reference/`, `architecture.md`, `contributing.md`), the ADRs and the research notes.
- **`templates/plugin/`:** a plugin to start one from, built and tested against this repository by `silt-plugin-test`'s tests.
- **`scripts/`:** `gate.sh`, `fetch-traces.sh`, `selfhost-walkthrough.sh`, which checks self-hosting.md on fresh containers, `wsl-handoff.sh`, which hands threads between Windows and a WSL distribution, `install-windows.sh`, which `just install-wsl` runs to build and install the Windows side from WSL, `cloud-setup.sh`, which installs the gate's tools in a Claude Code cloud container, `local-model-probe.sh`, which reports on a machine's GPU, llama.cpp's builds and Claude Code on a local model, for whoever wants to check theirs (ADR 0092), and `build-sandbox-tools.sh`, which builds the static bubblewrap and pasta that go beside `silt` in release archives, once there are releases.

## Generated files
Each one fails a test while it's out of date. Never edit them by hand.
- `web/src/api.generated.ts`: `UPDATE_API_TYPES=1 cargo test -p silt-daemon api_types`.
- `docs/reference/cli.md`: `UPDATE_CLI_REFERENCE=1 cargo test -p silt-cli --test reference`.
- `web/src/frame.generated.ts`: `UPDATE_FRAME=1 cargo test -p silt-design frame_ts`.
- `docs/reference/theme.schema.json`: `UPDATE_THEME_SCHEMA=1 cargo test -p silt-theme theme_schema`.
- `.sqlx/`: `cargo sqlx prepare --workspace -- --all-targets` (see Rules).
- Built, not committed:
  - `web/src/wasm/`: `pnpm -C web wasm`;
  - `web/dist/`: `pnpm -C web build`;
  - `target/site/`: `cargo run -p silt-site`.

## Test fixtures
- `crates/silt-cli/tests/common/`:
  - multi-machine tests on a real server, each machine with its own `SILT_HOME` and `CLAUDE_CONFIG_DIR`;
  - the agent CLIs' stand-ins are `silt-fake-agent`'s `silt-fake-<agent>` programs (`common::fake_agent`), which run everywhere.
- `silt-fake-agent`: a scripted ACP agent, driven by the `FAKE_*` variables, and `silt-fake-claude`, `-codex`, `-gemini`, `-copilot` and `-opencode`, the agent CLIs' stand-ins, on every platform. `cargo test --workspace` builds them; `cargo test -p silt-cli` alone needs `cargo build -p silt-fake-agent` first.
- `web/e2e/*fixture.sh`: the Playwright fixtures:
  - `fixture.sh`: a thread, a fork and an MR;
  - `live-fixture.sh`: a server and two people;
  - `workspaces-fixture.sh`: two servers;
  - `local-fixture.sh`: a Local only workspace with a local model, and `fake-hub.py` standing in for Hugging Face.
  They need `target/debug/silt`.
- `web/e2e/probe-repo.sh`: the probe plugin as a third party's git repository, with the SDK inside it, and `bump` to tag a newer version; `crates/silt-cli/tests/git_plugins.rs` builds the same.
- `silt-bench` fixtures are cached in `target/silt-bench`. The CRDT traces come from `scripts/fetch-traces.sh`, and tests skip without them.

## Pitfalls met so far
- **The web UI is embedded.** Run `pnpm -C web build` before `cargo build`, or `silt` serves a stale UI.
- **The two serving modes.** The daemon's `/api` and the server's `/app/api` answer the same client. A change to what the UI reads needs both, through `silt_proto::view`.
- **The protocol version.** A change to the wire format bumps `silt_proto::PROTOCOL`, and CHANGELOG says to update every machine and the server together.
- **Capture without a daemon.** Capture must stay correct with no daemon running. Put capture logic in the shared step, never only in the daemon's path (ADR 0018).
- **Peer content is untrusted.** Deltas come from possibly hostile peers. Writes to disk go through `silt_checkout::is_writable`, and ids are checked before they become file names or arguments (ADR 0016).
- **The gate runs on the pull request** (ADR 0130): CI on every push, and the full gate through `/claude gate` or `/claude merge` before a merge. Running `scripts/gate.sh` locally is optional; `scripts/gate.sh --quick` takes a few minutes and catches most of what CI would.
- **Screen references** are compared against the perf build, as the gate runs them: take them with `pnpm -C web e2e:update` and `SILT` set to the perf binary's absolute path, since the debug build renders some screens differently.
- **A fresh worktree's first gate is slow:** it builds the `perf` profile and wasm too.
- **In a Claude Code cloud session,** check the setup ran before running the gate there: `cargo nextest --version`, `bwrap --version`, and `debug = "line-tables-only"` in `~/.cargo/config.toml`. If not, run `sh silt/scripts/cloud-setup.sh` from the folder above the checkout. Run the gate in the background with its output in a file; it takes 20 to 30 minutes. Out of disk, delete `target/debug` and `target/perf`. docs/contributing.md has the rest.
- **Tests that start daemons** use a temporary `SILT_HOME`. Never point one at `~/.local/share/silt`.
- **The docs site** fails when a page under `docs/guide/` isn't in `PAGES` (`crates/silt-site/src/lib.rs`) or isn't linked from the guide. It also fails when a doc names a `silt` command the reference lacks.
- **The desktop app is its own workspace.** `cargo build --workspace` skips it; run `cargo` in `crates/silt-desktop`. The gate checks it only when `webkit2gtk-4.1` is installed. So is `crates/silt-voice`, which the gate checks on Linux.
- **WSL watchers:** `/mnt/<drive>` has no inotify. See Rules.
- **Windows.** There's no Windows CI job and no Windows release (ADR 0119). The gate cross-checks with clippy when `x86_64-pc-windows-gnu` and MinGW are installed; Wine and `just install-wsl` on a real machine check the rest. Only the `silt-cli` tests that send Unix signals or need a Unix terminal are `#[cfg(unix)]`. Paths in deltas stay `/`-separated.


## Working agreement
1. **Starting a session:** read `ROADMAP.md`, then the open pull requests and the GitHub issues of the milestones under Now. Continue what's in progress.
2. **Starting a task:** work from its issue, and say on the issue that it's being worked on.
   - Commit messages are Conventional Commits (`feat:`, `fix:`, `test:`, `docs:`, `refactor:`, `perf:`, `chore:`, `build:`), with an optional scope such as `feat(crdt): ...`.
   - They describe the change itself and never mention milestone or task ids (M1, M1-07, …).
   - Push after every commit.
   - Pull requests follow `.github/pull_request_template.md`. A UI change needs screenshots of each new or changed screen in its description, uploaded to the PR rather than committed.
   - Commit with git. No local gate is required before committing or pushing: CI runs on the pull request, and the full gate runs before it merges.
3. **UI work starts with mockups:** before writing code for a feature that touches the UI, make artifact mockups of the finished feature, and share them so it can be seen and agreed on first.
   - Make one for each new or changed screen and state: empty, loading, error and full.
   - Build them from Silt as it actually looks, not from the Silt or Nous design system alone. Take the app's real CSS (`web/src/nous.css`, `web/src/index.css`) and the markup its components render. Check against the running app (`target/debug/silt` on a `web/e2e/*fixture.sh` fixture).
   - Put the new parts in the real screen around them: sidebar, breadcrumbs, panels and realistic data. The mockup should show exactly how the feature will look in the app.
   - Show light and dark, and phone width where the screen works there.
4. **Finishing a task:** a task is done only after its pull request's CI and the full gate pass (`/claude gate`, or `/claude merge`, which merges only then). Then:
   - its pull request closes the issue (`Fixes #<n>`);
   - update `CHANGELOG.md` when behaviour changed.
5. **New work:** open an issue for newly discovered work, with the labels and milestone it fits; don't do it silently. Label what's blocked `blocked`, saying what unblocks it.
6. **Ending a session:** leave the issue of unfinished work saying where it stands and what's next.
7. **Decisions:** record a significant design decision, or a reversal of one, as a new ADR in `docs/adr/`.

## Rules
- Never copy code or assets from closed-source products. Open-source code may be used under its licence, credited in NOTICE (as Zed's `sum_tree` is).
- SQLx migrations: always create them with `sqlx migrate add -r <name>` and apply them with `sqlx migrate run`. Never hand-write migration files.
  - Run these from the crate dir, so its `sqlx.toml` (per-crate `table-name = "_silt_<crate>_migrations"`) applies.
  - Each crate has its own database variable, `SILT_STORE_DATABASE_URL` or `SILT_SERVER_DATABASE_URL` (`common.database-url-var` in its `sqlx.toml`), set to an absolute URL such as `sqlite://$(git rev-parse --show-toplevel)/target/dev-silt-store.db?mode=rwc` (relative `sqlite://../..` paths fail to open).
  - After changing a `query!`, run `cargo sqlx prepare --workspace -- --all-targets` from the root with both variables set, and `DATABASE_URL` too, which the CLI checks; commit the `.sqlx/` cache it writes. Builds use the cache when the variables are unset.
  - The CLI needs sqlite and sqlx-toml support (`cargo install sqlx-cli --locked`).
- Watchers: keep checkouts on ext4. Under WSL, `/mnt/<drive>` has no inotify, so polling is the fallback there (docs/research/probes.md).
- Rust style:
  - `thiserror` in libraries, `exn` in binaries
  - functional core, imperative shell
  - Sans-I/O protocols
  - newtypes
  - `#[expect]` over `#[allow]`
- Keep comments minimal. Only explain *why*.
