# 0100: Updating from the app, on a stable or a nightly channel

- Status: accepted
- Date: 2026-11-17
- Amended by: [ADR 0109](0109-installing-and-updating-silt-over-ssh.md): Installing and updating Silt over SSH
- Amended by: [ADR 0115](0115-the-sidebar-names-the-workspace-and-the-machine.md): The sidebar names the workspace at its top and the machine at its bottom
- Amended by: [ADR 0129](0129-nightly-releases-and-three-channels.md): Nightly releases, and three channels: stable, nightly and dev
- Amended by: [ADR 0132](0132-no-telemetry-and-an-offline-switch.md): update checks can be turned off in Settings › Network, and are off offline

## Context
Updating a Silt built from source means going to its clone, pulling `main` and running `just install`. Release binaries exist (ADR 0043), but most people running Silt today build it themselves, and they want the app to tell them when `main` has moved and to update without a terminal. ADR 0041 planned signed updates for release packages, which aren't built yet.

Nothing in a build said where it came from: every build was `0.0.1`, with no commit, no clone and no way to tell a release from a build of a clone. What's already there helps: the first `silt` command after an install restarts a daemon whose program was replaced, unless agents are mid-turn, and the desktop app follows a daemon that restarts, reloading the page it was on.

## Decision
**Every build names its channel.**
- `silt-update`'s build script bakes in the commit, the branch, whether tracked files had changes, the clone's path and the channel. Environment variables override each (`SILT_BUILD_*`).
- A build of a git clone is **nightly**. The release workflow sets `SILT_BUILD_CHANNEL=stable`. A build with no git, from a tarball, is stable as well.
- `silt --version` names them: `silt 0.0.1 (nightly a1b2c3d)`.

**Nightly updates the clone it was built from.**
- The daemon fetches the upstream of `main` in that clone: at start, every 6 hours, and when asked. `main` ahead of the build is an update. The UI lists the new commits, and the sidebar shows an Update button.
- No update is offered, and the UI says why, when:
  - the build or the clone is off `main`;
  - either had changes to tracked files;
  - `main` has commits its upstream lacks;
  - the clone is gone.

  Silt never switches branches or stashes someone's work.
- An update fast-forwards `main` and runs `just update`. The justfile stays the one place that says how Silt is built and installed:
  - `just update` is `install` on Linux and macOS, skipping the desktop app where WebKitGTK is missing.
  - In WSL it is `install-wsl`.

**A detached runner does the work.**
- `silt update run` is started detached (`silt_os::spawn_detached`), so it outlives the daemon restarting under it, as `install-cli` does mid-way and the runner does at the end.
- It writes its progress to `$SILT_HOME/update/state.json` and its output to `update/output.log`. The daemon reads them, so the page picks up where it was on the restarted daemon.
- It names its steps from the recipe lines `just` echoes and counts `Compiling` lines against the last run's.

**Restarting waits for agents.**
- Once installed, the runner restarts the daemon when no agent the web UI started is mid-turn.
- While agents are mid-turn, the runner asks the daemon to restart itself once none is (`RestartWhenIdle`, as `silt daemon restart` does) and waits for the new build to answer. `just install-cli` asks the same, so there is one restart; two restarts at once could each find the daemon stopping under the other.
- Until then the page says which agents it waits for. **Restart now** has the daemon stop, ending their turns, and the runner starts the new one.

**A relaunch only when the app changed.**
- The page's daemon restarts; the desktop app's window follows it, so most updates need no relaunch.
- The app tells the page its own commit. The daemon diffs `crates/silt-desktop` between that commit and the new one.
- When that differs, the page offers **Relaunch Silt**, which navigates to `silt://relaunch`. The app catches that link and restarts itself.

**Windows goes through WSL.**
- A native Windows build is made from a WSL clone by `install-windows.sh`, which passes that clone's commit, branch and `wsl://<distribution>/<path>` to Windows' cargo.
- Its daemon runs the checks and `just update` in that distribution through `wsl.exe`, in the person's login shell.
- `install-windows.sh` already renames running programs rather than overwriting them.

**Stable checks only, for now.**
- A stable build asks GitHub for the latest release and links to it. Updating it in place comes with ADR 0041's signed packages.

## Consequences
- The daemon runs `git fetch` in the person's clone every 6 hours. Only remote-tracking refs change; the branch and the working tree move only when they choose to update.
- The daemon may lack a terminal's `PATH` when the desktop app started it. The runner runs `just` in the person's login shell and fails naming the tool it couldn't find.
- An update that fails part-way leaves what it installed: say, `silt` new and the desktop app old. The page says which, and Retry runs the whole update again.
- A daemon reads the state a runner of another build wrote, so `state.json` only gains fields.
