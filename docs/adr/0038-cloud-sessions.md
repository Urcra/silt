# 0038: Recording Claude Code cloud sessions

- Status: proposed
- Date: 2026-10-22
- Amends: [ADR 0007](0007-agent-session-capture.md) (Starting)

## Context
Claude Code on the web runs each session in a container. Something other than the user starts `claude` there, so `silt claude` can't, and the container is thrown away when the session goes idle. Such "cloud runners" had been left for later.

Tried in one of those containers, everything Silt needs was already there:
- The session is written where Silt reads it, `~/.claude/projects/<slug(cwd)>/<id>.jsonl`.
- `CLAUDE_CODE_REMOTE=true` marks the session as a cloud one.
- Hooks in the repository's `.claude/settings.json` run there, and the environment's setup script runs before them, with its variables.
- Outbound HTTPS goes through a proxy that reqwest uses from `HTTPS_PROXY`, with its CA in the system store. The server's host has to be allowed by the environment's network policy.
- A signed-in container records the session and pushes it. A second machine finds the session with `silt search` and continues it with `silt resume`.

What was missing is a way for a hook to start the recording. `silt track` attaches to the most recent session in the current directory, and with `SessionStart` input, a hook knows exactly which session it is.

## Decision
**Repository hooks start the recording.**
- `silt track --from-hook` reads a `SessionStart` hook's input on stdin, records the session it names in that session's cwd, then captures the hook as `silt hook` would. Like `silt hook`, it always exits 0, and writes only the hook's reply to stdout.
- `silt track --session <id>` does the same from a shell.
- Recording a session that is already recorded returns its thread. `SessionStart` fires again on resume, `/clear` and compaction.

**Hooks only in the cloud.** `packaging/claude-cloud/settings.json` holds the hooks a repository adds. Each runs only when `CLAUDE_CODE_REMOTE` is `true` and `silt` is on the `PATH`. On a person's own machine `silt claude` adds its own hooks through `--settings`, and two of each would capture twice.

**The environment signs in.** Its setup script installs `silt` and runs `silt auth login --password-stdin` with the person's own account, from the environment's variables. A thread belongs to the account that recorded it, so recording as the person is what lets them resume it. The container is one more device of theirs.

**Nothing waits for the push.** Each `Stop` starts `silt sync --once` and the daemon pushes as it runs, as on any machine. A container reclaimed before a push finishes loses the end of that turn, along with the session itself.

## Consequences
- A cloud session's thread reaches the person's machines turn by turn, which is the handoff ADR 0006 is about.
- The password sits in the environment's settings, readable by anyone who can edit the environment. Device tokens that can be minted and revoked for one environment would be better.
- A cloud session resumed in a new container records into a new thread, since the container's store starts empty. Finding the thread by session id on the server would fix this.
- Handing a thread to a cloud session isn't possible: Claude Code chooses its session id there, so a restored session can't be launched in its place.
- Installing means building from source until release binaries exist (M23).
