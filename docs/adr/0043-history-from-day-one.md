# 0043: Your history from day one: release binaries, importing old sessions, sending them

- Status: accepted
- Date: 2026-10-22
- Amends: [ADR 0032](0032-several-servers-and-local-projects.md) (Moving a project to a server)

## Context
Trying Silt means building it from source, and it then starts with an empty history: it records only the sessions started after it. Yet the sessions people already ran are on disk, in each agent's own files, which are the same files Silt reads when it records (ADR 0007). The search across all of them is most of what a new user wants on day one.

Two things stand in the way. Nothing in `silt-agents` lists every session on disk: `discover` finds the newest one in one directory. And `silt track`, the one way to record a session that already ran, makes a live thread. It imports the clone's whole tree as the family's files, and takes over the clone's checkout. For one session that's right. For hundreds of finished ones it would cost a full tree per session, and it would take each checkout away from the thread working in it.

## Decision
**Release binaries.**
- A tag `v*` runs `.github/workflows/release.yml`. It builds the web UI, then `silt` for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, each on a runner of its own architecture. It publishes `silt-<target>.tar.gz` and `SHA256SUMS` on the tag's GitHub release.
- Static musl binaries run in any distribution or container, with no glibc version to match.
- `scripts/install.sh`, run as `curl -fsSL https://raw.githubusercontent.com/Urcra/silt/main/scripts/install.sh | sh`, picks the archive for the machine's architecture and checks it against `SHA256SUMS`. It installs `silt` into `~/.local/bin`, or `$SILT_INSTALL_DIR`. `SILT_VERSION` picks a release; the default is the latest. `SILT_DOWNLOAD_URL` replaces the release's URL, so a test can install from its own server.

**Or with Cargo.** `cargo install --locked --git https://github.com/Urcra/silt silt-cli` installs and updates `silt` wherever Rust runs.
- The UI is embedded from `web/dist`, which a checkout lacks. A new crate, `silt-web`, holds the embedded UI for the daemon and the server. Its `build.rs` builds `web/dist` when it's missing: `pnpm install` and `pnpm build`. Its own wasm build goes to a target dir under `OUT_DIR`, since the cargo running the build script holds the outer one.
- A checkout that has `web/dist`, as the gate's and the release workflow's do, builds as before.
- Without pnpm, wasm-bindgen 0.2.126 or the wasm32 target, the build fails and names what's missing. `SILT_SKIP_WEB_BUILD=1` builds `silt` without the UI. The cloud setup script falls back to that, since recording needs no UI.

**Listing the sessions on disk.** `Agent::on_disk` lists every session the agent has on disk, each with its id, its transcript, and the directory it ran in and when it started when the files say.
- **Claude Code:** `projects/*/*.jsonl`; the `cwd` and `timestamp` of its first records.
- **Codex:** every rollout's `session_meta`.
- **Gemini CLI:** each project's chats, with the directory from the project registry or `.project_root`.
- **Copilot:** each `session-state/*/events.jsonl`, with `session.start`'s cwd.
- **OpenCode:** keeps its sessions in a database read through its CLI. `opencode session list` answers for the directory it runs in. It's asked in each directory Silt knows: the clones it has seen, the directories the other agents' sessions ran in, and the ones named on the command line.

**`silt import [<path>…] [--agent <agent>]`** imports every session on disk of every agent, or those that ran under the given paths.
- **Importing twice adds nothing.** A session already recorded (`sessions`) or imported (the new `imports` table, by agent and session id) is skipped. A session still being written is imported as it is now, and importing it again later adds nothing. Recording continues only for sessions Silt runs or tracks.
- **One thread per session,** in a family of its own. Its project is the normalized origin of the clone the session ran in. A session whose directory is gone or isn't in a git clone gets a thread with no project: readable and searchable, but resume and fork say they need the clone it came from.
- **Code: the conversation only.** The family starts from a genesis at the commit HEAD was at when the session started: from the reflog, else the newest commit on HEAD before then, else HEAD. Nothing else of the tree is imported, and the edits the session made aren't reconstructed: its tool calls show them, and no diff does.
- **The tree comes when it's needed.** Resuming or forking a thread whose history has a genesis but never created a file first imports the genesis commit's tree onto the thread being checked out. This needs no new op: it is the same import a new thread starts with, done later.
- **The original times.** Each line's delta carries the time its record has (`timestamp` where the agent writes one), else the time of the record before it, else the session's start. The thread's creation time is the session's start. Turn marks and subagent transcripts are read as when recording.
- **No checkout, no watcher.** An imported session never becomes one the daemon tails. The import reads each transcript once, in batches like a recorded backlog's, and nothing about the clone changes.

**Kept here until sent.**
- An imported family's home is this machine (ADR 0032), whatever the project's home or the sign-ins.
- Unlike other families kept here, it stays when the project's home is set to a server: `silt project home` moves only the families that were made on this machine as new threads.
- `silt send <thread>…`, `silt send --project <project>` or `silt send --all` sends imported threads to `--server <url>`, or to the default server. It sets their families' home there, and the push takes it from there.
- What's pushed is redacted as every push is (ADR 0015, ADR 0019): secret-looking variables and the secrets file at the time of sending, and the well-known token formats. Old sessions can hold secrets that are gone from today's environment. `silt send` says how many were redacted, and the guide says to add old secrets to the secrets file before sending.

## Consequences
- Search, `silt show`, the web UI and `silt_history_read` cover the history from before Silt, with no new view: an imported thread is a thread.
- An imported thread costs its transcript and nothing per file until it's continued.
- If someone forks an imported thread before its owner ever continues it, the fork gets its own copy of the tree. A merge request from that fork back into the thread, once the owner has continued it too, then meets two copies of every file. That takes a fork of a thread sent unopened, then both sides continuing it.
- `silt track` of a session that was imported records it again in a new thread. Taking an imported thread over for recording is left for later.
- OpenCode sessions in directories Silt has never seen need the directory named: `silt import ~/src/that-project`.
- The release workflow is the first CI in the repository. The gate still runs locally. Only tags run CI.
- `scripts/install-walkthrough.sh` checks the guide on a fresh container: install from a release built locally, import, search.
- `import.session` joins the budgets (ADR 0034): importing a 100 MB transcript takes under a second, as tracking one does.
