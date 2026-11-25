# Roadmap

Where Silt is going, in milestones. Each open milestone has its issues on GitHub, under [milestones](https://github.com/Urcra/silt/milestones); the decisions behind them are in [docs/adr/](docs/adr/README.md). The ADRs name milestones by their number, as in "M21".

## Direction
From [ADR 0006](docs/adr/0006-product-direction.md):
- **Async handoff:** continue your own work on another machine, with the agent session intact.
- **Forking:** fork someone else's in-progress work, code and chat both.
- **Merge requests:** bring a fork back only after approval.
- **History:** full history across compaction, plus search.

Live multiplayer comes last.

## Now
- **M18: Performance.** Recording, starting, syncing and the web UI keep up with real-sized history, held to budgets the gate checks ([ADR 0034](docs/adr/0034-performance-budgets.md)).
- **M27: The standard agent workspace.** A thread starts from the prompt box, and every screen shares one layout ([ADR 0044](docs/adr/0044-the-standard-agent-workspace.md)).
- **M29: Workspaces.** Personal and work projects on one machine, each kind of work with its own server, defaults and rules an org can set ([ADR 0065](docs/adr/0065-workspaces.md)).
- **M31: Local models.** Silt runs a local model for Claude Code and for its own agent, and a Local only workspace keeps everything on the machine ([ADR 0092](docs/adr/0092-local-models.md)).
- **M36: Agents ask other agents.** An agent on one harness and model asks an agent on another for part of its work ([ADR 0120](docs/adr/0120-agents-ask-other-agents.md)).

## Next
- **M21: Desktop app.** A Tauri 2 app around the web UI, with a tray and native notifications ([ADR 0041](docs/adr/0041-the-desktop-app.md)).
- **M22: Capture you can rely on.** An agent CLI's format change shows up in CI before anyone hits it, and never costs a session.
- **M28: Windows,** natively and in WSL, then **M25: macOS** ([ADR 0045](docs/adr/0045-windows-natively-and-in-wsl.md)). Releases come once every platform can be built.
- **M26: Claude Code cloud sessions** reach your machines like any other ([ADR 0038](docs/adr/0038-cloud-sessions.md)).
- **M32: Sandboxed agents and plugins:** what's left of Silt's sandbox, such as Landlock, Seatbelt on a real Mac and heavier backends ([ADR 0093](docs/adr/0093-silt-sandboxes-the-agents-it-starts.md)).
- **M35: Plugin marketplace** ([ADR 0105](docs/adr/0105-the-plugin-marketplace.md)).

## Done
- **M0–M4:** the CRDT, local capture, a self-hosted server with handoff, and forks with merge requests.
- **M5–M8:** history and search, more agents, accounts and orgs, and live collaboration with agents started from the web UI.
- **M9–M10:** hardening after a review of the whole code, and capture that needs no daemon.
- **M11–M17:** the web UI: an agent workspace, projects and review, a hosted app and phones, the history graph, and several servers per machine.
- **M19–M20, M23–M24:** docs for people and agents, the web UI as designed, importing your history from day one, and git and forge interop.
- **M30, M33–M34:** artifacts, themes and plugins.
