# 0040: Starting and steering Claude Code cloud sessions from the web UI

- Status: proposed
- Date: 2026-10-22
- Builds on: [ADR 0038](0038-cloud-sessions.md) (recording cloud sessions), [ADR 0007](0007-agent-session-capture.md) (people keep their own agent CLI)

## Context
ADR 0038 records a cloud session as it runs, and its thread reaches the person's machines turn by turn. Starting it and talking to it still happen on claude.ai. A person should be able to do all of it from Silt's web UI, without a terminal: start a session in a project, watch it, send it follow-ups, and bring it to their machine.

What Claude Code offers (CLI 2.1.308, and its cloud docs):
- **`claude --cloud "<task>"`** creates a session for the current repository's GitHub remote and branch. It is interactive only: with `-p` it refuses, and without a terminal its UI can't start.
- **`claude -p "<message>" --cloud <id> --output-format json`** queues one message into a running session and exits with `{ok, session_id, url}` or `{ok: false, session_id, error}`. It runs on any machine signed in to the same claude.ai account. The id is `session_…`, `cse_…` or a claude.ai/code URL.
- **Scheduled tasks with an API trigger** start a session from an HTTP POST and return its id. It needs a task set up on claude.ai for each repository, and passes the task only as untrusted payload.
- **The Sessions API** that claude.ai uses isn't public. Its only sign-in is a claude.ai one, and Claude Code's terms forbid other software from collecting, storing or intermediating claude.ai credentials or session tokens. So Silt doesn't call it.
- **Inside the container,** `CLAUDE_CODE_REMOTE_SESSION_ID` holds the session's id as `cse_<suffix>`. claude.ai names the same session `session_<suffix>`.

## Decision
**The daemon drives the person's own `claude`.** Only the unmodified `claude` binary, signed in through Anthropic's own flow, ever holds the person's claude.ai credentials. Silt runs it as a program, like any agent CLI.

**The thread knows its cloud session.** `silt track --from-hook` reads `CLAUDE_CODE_REMOTE_SESSION_ID` and records it with the session, as a new conversation op, `ConvOp::CloudSession`. Every replica and the server can then map a thread to its cloud session.
- The op is checked like any peer content (ADR 0016): the id is `cse_` or `session_` and then letters and digits only, because it becomes an argument to `claude`.
- Ids are compared by their suffix.
- This changes the wire format, so it bumps `PROTOCOL`.

**Starting: "In the cloud" on a new thread.** The daemon runs `claude --cloud "<task>"` in the project's clone, in a pseudo-terminal it owns. It reads the terminal's output until a session id or claude.ai/code URL shows, then ends the process: the session runs on in the cloud. The web UI shows the launch at once, linked to claude.ai, and opens the thread when it arrives.
- **When the output changes or never names the session,** the start fails after a minute, with the screen's last lines as the error. Scraping a terminal UI is the price of using only the CLI. The id pattern is the one thing it depends on.
- **Where it starts:** the repository's GitHub remote at the clone's branch, as `claude --cloud` does. A branch that isn't pushed starts from what `claude` picks, and its warning is shown.

**Watching: the thread is the view.** The thread a cloud session records is shown like any other, with its state from its turns. It's marked as a cloud thread, with a link to claude.ai.
- A launch that hasn't recorded yet is listed with the daemon's other running agents. That happens when the repository isn't set up as in docs/guide/cloud.md, so the UI says so.

**Steering: a reply box on cloud threads.** The daemon runs `claude -p "<message>" --cloud <id> --output-format json` and returns its result. The reply arrives as the thread's next turn.
- **The hosted app** can't send by itself: the server holds no claude.ai sign-in, and mustn't. It can ask a signed-in machine of the person's to send, through remote control (ADR 0026), which comes later.

**Pulling: Resume, unchanged.** A cloud thread is a thread, so Resume in the web UI (or `silt resume`) brings its code and conversation to this machine as ADR 0006 hands off any thread. The resumed session no longer follows the cloud one. To keep both, fork.

## Consequences
- Start, watch, follow up and pull all happen in Silt's web UI, with no credentials beyond the `claude` sign-in the person already has.
- Starting depends on how `claude --cloud` draws its terminal, which may change with any release. The failure is visible, never silent, and the fix is local to one parser.
- Only a daemon on a machine with `claude` signed in can start or steer. Phones and the hosted app wait for remote control to relay it.
- A launch and its thread are joined by the cloud id, so two launches at once can't be confused.
- A session started this way records only in a repository set up for ADR 0038. Elsewhere, the launch still works, but Silt sees no more than its link.
