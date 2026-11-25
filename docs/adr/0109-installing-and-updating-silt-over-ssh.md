# 0109: Installing and updating Silt over SSH

- Status: accepted
- Date: 2026-11-20
- Amends: [ADR 0108](0108-machines-over-ssh.md) (a machine without `silt`, or with another build of it, can get this computer's), [ADR 0100](0100-update-channels.md) (an update that comes from another machine)
- Revised 2026-11-26: with no releases until every platform can be built, step 2 (installing a release with `scripts/install.sh`) is gone, and the script with it. A machine of another OS or CPU is told how to build Silt there.

## Context
A machine over SSH (ADR 0108) needs `silt` installed there. When it isn't, Settings › Machines says so and stops: the person SSHes in, installs it, and comes back. When it is but runs another build, the window shows that build's UI, and nothing says it differs from this computer's, nor offers to bring it level.

The app already reaches the machine with the person's own `ssh`, and runs commands there through their login shell. So it can put a `silt` there just as well as start one.

Which `silt` to put there isn't obvious:
- `scripts/install.sh` installs a GitHub release, for Linux on x86_64 and aarch64. But a person on a nightly build, or on a fork, or with no releases reachable (a private repository), has no release matching theirs.
- This computer's own `silt` is the build whose UI the window already speaks to, and the one the person chose. When the machine has the same OS and CPU, it can run there as it is, provided the machine's C library is as new as the one it was built against.

## Decision
**The machine gets this computer's build.** Not the newest release, and not whatever `silt --version` says the latest is: the one this window runs, so both ends agree.

**How it gets there,** tried in order:
1. **Copy it** when the machine's `uname -sm` matches this computer's OS and CPU. The app sends its `silt` (the one `silt_program` finds), and `silt-bwrap` and `silt-pasta` when they're beside it, over ssh's stdin, to a temporary file beside the target. It runs `<temp> --version` there, and only if that prints this computer's version does it rename the file into place. Progress is the bytes sent.
2. **Install the release** when this computer runs a stable release, the copy can't run there, and the machine is Linux on x86_64 or aarch64: the app pipes its own `scripts/install.sh` (built into the app) to `sh` there, with `SILT_VERSION=v<version>` and `SILT_INSTALL_DIR` set to where the `silt` goes. Its last `silt install: ` line becomes the error.
3. **Otherwise explain:** what differs (the CPU, the OS, or a nightly build with no release), and the command that builds it there, `cargo install --locked --git <repository> silt-cli`.

A copy that fails its `--version` there (most often a C library older than this computer's: `GLIBC_2.39 not found`) leaves nothing behind, and the error names what's missing.

**Where it goes.**
- A fresh install goes to `~/.local/bin/silt`, as `install.sh`'s does. Finding `silt` also looks there when `command -v silt` doesn't find it, since a non-interactive login shell's `PATH` often lacks it.
- An update replaces the `silt` that was found, in place.
- A path the signed-in user can't write, such as `/usr/local/bin/silt` owned by root, is an error. Silt never uses `sudo`: `BatchMode` leaves no way to type a password, and the app shouldn't hold root on another computer.

**After it's in place,** the app runs `<silt> daemon restart --if-replaced` there. That restarts the daemon now when no agent is working, else once none is (ADR 0100), so an update never cuts an agent off. The app then connects again, since the daemon's port may have changed.

**When it's offered.**
- **Install:** when finding `silt` fails (`no silt`), on the add form's failed test and on a machine's row.
- **Update:** when a connected machine's `silt --version` differs from this computer's. Nightly versions name commits and can't be ordered, so it's "differs", not "older". The row says which build this computer has, and its options menu gains **Update Silt there**.
- Both are only buttons the person presses, never automatic, and only from this computer's page. They're one link, `silt://machines/install`, with the key, as the other machine links (ADR 0108): it installs when the probe finds no `silt` and updates the one it finds, so the page never has to guess which. `add=1` adds the machine once it connects, as a test's does. A machine's own page shows that an update is available "from This computer", without a button.
- An update that fails before it replaced anything (it doesn't run there, or the path isn't the user's) leaves a connected machine connected, on its old build, and its row says so.

**The steps are shown as a test's are,** through the same `silt-machine-test` events, which gain `action` (`test` or `install`) and `canInstall`, set when a test failed only because `silt` isn't there. An install lays out sign in, what the machine runs, copy, check, start and forward; an update lays out sign in, find, copy, check, restart and forward. The copy's text says how many megabytes went, at most every quarter second. A machine's entry gains `installing`, `canInstall` and `pendingRestart`, the agents an update's restart waits for.

**The app learns its own version** by running the `silt` it starts (found on `PATH` when it's a bare name) with `--version` once, in the background at launch, and gives it to the page as `version` on the native entry of `window.__siltBackends`.

## Consequences
- A fresh VPS of the same OS and CPU needs only SSH: add it, press Install, and it runs the same build as this computer, nightly or fork included, with no release and no repository access there.
- A machine with an older C library than this computer's can't take a dynamically linked copy. The error says so; the person builds it there, or uses a release, whose binaries are built against an older one.
- Installing puts a binary on another computer, under the person's user, through their SSH. It's no more than they could do by hand with `scp`, and it's never done without their press.
- An update changes the UI the window shows for that machine. Since it becomes this computer's build, the two agree afterwards.

## Alternatives considered
- **Always `install.sh`.** It needs a release matching this computer's, which nightly builds and private repositories don't have, and it would leave the two ends on different builds.
- **Always build there with `cargo install`.** It needs Rust, a C toolchain and time on the machine, and access to the repository.
- **`sudo` for system paths.** Needs a password prompt, which `BatchMode` rules out, and root on another computer for an app that otherwise needs none.
- **Updating automatically when the versions differ.** The machine may be running agents, or be kept on another build on purpose. The person decides.
- **`scp` or `sftp` for the copy.** The SFTP subsystem may be disabled, and a second tool means a second set of failures; `cat > file` over the same `ssh` needs nothing else.
