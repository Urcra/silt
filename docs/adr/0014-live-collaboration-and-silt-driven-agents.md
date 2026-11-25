# 0014: Live collaboration, agents from the web UI, the native harness

- Status: accepted
- Date: 2026-10-20
- Amended by: [ADR 0017](0017-acp-agents-only.md): the native harness and the sandbox are removed; agents from the web UI are ACP agents only
- Amended by: [ADR 0081](0081-live-in-a-page-rooms-and-whiteboards.md): pages get live rooms over a WebSocket path, faster than thread presence
- Amended by: [ADR 0020](0020-next-generation-web-ui.md): The next-generation web UI
- Amended by: [ADR 0021](0021-browser-event-bus.md): The browser event bus
- Amended by: [ADR 0022](0022-agent-runtime.md): The agent runtime
- Amended by: [ADR 0026](0026-hosted-mode-and-remote-control.md): Hosted mode and remote control
- Amended by: [ADR 0050](0050-acp-v2-and-turns-the-agent-opens.md): ACP v2 where the agent speaks it, and turns the agent opens itself
- Amended by: [ADR 0056](0056-signing-an-agent-in-from-the-web-ui.md): Signing an agent in from the web UI

## Context
ADR 0006 made Silt async-first: one writer per thread, with others reading near-live, forking and offering merge requests. M8 adds the live parts:
- several people looking at and editing one thread at the same time, with presence and shared drafts;
- agents started from the web UI rather than from a terminal;
- Silt's own agent harness, a sandbox for what it runs, and a CRDT replica in the browser.

The CRDT already merges concurrent edits to a thread. What's missing is permission to write into someone else's thread, live signals, and the UI for them.

## Decision
**Collaborators.** A thread's owner can add collaborators (`silt thread share <thread> <user>`, or `--remove`).
- **Who may write a thread.** The server now enforces this on every new delta:
  - its owner and collaborators may write anything;
  - anyone who sees the thread may add comments;
  - everyone else gets `403`.
- **Leases.** Leases stay with the owner's devices: a collaborator edits the thread through the web UI rather than taking it over.

**Presence and live drafts.**
- **Heartbeats.** Every 2 seconds a viewer's daemon sends `POST /v1/presence` with:
  - the thread;
  - the file being viewed, if any, and the cursor's line in it, which lets others follow the viewer;
  - the person's draft prompt.
  The server keeps these in memory for 15 seconds and answers with everyone else on the thread who is allowed to see it. Nothing ephemeral reaches the log.
- **Sending drafts.**
  - In the web UI, "Send" submits the sender's draft together with everyone else's, each marked with its author.
  - When the owner prompts their CLI agent, the prompt hook hands the agent the teammates' current drafts.
  - A draft clears once the conversation shows it was sent.
- **The agent, for co-viewers.** The heartbeat also carries what the viewer's machine runs in the thread: the agent's name, its state and what it is saying right now. Co-viewers see it working; their drafts go along with the next prompt its owner sends.

**Live editing in the browser: a WASM replica.**
- **The replica.** `silt-wasm` compiles `silt-crdt` and `silt-log` for the browser, using wasm-bindgen.
  - **Opening a file:** the web UI loads the family's deltas and the file's base text from its daemon into the replica.
  - **Local edits** become deltas in the browser, made by a per-tab actor and authored by the user, and are posted to the daemon.
  - **Remote deltas** arrive over the thread's server-sent event stream and merge in the browser, so the editor never waits for a round trip.
- **The daemon's side.** It treats the browser's deltas like any other device's: it stores and syncs them, and writes them to the thread's checkout if there is one here.
- **Where the browser connects.** Only to its own daemon, as before.

**Agents from the web UI over ACP.**
- **The host.** `silt-acp` runs any Agent Client Protocol agent in a thread's checkout, as a task on the daemon's runtime, and talks to the daemon over channels. The SDK handles one message at a time, so prompts and permission waits run as tasks of their own.
- **Where it runs.** In the thread's checkout on this machine. If there is none, the daemon checks the thread out from a clone it knows, as `silt resume` would. "New thread with an agent" starts a thread in a managed checkout of a known clone.
- **Agents offered:**
  - the ACP registry's agents (ADR 0005), cached for a day, with a snapshot built into Silt as fallback; binary distributions are downloaded and checked against their SHA-256;
  - custom commands from `agents.json` in Silt's data directory;
  - Silt's own harness.
- **Recording.** The daemon writes the session as records of its own `acp` adapter (prompts, messages, thoughts, tool calls and results, turn ends), so history, search, forks and handoffs to CLI agents work as for any agent.
  - Streamed text is recorded once the message is complete; until then it is shown live.
  - Tool calls and results mark the turn, so the agent's edits are credited to the tool call that made them.
  - The prompt record names everyone whose draft went into it.
- **Permissions.** Permission requests wait in the UI for a person to answer.
- **Picking up again.** Resuming such a thread from the web UI uses `session/load` when the agent supports it, and otherwise a session primed with the rendered transcript (ADR 0012).

**The native harness.** `silt harness` is an ACP agent.
- **Models:** Anthropic's Messages API or any OpenAI-compatible chat completions API (OpenAI, OpenRouter, local servers), chosen by the environment.
  - `ANTHROPIC_API_KEY` (and `ANTHROPIC_BASE_URL`) picks Anthropic, by default with `claude-opus-5-5`.
  - Otherwise `OPENAI_API_KEY` with `OPENAI_BASE_URL` and `SILT_HARNESS_MODEL`.
  - The model can be changed with `SILT_HARNESS_MODEL` either way.
- **Tools:**
  - `read_file`, `write_file`, `edit_file` and `list_files`, confined to the checkout;
  - `run`, for shell commands, which only runs inside the sandbox, after the person allows it (once, or for the rest of the session). `SILT_HARNESS_UNSANDBOXED=1` runs commands without the sandbox, for systems without user namespaces.
- **Sessions** are saved in Silt's data directory, so the harness can load them again.
- **Stale edits.** An edit or write is refused when the file changed since the agent last read it, for example because a collaborator's edit merged into it. The agent then reads it again instead of overwriting work.

**The sandbox.** `silt sandbox --write <dir> [--net] -- <command>` needs no setuid helper and no extra package:
1. It enters new user and mount namespaces, plus a network namespace unless `--net` is given, mapping the user to themselves.
2. It takes detached copies of the writable directories' mounts (`open_tree`), then makes every mount read-only recursively with `mount_setattr`.
3. It mounts a private tmpfs on `/tmp` and moves the writable copies back on top (`move_mount`), which also works for directories under `/tmp`.
4. It runs the command.

The command can read the system but write only the checkout and its own `/tmp`, and has no network. Kernels without unprivileged user namespaces get a clear error, and nothing runs unsandboxed unless the user asks for that.

## Consequences
- **Permissions:** the push check is new, so threads written by people other than their owner now need the owner to share them, except for comments. This closes a gap: before, any device that could see a family could write any of its threads.
- **Build:** the browser bundle includes a WASM build of the CRDT (`wasm-bindgen` 0.2.126), which the gate builds.
- **Live updates** travel daemon → server → daemon at sync speed (long polling, about a second). That's enough for co-editing at human speed, not for typing in lockstep.
- **Linux only:** the harness's sandbox needs Linux.
