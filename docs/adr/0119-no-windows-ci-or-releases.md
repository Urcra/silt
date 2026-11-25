# 0119: No Windows CI or Windows releases

- Status: accepted
- Date: 2026-11-24
- Amends: [ADR 0045](0045-windows-natively-and-in-wsl.md) (CI and the gate, release builds)

## Context
ADR 0045 put Windows ahead of macOS. It gave Windows its own CI workflow, `.github/workflows/windows.yml`, with clippy and the tests on GitHub's Windows runners and, behind a `windows-full` label, the WSL handoff, the desktop installer and an aarch64 build. It also added Windows jobs to `release.yml`: `silt.exe` zips for x86_64 and aarch64, which `scripts/install.ps1` installed, and the desktop app's NSIS installer and MSI.

Windows runners cost more of the account's Actions minutes than Linux runners. Since 2026-11-14 every Windows job had failed within seconds, before any step ran, so merges had no Windows check anyway. The release's Windows jobs run on the same runners.

## Decision
**No Windows workflow, and no Windows builds in releases.**
- `.github/workflows/windows.yml` is removed.
- `release.yml` builds Linux only: the `*-pc-windows-msvc` targets and the `desktop-windows` job are removed.
- `scripts/install.ps1` is removed, since no release has its zips from now on.

**Windows support itself stays.** The code, the platform modules in `silt-os` and the tests that run on Windows are kept, and Windows is checked as follows:
- the gate's clippy cross-check for `x86_64-pc-windows-gnu`, when the target and MinGW are installed;
- Wine, for `silt-os`'s tests;
- `scripts/wsl-handoff.sh`, locally with `SILT_HANDOFF_LOCAL=1`;
- `just install-wsl` on a real Windows machine, which builds `silt.exe`, the desktop app and `silt-voice` with Windows' own cargo.

People on Windows build `silt.exe` with `cargo install`, or from a WSL clone with `just install-wsl`.

## Consequences
- A change can break the Windows build or its tests without anything failing before the merge, unless whoever made it ran the gate with MinGW installed or tried it on Windows.
- Nothing registers `silt://` links or puts `silt.exe` on the `PATH` on Windows, as the installer did. People do the `PATH` part themselves.
- The Authenticode signing in `release.yml` goes with the installer job, and the `WINDOWS_CERTIFICATE` secrets are no longer used.
- Earlier releases keep their Windows assets. `install.ps1` is no longer in the repository, so nothing fetches them for you.
- Bringing either back means restoring the workflow or the release jobs from history, once the account's Windows minutes are paid for.
