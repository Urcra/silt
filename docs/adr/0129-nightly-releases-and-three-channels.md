# 0129: Nightly releases, and three channels: stable, nightly and dev

- Status: accepted
- Date: 2026-11-26
- Amends: [ADR 0100](0100-update-channels.md) (the channels: a build of a clone is dev, and nightly is a release), [ADR 0119](0119-no-windows-ci-or-releases.md) (Windows is built in releases again)
- Builds on: [ADR 0043](0043-history-from-day-one.md), [ADR 0041](0041-the-desktop-app.md)

## Context
Nothing builds Silt's binaries from `main`. `release.yml` builds static Linux binaries from a `v*` tag, no tag has been pushed, and macOS and Windows aren't built at all. Everyone running Silt builds it from a clone, and ADR 0100 calls those builds nightly.

The repository is public now, so GitHub's macOS and Windows runners cost no paid minutes, which was ADR 0119's reason for dropping Windows from releases. A build of `main` each night would give people something to download before the first stable release. It would also show each morning whether every platform still builds.

That makes a third kind of Silt, and each kind updates differently:
- a tagged release, updated with its installer;
- a binary built from `main` by CI, which has no clone to pull;
- a build of someone's own clone, which pulls and builds again.

## Decision
**Three channels.**

| Channel | What it is | How it learns of an update | How it updates |
|---|---|---|---|
| **stable** | a `v*` release | `releases/latest` is a newer version | its installer; in place once releases are signed (ADR 0041) |
| **nightly** | the `nightly` release, built from `main` | the `nightly` release is past its commit | downloading the new nightly |
| **dev** | a build of a clone | `main`'s upstream is past its commit | `silt update`: fast-forward and `just update` (ADR 0100) |

- What ADR 0100 called nightly is **dev**. A build of a clone defaults to it, and `silt --version` says `0.0.1 (dev a1b2c3d)`.
- The release workflow names the channel with `SILT_BUILD_CHANNEL`: `stable` for a tag, `nightly` for the nightly.
- Only a dev build keeps the path of the clone it was built from. A release is built in CI's checkout, which nobody updates.
- A nightly build asks GitHub's API to compare its commit with the `nightly` tag. Settings › Updates lists the new commits and links to the release. `releases/latest` skips prereleases, so stable builds never hear of a nightly.
- The desktop window's title names the channel, as `Silt (dev)` and `Silt (nightly)`. A stable build's is plain `Silt`.

**One rolling `nightly` prerelease.**
- `release.yml` also runs on a schedule each night and on `workflow_dispatch`. It builds `main` and skips the night when the `nightly` tag already points at `main`'s commit.
- Publishing moves the `nightly` tag to the commit it built and replaces the release's assets. There's one nightly release, so the releases page and the link to it stay the same.
- Tag builds and nightlies share the workflow's jobs.

**Every platform is built, and Linux is required.**
- `silt` is built for Linux on x86_64 and aarch64 (static, with bubblewrap and pasta), macOS on aarch64 and x86_64, and Windows on x86_64. Each archive carries the licence notices.
- The desktop app is built as an AppImage, deb and rpm on Linux, a dmg for each macOS architecture, and an NSIS installer and an MSI on Windows. Each carries the `silt` built beside it.
- A nightly needs every Linux asset to publish. A macOS or Windows asset that failed to build is left out, and the release notes say which. A stable release needs every asset.
- Nothing is signed yet. The release notes say so: macOS's Gatekeeper and Windows' SmartScreen will warn. Signing and notarization stay with the release work they belong to.

**Windows is back in releases, not in CI.** This reverses ADR 0119's release half. Its CI half waits for a Windows workflow that passes; until then a nightly that fails on Windows is how a break shows.

## Consequences
- Builds of clones now say `dev` where they said `nightly`. The web UI, `silt update --check` and `silt --version` change their words; the update itself doesn't change.
- A machine on the nightly channel can't update itself in place yet. It shows what's new and links to the downloads. Installing it in place is later work.
- The nightly runs on GitHub's macOS and Windows runners each night `main` moved. That's free while the repository is public.
- A macOS or Windows nightly that builds hasn't been run on a real machine. The milestones for those platforms still need real machines.
- `silt-voice` isn't in releases yet.
