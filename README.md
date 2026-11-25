# Silt

A self-hostable, open-source layer under your coding agents. It records every agent session, code and conversation, so you can:

- **Resume anywhere.** Stop on your laptop and run `silt resume` on your home PC: same code, same agent session, full chat context, no WIP commit.
- **Fork in-progress work.** Pick up a teammate's half-finished feature with their whole conversation, keep going in your own agent, and send a **merge request** back. They approve it, or both lines of work carry on.
- **Keep all history.** Chat and edits survive compaction, for you and for the agent. Search across everything that's in flight.

You keep your own agent CLI: Claude Code, Codex (or its fork Qudiks), Gemini CLI, Antigravity CLI, OpenCode or GitHub Copilot CLI. Silt records underneath, and syncs through a server you host, or keeps a project on your machine alone.

> **Status: alpha.** Silt is usable by small teams that run their own server, but it has no releases yet, and its sync protocol still changes: when it does, every machine and the server need updating together (the changelog says when).
>
> **Platforms:** Linux and WSL 2 are what it's built and tested on. Windows runs natively, built from source, though no CI checks it. macOS isn't supported yet: its code is written, but hasn't been tried on a Mac. Silt's own sandbox is Linux only for now.

## Install
Until there are releases, build it with Cargo. It builds the web UI and the plugins Silt ships too, so you need:
- Rust 1.99 or newer, with the `wasm32-unknown-unknown` and `wasm32-wasip2` targets: `rustup target add wasm32-unknown-unknown wasm32-wasip2`;
- Node.js 22 or newer, with pnpm: `corepack enable pnpm`;
- git.

```sh
cargo install --locked --git https://github.com/Urcra/silt silt-cli
```

## Five minutes with Silt
In a clone of any repository, run your agent through Silt and work as usual:

```sh
silt claude          # or silt codex, silt gemini, silt agy, silt opencode, silt copilot
```

When you stop, see what was recorded, and open it in the browser:

```sh
silt log             # your threads
silt show <thread>   # its conversation, and the files each tool call changed
silt open            # the web UI
```

To carry a thread to another machine or a teammate, run a server ([Self-hosting](docs/self-hosting.md) takes about ten minutes). Then sign each machine in, and pick the thread up there:

```sh
silt auth login --server https://silt.example.com
silt resume <thread>   # your own thread, on another machine
silt fork <thread>     # someone else's, or a different direction for yours
```

The [guide](docs/guide/index.md) covers the rest.

## How it fits together
- **The binary.** One `silt` binary is the CLI, a daemon on each machine, and the server.
- **Recording.** An agent runs through `silt claude` and the like, and Silt records it through the agent's hooks and session files. A daemon makes capture faster, but recording works without one.
- **The log.** Each thread and its forks share one log of deltas: file edits on an Eg-walker-style text CRDT, the agent's session lines kept verbatim, commits, merges and comments.
- **The daemon** keeps this machine's copy in SQLite and syncs each thread with its server. It also serves the web UI and runs the agents started from it.
- **The server** keeps every family it's sent. It holds accounts, orgs, search, the inbox and a git mirror, and serves the same web UI as a hosted app for browsers and phones.

It's a Rust workspace of about forty crates: pure ones with no I/O, which also build for the browser, shell ones around them, and the daemon, the server and the `silt` binary wiring them together. The web UI is React, Vite and TypeScript, embedded in the binary. [How Silt is built](docs/architecture.md) goes through every crate, with the decision behind each part.

## Read on
- [The guide](docs/guide/index.md): handing off, forks and merge requests, history and search, your agents, the web UI, sharing, the hosted app and phones, and several servers.
- [Self-hosting](docs/self-hosting.md): from a fresh Linux machine to a team exchanging forks.
- [Command-line reference](docs/reference/cli.md).
- [Contributing](CONTRIBUTING.md): building, the gate, tests and the development loops.
- [Security](SECURITY.md): reporting a vulnerability, and the known limitations.
- [Code of conduct](CODE_OF_CONDUCT.md).
- [How Silt is built](docs/architecture.md), and the decisions behind it in [docs/adr](docs/adr/).
- [ROADMAP.md](ROADMAP.md): what's done and what's next.

## License
[AGPL-3.0](LICENSE), except the plugin SDK and the crates under it (`silt-plugin-api`, `silt-plugin`, `silt-access`, `silt-plugin-test`, `silt-plugin-host`) and the plugin template, which are MIT so plugins can be under any licence. See [NOTICE](NOTICE) for third-party attributions.
