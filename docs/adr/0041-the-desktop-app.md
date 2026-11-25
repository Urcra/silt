# 0041: The desktop app

- Status: accepted
- Date: 2026-10-22
- Amends: [ADR 0020](0020-next-generation-web-ui.md) ("native apps are not planned", for the desktop)
- Amended by: [ADR 0045](0045-windows-natively-and-in-wsl.md): Windows, natively and in WSL
- Amended by: [ADR 0108](0108-machines-over-ssh.md): Machines over SSH

## Context
The web UI is a browser tab on a port that `silt open` prints. M21 makes Silt something you open: a Tauri 2 app around the same UI, Linux first. ADR 0018 still holds: recording never depends on a window, or on the daemon.

Four things had to be decided before building it: how the app runs next to the daemon, where its crate sits, the `silt://` scheme, and updates and signing.

## Decision
**The app attaches to the daemon through `silt`, and starts it the same way.**
- It runs `silt open --no-browser`. That attaches to a running daemon, or starts `silt daemon run` in its own process group, as every `silt` command does, and prints the web UI's address. The window loads it.
- The app never links or runs the daemon. Closing the window ends the app and nothing else: the daemon, its watchers, sync and agents keep running. Starting the daemon stays in one place, `ensure_daemon`, with its waits for a busy daemon.
- `silt` is `$SILT_BIN`, else the one installed beside the app, else whichever the `PATH` finds. Packages install both.
- **The window follows the daemon.** Every 2 s the app asks the UI it shows for `/api/status` with its token. A port nobody listens on means the daemon stopped; a 401 means it started again on a fixed `SILT_WEB_PORT` with a new token. Either way the app runs `silt open` again, which starts the daemon if none runs, and loads the same page on the new address. A daemon too busy to answer within 3 s still counts as running. Failed tries back off from 500 ms to 10 s, and a first launch that failed keeps trying the same way.
  - So while the window is open, a daemon stopped with `silt daemon stop` starts again: the app keeps one running, as `silt open` would. Close the app first to keep it stopped.
- The address carries the daemon's lasting token rather than a one-time code: it reaches the app through a pipe, not a command line other users can read.
- The window stays on the daemon's origin and the app's own starting page. Other links, such as one in a transcript, open in the browser.
- **The page gets no Tauri IPC.** It shows peer content (ADR 0016), so no capability grants the daemon's origin anything. The tray and notifications read the daemon's API from the app's Rust side instead.
- A daemon started by the app inherits its environment, as one started by a hook inherits the agent's (ADR 0036). Started from a desktop menu, that can lack variables such as `CLAUDE_CONFIG_DIR` that a shell sets; a later `silt claude` from a shell warns that its agent homes differ, as it does for any daemon started elsewhere.
- One app at a time: a second launch focuses the first window (Tauri's single-instance plugin), which is also how deep links will reach it.

**The crate is its own workspace.** `crates/silt-desktop` is excluded from the root workspace and has its own `Cargo.lock` and `target/`.
- Tauri on Linux needs WebKitGTK 4.1's development libraries. As a member, every `cargo build --workspace` would need them, including on servers and in containers that never show a window.
- The gate checks it (fmt, build, clippy, tests) when `pkg-config` finds `webkit2gtk-4.1`, and says it skipped it otherwise. `scripts/cloud-setup.sh` installs the libraries.
- It doesn't depend on the other crates. It talks to `silt`, which keeps GTK out of their builds and their builds out of its.
- Tauri 2, the stable line. Tauri 3 is in alpha.

**`silt://` opens a path of the app.** `silt://open/<path>`, where `<path>` is one of the app's permalinks (ADR 0031): `silt://open/t/<thread>#turn-3` opens `/t/<thread>#turn-3` in the window.
- The app checks the path against the app's routes, and the ids in it as ADR 0016 asks, before loading it. Nothing else in a link is used.
- It is registered on install, through the desktop file's `x-scheme-handler/silt` and Tauri's deep-link plugin. A link opened while the app runs reaches it through the single-instance plugin.
- `silt open` and permalinks on a machine with the app use it.

**Updates are signed with Tauri's updater key; packages come from CI.**
- A CI workflow builds the AppImage, deb and rpm on each release tag, and signs the update bundles with Tauri's updater (minisign) key. The key lives only in CI's secrets; its public half is in `tauri.conf.json`.
- The AppImage updates itself: the app checks the release's `latest.json`, verifies its signature, and asks before installing.
- Debs and rpms are updated by the package manager. The app only says a newer version exists. Signing packages for an apt or rpm repository waits until there is one; until then the release lists their checksums.
- `silt` and the app are one release, and the packages carry both, so the app always runs the `silt` it was built with.

## Consequences
- The app needs `silt` installed. Without it, the window says so instead of the UI.
- `cargo build --workspace` doesn't build the app, and `cargo` commands for it run in `crates/silt-desktop`.
- The app adds no API of its own: whatever the UI does in a browser it does in the window, and the daemon's API stays the only one.
- Phones stay a PWA.
