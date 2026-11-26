# Getting started

Silt keeps your coding agent's sessions, the conversation and the code, so that you can pick them up anywhere. Continue a thread on another machine, fork it to try something else, offer the fork back as a merge request, and search everything any agent ever said, before and after it compacted its context.

You keep using your own agent CLI. Claude Code, Codex, Qudiks, Gemini CLI, Antigravity CLI, OpenCode and GitHub Copilot CLI all work. Silt records underneath: every message, tool call and file edit, as a thread. Threads sync through a server you host yourself. You can also keep them on your own machine.

## Start
1. Install the `silt` binary on each machine. There's no stable release yet. The [nightly release](https://github.com/Urcra/silt/releases/tag/nightly) has `main` built each night for Linux, macOS and Windows, with the desktop app; nothing in it is signed yet. Or build it with Cargo, which works everywhere Rust does and updates the same way:

   ```sh
   cargo install --locked --git https://github.com/Urcra/silt silt-cli
   ```

   That builds the web UI and the plugins Silt ships too, so it needs Rust 1.99 or newer with the `wasm32-unknown-unknown` and `wasm32-wasip2` targets (`rustup target add wasm32-unknown-unknown wasm32-wasip2`), and Node.js 22 or newer with pnpm (`corepack enable pnpm`, or `npm install -g pnpm`). With `SILT_SKIP_WEB_BUILD=1` it builds `silt` without the web UI, and with `SILT_SKIP_PLUGIN_BUILD=1` without the shipped plugins.

   After updating, `silt daemon restart` starts the new daemon, once agents the web UI started finish their turns. Update every machine and the server together when the changelog says the protocol changed.

   A nightly build says when a newer nightly is out: the sidebar shows **Update**, and Settings › Updates lists the new commits and links to the downloads. Built from a clone of your own with `just install`, Silt is a dev build and updates itself: when `main` moves past your build, the sidebar shows **Update**, and Settings › Updates lists the new commits, fast-forwards your clone's `main`, runs `just update` there and shows each step. The daemon restarts on the new build once no agent is mid-turn, or at once with **Restart now**, and the desktop app asks for a relaunch only when its own code changed. `silt update` does the same in a terminal, and `silt update --check` only says whether an update is out. It leaves a clone alone that's off `main` or has uncommitted changes, and says so.
2. Bring in the sessions your agents already have on disk: `silt import`. Each becomes a thread kept on this machine, and `silt search "<words>"` finds them right away. See [Import your history](import.md).
3. In a clone of your repository, run your agent through Silt: `silt claude`, or `silt codex`, `silt gemini` and so on. Work as usual.
4. See what was recorded with `silt log` and `silt show <thread>`, or open the web UI with `silt open`.

To share threads between machines and people, set up a server: see [Self-hosting](../self-hosting.md). Then sign each machine in with `silt auth login --server <url>`.

## Guides
- [Import your history](import.md)
- [Hand off a thread to another machine](handoff.md)
- [Forks and merge requests](forks-and-merge-requests.md)
- [History and search](history-and-search.md)
- [Your agents](agents.md)
- [Agents asking other agents](asking-agents.md)
- [Auto for every agent](auto.md)
- [Windows and WSL](windows.md)
- [Machines over SSH](machines.md)
- [What Silt sends over the network](network.md)
- [The web UI](web-ui.md)
- [Themes](themes.md)
- [Live collaboration and agents from the web UI](live-and-agents.md)
- [Sharing and who sees what](sharing.md)
- [The hosted app, phones and notifications](hosted-app-and-phones.md)
- [Work with several servers, or keep a project on one machine](servers.md)
- [Workspaces: work and personal projects apart](workspaces.md)
- [Plugins: what sandboxed agents reach through Silt](plugins.md)
- [Writing a plugin](writing-plugins.md)
- [Silt's sandbox: what it protects, and what it doesn't](sandbox.md)
- [Local models: agents on a model this machine runs](local-models.md)
- [Record Claude Code cloud sessions](cloud.md)
- [Git notes, blame, pull requests and the Agent SDK](git-and-forges.md)
- [Self-hosting a server](../self-hosting.md)
- [Command-line reference](../reference/cli.md)

## Working on Silt
- [How Silt is built](../architecture.md)
- [Contributing](../contributing.md)
