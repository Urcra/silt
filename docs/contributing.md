# Contributing

How to build Silt, check a change and work on it day to day. How the pieces fit is in [How Silt is built](architecture.md). Silt is AGPL-3.0, apart from the plugin SDK, which is MIT. Never copy code or assets from closed-source products; open-source code may be used under its licence, credited in `NOTICE`.

## What you need
- **Rust**, as `rust-toolchain.toml` pins it, with clippy, rustfmt and the `wasm32-unknown-unknown` and `wasm32-wasip2` targets. rustup installs them on the first build in the checkout.
- **Node and pnpm**, for `web/`: Node 22 or newer (`.nvmrc`), and the pnpm `web/package.json` names (`packageManager`), which `corepack enable pnpm` gives you. `web/pnpm-workspace.yaml` takes no package published less than a day ago (`minimumReleaseAge`), so a lockfile resolved anywhere also passes a pnpm that checks the lockfile against that policy.
- **git**, which Silt runs for its own writes.
- **sqlx-cli**, only to add migrations or change queries: `cargo install sqlx-cli --locked`.
- **WebKitGTK 4.1**, only for the desktop app: on Debian and Ubuntu, `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev`. Without it the gate skips `crates/silt-desktop`.
- **Network access on the first build of `crates/silt-voice`**, the speech helper, whose build script downloads the engine's prebuilt library; `SHERPA_ONNX_ARCHIVE_DIR` points it at one downloaded already. The gate builds it on Linux, unless `SILT_SKIP_VOICE=1`. Dictation in the desktop app also needs GStreamer's good plugins at run time.
- **cargo-nextest**, which the gate runs the tests with when it's installed: `cargo install --locked cargo-nextest`. Without it the gate falls back to `cargo test`, which runs the test binaries one after another and takes minutes longer.
- **cargo-deny**, which the gate checks every dependency's licence with against `deny.toml`: `cargo install --locked cargo-deny`. Without it the gate skips that check.
- **python3**, which the end-to-end fixtures use to find free ports and to stand in for Hugging Face.
- **Playwright's Chromium**, for the end-to-end tests: `pnpm -C web exec playwright install chromium`, with `--with-deps` on a bare Debian or Ubuntu. The screen references are taken with it, so another Chromium draws text differently and fails them.
- **bubblewrap and pasta** (`bubblewrap` and `passt` on Debian and Ubuntu), which Silt's sandbox runs agents in and builds others' plugins in ([ADR 0093](adr/0093-silt-sandboxes-the-agents-it-starts.md), [ADR 0104](adr/0104-one-plugin-model.md)). The tests that build a plugin from git fail without them.

### In a Claude Code cloud session
Make the environment's setup script call this repository's, so it stays current, rather than hold a copy of it:

```sh
if [ -d silt ]; then sh silt/scripts/cloud-setup.sh; fi
```

`scripts/cloud-setup.sh` installs everything above: the toolchain, cargo-nextest, cargo-deny, WebKitGTK, bubblewrap and pasta, the speech engine's library, the web dependencies and Playwright's Chromium beside the container's older one. `SILT_SETUP_SQLX=1` adds sqlx-cli. It's safe to run again, so a session that started without it, or on an older copy, runs it once from the checkouts' folder (`sh silt/scripts/cloud-setup.sh`) to catch up.

A cloud container differs from a workstation in two ways the script allows for:
- **Disk.** A session's disk is a fixed allowance, and a full gate's builds outgrow it with full debug info and incremental caches. The script sets `debug = "line-tables-only"` and `incremental = false` in the container's own `~/.cargo/config.toml`, not the repository's. If a build still fails with "No space left on device", or the linker dies with a bus error, delete `target/debug` and `target/perf`, which the gate builds again; anything else under `target/` is small.
- **Proxy.** Outbound HTTPS goes through the container's proxy. Tools that ignore it, or don't trust its certificate, fail to download: the speech engine's build script is one, so the setup script fetches its archive with curl first, into `crates/silt-voice/target/sherpa-onnx-prebuilt`, where the build looks.

A full gate from a clean container takes about half an hour, and a later one with nothing to rebuild about twenty minutes, most of it the tests and the end-to-end run. Run it in the background with its output in a file, `scripts/gate.sh > gate.log 2>&1`, and look at the `gate: [m:ss] <step>` lines to follow it. When it passes, `scripts/commit.sh` commits the tree it checked without running it again.

## Building
The web UI is embedded in the binary, so build it first:

```sh
pnpm -C web install
pnpm -C web build
cargo build --workspace
```

`target/debug/silt` is then the whole of Silt: CLI, daemon and server. `pnpm -C web build` also builds the WASM replica into `web/src/wasm`, with its JavaScript bindings from `cargo xtask wasm-bindgen`, which links the same wasm-bindgen as `silt-wasm`, so there's no CLI of a matching version to install.

The `justfile` does release builds of both the CLI and the desktop app: `just build`, or `just install` to install `silt` and `silt-desktop` into `~/.cargo/bin` with a launcher entry for the desktop app. `just build-cli`, `just install-desktop` and the like do one of them. `just install` skips the speech helper, with a note, when it can't be built, as offline or where the speech engine has no build; `just install-voice` alone fails instead. In WSL, `just install-wsl` installs the CLI and `silt-voice` there and, with Rust installed on Windows too, `silt.exe`, `silt-desktop.exe` and `silt-voice.exe` with its DLLs into `%LOCALAPPDATA%\Silt`, where the desktop installer puts them: `scripts/install-windows.sh` copies the tree to `%LOCALAPPDATA%\silt-build` and builds it with Windows' cargo. Add `%LOCALAPPDATA%\Silt` to the `PATH` on Windows yourself; nothing registers `silt://` links there now that releases have no installer (ADR 0119).

## The gate
The sandbox's bubblewrap and pasta can go beside `silt` (ADR 0093), as the release workflow puts them in its archives: `scripts/build-sandbox-tools.sh <dir>` builds them statically, from versions pinned by checksum, as `silt-bwrap` and `silt-pasta`, with their licenses. It needs meson, ninja, make and `libcap-dev`. `silt` finds them beside itself, else `SILT_BWRAP` and `SILT_PASTA`, else the system's.

`scripts/gate.sh` decides whether a change is done. It runs the cheap checks first, so most mistakes fail within a minute or two ([ADR 0091](adr/0091-a-faster-gate-and-a-pass-that-stands.md)):
1. `cargo fmt --check`, here and in `crates/silt-voice` and `crates/silt-desktop` where they're built, the web UI's formatting (`pnpm -C web fmt:check`, oxfmt), and its lint (`pnpm -C web lint`, oxlint, where a warning fails), then every Rust dependency's licence against `deny.toml` (`cargo deny check licenses`) in each workspace;
2. the web UI's build, which the binary embeds. It fails on a bundled package under a licence `web/vite.config.ts` doesn't allow, and writes `THIRD-PARTY-NOTICES.txt` beside the UI with every bundled package's licence;
3. `cargo clippy --all-targets -- -D warnings` on the whole workspace, and again for Windows (`x86_64-pc-windows-gnu`) when that target and MinGW are installed;
4. the pure crates built for `wasm32`;
5. the web UI's unit tests, then `cargo build` and the tests, with `cargo nextest` when it's installed and `cargo test` otherwise;
6. `crates/silt-desktop` when WebKitGTK is installed, and `crates/silt-voice` on Linux unless `SILT_SKIP_VOICE=1`: build, clippy and tests;
7. the performance budgets that are enforced, on an optimized build. Their limits are set for the machines Silt is developed on, with looser ones for a cloud session's and a GitHub Actions runner's, so on another slower one `SILT_BENCH_ENFORCE=0` only reports them, and the end-to-end timing too; a pass made that way isn't recorded;
8. the Playwright end-to-end tests, two spec files at a time, then the timed one alone.

Format the web UI with `pnpm -C web fmt`. A commit that only reformats goes in `.git-blame-ignore-revs`, which GitHub's blame skips; `git config blame.ignoreRevsFile .git-blame-ignore-revs` makes `git blame` skip it too.

It names each step as it starts, with the time so far, and ends with how long each took. `scripts/gate.sh --quick` runs only steps 1 to 3, the web UI's unit tests and the tests, in a few minutes, for while you iterate; it records no pass.

Commit however you like; a pull request's CI runs the whole gate, its three parts on runners of their own (`scripts/gate.sh --part`), forks' pull requests included (ADR 0130). Maintainers commit with `scripts/commit.sh "<message>"`, which commits everything (`git add -A`) and pushes, and only a tree the gate passed on. A full gate that passes records the tree it checked (`scripts/tree-id.sh`, the tree `git add -A` would commit) in `target/gate-passed`, so a commit right after `scripts/gate.sh` doesn't run it again, nor does trying again after a failed push. Otherwise `commit.sh` runs the gate itself, showing each step while the output goes to a log, and refuses to commit when the tree changed while the gate ran, as the gate didn't check those edits. A change to Markdown alone skips the gate, except the generated `docs/reference/cli.md`. Messages are Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `perf:`, `chore:`, `build:`, with an optional scope such as `feat(crdt):`), and say what changed rather than which task it was.

People with write access can have Claude work on an issue or pull request from a comment (`.github/workflows/claude.yml`). It runs in GitHub Actions, and what it writes comes from `claude[bot]`. Notes after the command steer it.
- On a pull request:
  - `/claude fix` merges the base in if it conflicts, makes the changes open reviews and comments ask for, replies and resolves them, and pushes. It never merges.
  - `/claude merge` runs the full gate on the PR merged with its base, fixes conflicts and failures (up to two rounds), waits for CI, and merges with a merge commit. It skips its own first gate when CI already passed the whole gate on the same head and base, and the PR doesn't change `.github/`. It refuses while review threads or change requests are open.
  - `/claude gate` does the same without the merge, and says whether the gate passed.
  - `/claude review` posts a review; `/claude review fix` also fixes what it found.
  - On a pull request from a fork, only `/claude review` and `/claude ask` run, and Claude's token there can read the code and post comments and reviews but not push or merge. The owner can have `/claude review dangerously full` give it the token and tools it has on members' pull requests (ADR 0130).
- On an issue:
  - `/claude fix` opens a pull request that fixes it, with screenshots of any screen it changes, or asks questions when the issue is unclear.
  - `/claude plan` comments a plan: the approach, the files, tests, risks and open questions.
  - `/claude triage` sets labels and a milestone, names likely duplicates, and asks for what the report lacks.
- On either, `/claude ask <question>`, or any question after `/claude`, answers from the code without changing anything, and `/claude cancel` stops that issue's or PR's running jobs.

Claude weighs each comment by its author: the owner first, then members, collaborators and contributors; anyone else's comments only inform it. It never decides to merge: `.github/scripts/claude.sh`, taken from the default branch, merges only a head that passed the gate and CI. The gate there holds the budgets to a runner's own limits, and counts in the status comment the budgets off their dev targets.

Windows (ADR 0045) has no CI job (ADR 0119), and only the nightly release builds it (ADR 0129), so it's checked locally: the gate's Windows clippy, Wine, `scripts/wsl-handoff.sh` between a native `silt` and a WSL distribution's, and a real Windows machine through `just install-wsl`. `SILT_HANDOFF_LOCAL=1` runs the handoff script on Linux, with both machines here. To check Windows locally, `rustup target add x86_64-pc-windows-gnu` and install MinGW (`apt-get install mingw-w64`); with Wine (`apt-get install wine64`), `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine64 cargo test --target x86_64-pc-windows-gnu -p silt-os` runs Windows tests too. Wine lacks some of Windows: symlinks and the pipe's first-instance flag behave differently there. The tests in `crates/silt-cli/tests` drive Rust stand-ins for the agent CLIs (`silt-fake-claude`, `-codex`, `-gemini`, `-copilot`, `-opencode`, in `silt-fake-agent`) and run on Windows too, except those that send Unix signals or need a Unix terminal. `cargo test -p silt-cli` alone needs `cargo build -p silt-fake-agent` first.

The first gate in a fresh clone or worktree builds everything, the `perf` profile included, so it takes a while. Later runs are incremental. The `dev` profile keeps full debug info for the workspace's crates and only line tables for dependencies, and on Linux leaves it in `.dwo` files beside the objects (`-Csplit-debuginfo=unpacked` in `.cargo/config.toml`) rather than copying it into each of the hundred-odd test binaries. gdb and lldb read it there. To step through a dependency with its variables, give it `debug = true` under a `[profile.dev.package.<crate>]` of its own.

## The tests
- **Unit tests** sit next to the code. Pure crates test their logic without I/O; the sync state machines are tested by a simulated network that drops, reorders and rolls back.
- **Integration tests** are in each crate's `tests/`. `crates/silt-cli/tests` drives the real `silt` binary on several pretend machines:
  - **Setup:** `common/mod.rs` gives each machine its own `SILT_HOME` and `CLAUDE_CONFIG_DIR`, with a real server between them.
  - **Agents:** `silt-fake-agent`'s `silt-fake-<agent>` programs stand in for each agent CLI and write that agent's own session format; `common::fake_agent` finds them.
- **`silt-fake-agent`** is a scripted ACP agent. The `FAKE_*` environment variables tell it what to say and edit.
- **End-to-end** (`pnpm -C web e2e`, needs `target/debug/silt`) runs Playwright against real daemons and servers. The screen references draw the mocks ([What the web UI mocks](ui-mocks.md)), so build the UI with `VITE_SILT_MOCKS=on pnpm -C web build` first, as the gate does. Each fixture script under `web/e2e` sets up its own temporary `SILT_HOME`s:
  - `fixture.sh`: a thread recorded with `fake-claude.sh`, a fork of it and a merge request;
  - `live-fixture.sh`: a server and two people;
  - `workspaces-fixture.sh`: two servers;
  - `screens-fixture.sh`: the screen references' world, `live-fixture.sh`'s with a fork and a merge request;
  - `busy-fixture.sh`: one machine with one thread with a lot in it;
  - `local-fixture.sh`: a Local only workspace with a local model, and `fake-hub.py` standing in for Hugging Face.
- **CRDT traces.** `scripts/fetch-traces.sh` downloads real editing traces into `target/editing-traces`. Tests that use them skip until they're fetched.
- **Self-hosting.** `scripts/selfhost-walkthrough.sh` follows [Self-hosting](self-hosting.md) word for word on fresh containers.
- **Releases** (ADR 0129). `.github/workflows/release.yml` builds `silt` for Linux (static, x86_64 and aarch64), macOS (aarch64 and x86_64) and Windows (x86_64), and the desktop app as an AppImage, deb and rpm, a dmg for each Mac, and an NSIS installer and MSI. Each night `main` moved, and when run by hand from the Actions tab, it publishes them to the rolling `nightly` prerelease with `SHA256SUMS`, moving the `nightly` tag to the commit it built; a macOS or Windows asset that failed is left out, and the notes say so. Pushing a tag `v*` publishes a stable release, which needs every asset. Builds are on the `nightly` or `stable` channel (`SILT_BUILD_CHANNEL`); a build of a clone is `dev`. Each archive carries `LICENSE`, `NOTICE`, the web UI's notices and `THIRD-PARTY-NOTICES-rust.txt`, which `cargo about generate` writes from `about.toml` and `about.hbs`. Nothing is signed yet.

## Working on it
- **The web UI.** `pnpm -C web dev:daemon` runs Vite with `/api` proxied to the running daemon, and prints the URL to open, so changes to `web/` show without rebuilding `silt`. `SILT` names the binary to ask, and `PORT` the dev server's port.
- **A scratch data dir.** Point `SILT_HOME` at an empty directory to try something without touching your own threads, with a `CLAUDE_CONFIG_DIR` of its own if an agent runs:

  ```sh
  export SILT_HOME=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d)
  target/debug/silt claude
  target/debug/silt log
  ```

  Each data dir gets its own daemon. Stop it with `silt daemon stop` before removing the directory. Run a server the same way, with `silt server init --data <dir>` and `silt server run --data <dir>`.
- **Dogfooding.** Install the worktree's build with `just install-cli` (or `just install` for the desktop app too). Both restart the daemon on the new build (`silt daemon restart --if-replaced`), as does the first `silt` command after an install and closing the desktop app. If agents the web UI started are in the middle of a turn, the daemon restarts once they finish; `silt daemon stop` restarts it at once, ending them. Sessions run with `silt claude` and the like carry on through the restart, recorded by their wrappers meanwhile. Only the program the daemon runs does this, so running `target/debug/silt` beside an installed one never swaps it. `packaging/systemd/silt-daemon.service` keeps a daemon running while you're logged in. A build of a clean `main` updates itself from Settings › Updates or `silt update`, through `just update` (ADR 0100); a build of another branch or with local changes has updates off. To try the updater on a debug build, point it at a clone and an older commit: `SILT_BUILD_SOURCE=<clone> SILT_BUILD_COMMIT=<commit> SILT_BUILD_BRANCH=main target/debug/silt update --check`.
- **The desktop app.** `crates/silt-desktop` is its own workspace, so run `cargo` there: `cargo run` opens the window on the daemon of `SILT_HOME`, starting it if none runs. `SILT_BIN` names the `silt` to run, such as `../../target/debug/silt`; otherwise it's the one beside the app, then the `PATH`'s.
- **Logs.** `SILT_LOG` sets the tracing filter, for example `SILT_LOG=silt_daemon=debug`. The daemon writes to `daemon.log` in its data dir.
- **Watchers.** Keep checkouts on ext4. Under WSL, `/mnt/<drive>` has no inotify, and Silt falls back to polling there.
- **Mockups before UI work.** A feature that touches the UI starts with artifact mockups of it finished, shared before its code is written. Make one for each new or changed screen and state (empty, loading, error and full), in light and dark, and at phone width where the screen works there. Build them from Silt as it actually looks, not from the Silt or Nous design system alone: use the app's own CSS (`web/src/nous.css`, `web/src/index.css`) and the markup its components render, checked against the running app on a `web/e2e/*fixture.sh` fixture. Put the new parts in the real screen around them (sidebar, breadcrumbs, panels and realistic data), so the mockup shows exactly how the feature will look.
- **Pull requests** follow `.github/pull_request_template.md`. One that changes the UI shows each new or changed screen, pasted into its description.

## Generated files
Each one has a test that fails while it's out of date, and the failure says how to write it again.

| File | Comes from | Write it again with |
|---|---|---|
| `web/src/api.generated.ts` | The daemon's and silt-proto's API types, by ts-rs | `UPDATE_API_TYPES=1 cargo test -p silt-daemon api_types` |
| `docs/reference/theme.schema.json` | The theme format's JSON Schema, from silt-theme's tokens and tags | `UPDATE_THEME_SCHEMA=1 cargo test -p silt-theme theme_schema` |
| `docs/reference/cli.md` | `silt`'s clap commands, by `silt reference` | `UPDATE_CLI_REFERENCE=1 cargo test -p silt-cli --test reference` |
| `.sqlx/` | The `query!` macros, checked against the schema | `cargo sqlx prepare --workspace`, see below |

Built rather than kept:
- `web/src/wasm/` comes from `pnpm -C web wasm`.
- `web/dist/` comes from `pnpm -C web build`. When it's missing, `silt-web`'s build script runs that build itself, as `cargo install` needs; `SILT_SKIP_WEB_BUILD=1` builds without the UI.
- `target/site/` comes from `cargo run -p silt-site`.
- The plugins shipped with Silt (`plugins/`, for `wasm32-wasip2`) are built by `silt-shipped`'s build script, in its own target folder, and embedded in `silt`; `SILT_SKIP_PLUGIN_BUILD=1` builds Silt without them.

## The database and sqlx
`silt-store` (each machine's store) and `silt-server` own their schemas, each in its own `migrations/`.

**Adding a migration.**
- Make it with `sqlx migrate add -r <name>` and apply it with `sqlx migrate run`, from the crate's directory, so its `sqlx.toml` applies. Never write a migration file by hand.
- Each crate reads its own variable, set to an absolute URL: `SILT_STORE_DATABASE_URL` or `SILT_SERVER_DATABASE_URL`. A relative `sqlite://../..` path fails to open:

  ```sh
  export SILT_STORE_DATABASE_URL="sqlite://$(git rev-parse --show-toplevel)/target/dev-silt-store.db?mode=rwc"
  export SILT_SERVER_DATABASE_URL="sqlite://$(git rev-parse --show-toplevel)/target/dev-silt-server.db?mode=rwc"
  ```

**After changing a `query!`.** Run `cargo sqlx prepare --workspace -- --all-targets` from the root (without `--all-targets`, the queries only tests make are dropped from the cache), with both variables set, and `DATABASE_URL` too, which the CLI checks. Commit the `.sqlx/` it writes. Builds use that cache when the variables aren't set.

## Performance budgets
`silt-bench` measures Silt against the budgets of [ADR 0034](adr/0034-performance-budgets.md), on fixtures as big as real use: a 100 MB transcript, a 50,000-file repository and a family of ten forks. Build it optimized with `cargo build --profile perf -p silt-cli -p silt-bench`.
- `target/perf/silt-bench report` measures every budget, and never fails.
- `target/perf/silt-bench check` measures only the enforced ones, and fails naming the budget and what it measured. The gate runs it.
- The limits are the development machine's, a cloud session's ([ADR 0037](adr/0037-cloud-machine-budgets.md)) or a GitHub Actions runner's ([ADR 0126](adr/0126-budgets-for-a-github-actions-runner.md)), from `SILT_BENCH_MACHINE=dev|cloud|ci`, or else from where it runs. The dev limits stay the targets: elsewhere, a budget over its dev limit prints an `off target:` line without failing, which on a runner is a warning on the job, and the job's summary gets the table.
- `--only <budget>` measures just that budget.
- `--tolerance <percent>` passes a budget over its limit by at most that much, printing a `slower than budget:` line for it. The gate takes it from `SILT_BENCH_TOLERANCE`. `scripts/commit.sh` sets it to 50 and repeats those lines with a warning, so a busy machine doesn't block a commit. When you see that warning, check whether the slowdown is expected ([ADR 0052](adr/0052-commits-tolerate-a-slower-machine.md)).

The fixtures are cached in `target/silt-bench`.

## The docs
`cargo run -p silt-site` builds this site into `target/site`: the landing page, the guide, self-hosting, the reference and these pages, as static HTML. It fails when a page links to a doc that isn't there.

## Rust style
- `thiserror` in libraries, `exn` in binaries;
- a functional core with an imperative shell, and Sans-I/O protocols;
- newtypes;
- `#[expect]` over `#[allow]`;
- comments only where the code can't say why.

The pure crates must stay free of I/O.

## Where the work is
- **ROADMAP.md** has the direction and the milestones, now and next.
- **GitHub issues** have the work itself, each in its milestone, labelled by kind and area; `blocked` says it waits on something, `good first issue` that it's a good place to start.
- **`docs/adr/`** has the decisions. A significant decision, or reversing one, gets a new ADR.
- **CHANGELOG.md** lists the user-visible changes.

Update CHANGELOG.md when behaviour changes.
