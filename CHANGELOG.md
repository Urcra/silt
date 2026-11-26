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
- The sidebar's Chats section folds by its name, as a project does, showing how many chats it holds or how many are working while folded. It stays folded after a reload, in the browser or the desktop app.
- The sidebar remembers which projects you folded or opened, and whether Older projects is open, after a reload. A project you never toggled still opens when recent and folds when older.
- Auto for every agent the web UI runs (ADR 0132). An agent without an auto mode of its own, such as GitHub Copilot, Codex or OpenCode, no longer greys Auto out: Silt checks each of its commands first, with a judge. See [Auto for every agent](docs/guide/auto.md).
  - The judge is by default the thread's own agent on its model, in a session of its own and in the thread's sandbox. It's given the call and your last prompts, never the agent's own words.
  - An allowed call goes through, and goes through again without a check while the thread stays on Auto. A refused one goes back to the agent with why. With no verdict within 60 seconds the call waits for you, saying why.
  - Each call shows its verdict: **Auto allowed**, **Auto refused** with its reason, **allowed before** or **allowed by you**, and **Auto is checking** while it's checked.
  - The agent can ask you to allow a refused call with `silt_escalate`; a card shows the call, Auto's reason and the agent's, and **Allow once** lets that exact call through once.
  - **Settings › Auto** sets who checks in each workspace, from the agents it allows, and an org can set it for its workspaces. A thread can choose another under Auto in its permissions menu, as **Checked by**.
- December decorations in the desktop app: lights under each page's header, a hat on the workspace's tile, snow resting on the composer, and on the New thread screen a tree drawn as a branch graph with snow falling behind it. The desktop app's starting page lights the tree while it connects. They take their colours from your theme, which stays as it is, and keep still when you ask for less motion. On by default from 1 December to 6 January; Settings › Appearance › Seasonal touches turns them off, keeps them all year, or stops the snow.
- Yule, a bundled Christmas theme in light and dark: snow on pine and a forest at night. It shows only when you pick it.
- Nightly releases: `main` is built each night for Linux, macOS and Windows, as `silt` archives and the desktop app's packages, into the rolling [nightly release](https://github.com/Urcra/silt/releases/tag/nightly). Nothing in it is signed yet.
- Three channels: stable, nightly and dev. A build of your own clone is now a dev build, and `silt --version` says `dev` where it said `nightly`. A nightly build says when a newer nightly is out and links to it; a dev build updates its clone as before. The desktop window's title names the channel, as `Silt (dev)`, and plain `Silt` on stable.

### Fixed
- On Windows, an agent showing an image or a video Silt hadn't stored yet got "Access is denied", and each try left a temporary file among the blobs.
- An agent in a chat can show files from the chat's own folder; it was refused as part of Silt's data. An artifact's page reads the chat's files too.
- `just install`, `just update` and the gate say how to fix a corepack too old for the web UI's pnpm 12, older than 0.34.5 as Node 22.16's is, where the build failed with "Cannot find module …/bin/pnpm.cjs".
- A thread's agent started again keeps the thread's model and effort, rather than taking the ones in its own configuration: a Claude thread on Opus no longer comes back on the model Claude Code's settings file names, when another of your devices starts it, an agent you asked is asked again, or after the daemon restarted, where the composer offers the thread's model too.
- `just install` and `just update` no longer fail where corepack's default pnpm isn't the one the web UI names: corepack looked for it at the repository's root, found none, and ran its own default, which then refused the web UI.
- An agent writing many files, as an asset import does, no longer holds up the web UI: recording each new file listed every path of its thread, so 500 files in a large checkout took 20 s, now about 1 s, and pages are answered between files and while a checkout is listed, waiting at most for one file. A long thread opens for the first time in about half the time.
- Switching threads no longer waits behind a long thread being read: the daemon reads a history it doesn't keep away from everything else, keeps the 16 read most recently rather than forgetting all of them at the 17th, and logs any event that holds it for 250 ms or more. Going back to one of the last 8 threads left shows it as it is now, never as it was when left.
- A thread just opened, or scrolled up until an earlier part of it loaded, sometimes drew its turns over each other until they scrolled away.
