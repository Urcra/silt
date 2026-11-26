# Changelog

User-visible changes, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/). A change to the sync protocol says so in bold: every machine and the server then need updating together.

## [Unreleased]

Silt has no releases yet; this section collects what changes until the first one. The git history has everything before this file began.

### What Silt does so far
Protocol version 24.
- **Recording** Claude Code, Codex, Qudiks, Gemini CLI, Antigravity CLI, OpenCode and GitHub Copilot CLI sessions, run through `silt <agent>`, with or without a daemon, and importing the sessions already on disk.
- **Handoff, forks and merge requests** between machines and people, through a server you host, or kept on one machine; several servers per machine.
- **History and search** across compaction, the history graph, forking from and reverting to a turn, and commits linked to turns.
- **The web UI**, served by each machine's daemon and, as a hosted app for browsers and phones, by the server; phones on your own network without a server; the desktop app.
- **Agents from the web UI** over ACP, agents asking other agents, artifacts, themes, workspaces with their own rules, Silt's sandbox on Linux, plugins, and local models.

### Added
- Nightly releases: `main` is built each night for Linux, macOS and Windows, as `silt` archives and the desktop app's packages, into the rolling [nightly release](https://github.com/Urcra/silt/releases/tag/nightly). Nothing in it is signed yet.
- Three channels: stable, nightly and dev. A build of your own clone is now a dev build, and `silt --version` says `dev` where it said `nightly`. A nightly build says when a newer nightly is out and links to it; a dev build updates its clone as before. The desktop window's title names the channel, as `Silt (dev)`, and plain `Silt` on stable.

### Fixed
- An ask of another agent reads as its thread does: its card opens to what the asked agent said, thought and did since it was asked, where it listed only its last few tool calls. Its card and the strip above the composer show the asked agent by its mark, not its name, and the card keeps the mark while it runs rather than a spinner.
- When the agents an agent asked report back and Silt wakes it, the thread says so in a note, rather than showing "The agents you asked reported back." as a message from you.
- An agent writing many files, as an asset import does, no longer holds up the web UI: recording each new file listed every path of its thread, so 500 files in a large checkout took 20 s, now about 1 s, and pages are answered between files and while a checkout is listed, waiting at most for one file. A long thread opens for the first time in about half the time.
- Switching threads no longer waits behind a long thread being read: the daemon reads a history it doesn't keep away from everything else, keeps the 16 read most recently rather than forgetting all of them at the 17th, and logs any event that holds it for 250 ms or more. Going back to one of the last 8 threads left shows it as it is now, never as it was when left.
- A thread just opened, or scrolled up until an earlier part of it loaded, sometimes drew its turns over each other until they scrolled away.
