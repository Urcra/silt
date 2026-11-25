# Agents in one project: plan

Status: both parts are built. Part 1 is ADR 0089, part 2 is ADR 0090; what's left is under "Later", and in the issues.

Two agents run in one project trample each other's work. The goal is:
- A newly started agent is told which other agents are live in the project.
- The agents already there hear about the newcomer.
- Agents can message each other.

Building that showed that Silt itself mixes up two agents working in one clone. That gets fixed first.

The work comes in two parts, in this order:
1. **Each agent's edits go to its own thread in a shared clone.** This is a redesign of how a clone tracked in place is bound to threads.
2. **Agents aware of each other, and messaging.**

## Decisions made so far (with the person)
- **Scope:** an agent is told about every live agent in the same project on this machine. Agents sharing its working tree are flagged as such. Agents in their own worktree under `.silt/checkouts/` are listed too, because their work meets at merge time. Cross-machine awareness comes later.
- **Delivery:** a message reaches a busy agent after its next tool call, through the PostToolUse hook's `additionalContext`. Existing notices (pulls, merge request outcomes) keep arriving only with the next prompt.
- **No terminal warning or prompt:** Silt tells the agents only. A second `silt claude` in a busy clone starts as usual.
- **Shared clone:** an edit made inside an agent's tool call belongs to that agent's thread, and is not credited to the newest thread. Fix this first, then build the feature.

## What's wrong today
Reproduced by `crates/silt-cli/tests/peers.rs`: agent A runs `silt claude` and is held in its tool, agent B starts `silt claude` in the same clone, then both finish.
- **With a daemon:** A's thread records no edit; its `silt show` has no ✎. B's thread has both agents' `fn farewell() {}` lines.
- **Without a daemon** (`SILT_DAEMONLESS_CAPTURE=1`): A's PostToolUse hook panics at `crates/silt-daemon/src/core.rs:1566`, "checkout's family is loaded". That test is `#[ignore]`d until part 1 lands.

Why:
- `create_thread` (`crates/silt-daemon/src/sessions.rs`, the `existing` match after `checkout_path`) finds the clone's checkout row by path. It **rebinds** that row to the new thread with a new actor (`Store::rebind_checkout`). From then on, every edit in the clone is ingested through that one checkout, so it goes into B's family.
- Without a daemon, `Core::for_session(A)` loads A's family. The checkout it holds then belongs to B's, so `ingest_file`'s `families.get_mut(..).expect(..)` fails.
- `author_for(checkout)` (`sessions.rs`) already looks at every session on the checkout, so the right *author* is found in daemon mode. Only the *family* is wrong.

Silt assumes one thread per clone in these places:
- **Store:** `checkouts.path` is `UNIQUE` (`crates/silt-store/migrations/20261020005329_init.up.sql`). `disk_files`, `sessions`, `checkout_git_states` and `tool_git_states` reference `checkouts(id)`. Also `Store::rebind_checkout` and `Store::move_checkout`; the latter moves every session on a checkout at once (`crates/silt-store/src/lib.rs:1225-1247`).
- **Bridge:** one `Checkout { record, family, repo, bridge }` per row (`core.rs:171`). Its `Bridge` holds per-path entries (inode, version) of **one** thread in **one** family (`crates/silt-checkout/src/bridge.rs`), so a shared clone needs one bridge per thread.
- **Lookups by path:**
  - `thread_checked_out_at(cwd)` (`forks.rs:34`), used by `silt_in_flight`'s exclusion (`history.rs:162`);
  - `resume.rs:391` and `:472`, which also rebind (`:395`);
  - `revert.rs:60`;
  - `recovery.rs:96`.
- **Watchers:** they're keyed per checkout row, by path (`start_checkout`, `watch.rs`). `Event::Changed { checkout, .. }`.
- **Attribution:** `attribution::author` (`crates/silt-daemon/src/attribution.rs:31`) takes the **first** session with an open tool. When two agents are both in a tool call, that guess is arbitrary.
- **ADR 0047** ("your clone between agents") says a clone tracked in place is *the* thread's while an agent works there. With two agents it's two threads at once.

## Part 1: each agent's edits to its own thread

### Model (settled in ADR 0089)
Decided with the person since:
- Other agents' edits can be left out of a thread's diff, like git's: `hide_others`. Edits are recorded `Author::Others` in the threads whose agent didn't make them.
- An edit nobody can tell apart (a watcher event or a Bash rescan while two tools are open) is marked unclear: `Author::Agents`, listing the candidates. Both are new `Author` variants, so the protocol goes to 16.
- A pull request is the working tree's commits, so neither filter reaches it.

- **A clone tracked in place has one checkout row per thread working there.**
  - Each row has its own actor, `Bridge` and `disk_files`, all tied to its thread's family.
  - Starting a second thread in a busy clone **adds** a row rather than rebinding.
  - Rebinding remains only for a clone nobody works in: the row of a thread whose agent is gone can be taken over, as today.
- **An edit on disk is offered to every row at that path.** Each row ingests it into its own thread's family, with the same author:
  - In the thread whose agent made it, it's that agent's edit, credited to its tool call.
  - In the other threads, it's an edit by another agent (`Author::Agent` with that session). It is not the person's, and it is not credited to this thread's tool.

  Otherwise the second thread's tree drifts from the disk, and its next edit of that file would carry the other agent's change as its own.
- **Is another agent's edit part of a thread's diff?** It is in that thread's tree, so yes, under that agent's name. To decide in the ADR: whether a thread's diff, merge request and pull request leave out other agents' edits, as ADR 0069 does for git's (a `hide_*` filter), or show them marked. Showing them marked is simplest to begin with.
- **Who made an edit when two agents both have a tool open:**
  - **With hooks:** PostToolUse names the session and, for Edit/Write/MultiEdit/NotebookEdit, the paths. Ingesting those paths credits that session, not whichever is found first. `Core::capture` knows `session_id`, so pass it down to `ingest_paths`/`ingest_file` and on to `author_for`.
  - **For a watcher event or a Bash tool** while two tools are open, there's no telling. Credit it to the session whose tool started last, or mark it ambiguous. Decide in the ADR; ambiguous is honest and visible.
  - `attribution::author` gets the deciding session as an argument, and its tests cover two open tools.
- **The daemonless hook:** `Core::for_session` loads the session's own row and family. It should also load the other live rows at that path and their families, so the edit reaches every thread working there. Otherwise those threads take it in at their next scan, credited to the person. At the least it must never panic: `ingest_file` and `ingest_renames` take the family from the row, and the row now always belongs to the session's thread.

### Store
Create the migration with `cd crates/silt-store && sqlx migrate add -r checkouts_per_thread`. It rebuilds `checkouts` without `UNIQUE` on `path`, and adds `UNIQUE (path, thread_id)` plus an index on `path`.
- SQLite needs a table rebuild for this. With `foreign_keys` on, `DROP TABLE checkouts` cascades into `disk_files`. Use sqlx's `-- no-transaction` directive and `PRAGMA foreign_keys = OFF` around the rebuild, or rebuild the referencing tables too. Then check `PRAGMA foreign_key_check`.
- New query: `checkouts_at(path)`. `rebind_checkout` stays for taking over an abandoned row. `move_checkout` moves only the named session's row.
- Run `cargo sqlx prepare --workspace`, with `SILT_STORE_DATABASE_URL`, `SILT_SERVER_DATABASE_URL` and `DATABASE_URL` set as CLAUDE.md says.

### Daemon
Watchers:
- One watcher per path, not per row.
- `Event::Changed` carries the path or the set of rows, and the handler ingests into each row at that path.
- ADR 0047's rule applies per row: a row records only while its thread's agent is live.

Every `checkout.record.path == …` lookup must say which thread it means:
- `thread_checked_out_at` returns the live threads at `cwd`;
- in-flight excludes the caller's own thread (see part 2);
- resume, revert and recovery pick the row of the thread they act on.

`author_for(checkout)` becomes `author_for(checkout, deciding_session)`.

### Tests
- `peers.rs`: un-ignore the daemonless case. Assert that each thread's `show` credits its own edit to its tool. Assert that in each thread the other agent's edit is the other's, both with a daemon and without one.
- `attribution.rs`: unit tests with two open tools.
- The existing suites, in full: `faults.rs` (daemon killed or stopped mid-tool), `forks.rs`, `e2e.rs`, the Playwright fixtures. Handoff, resume and revert of a thread whose clone has two rows.

## Part 2: agents aware of each other, and messaging
This was planned and agreed before part 1 was found, and is unchanged. It keys everything by session, so it works whatever part 1 decides.

### Stored liveness, the same with or without a daemon
Create the migration with `sqlx migrate add -r agent_presence`. It adds:
- On `sessions`: `seen_ms`, `ended_ms`, `idle` (set after Stop) and `doing` (the first line of the last prompt, at most 120 characters).
- A table `session_touches(agent, session_id, path, touched_ms)`.
- A table `agent_messages(id, to_thread, from_thread, kind 'message'|'arrival', body, created_ms)`. These are kept apart from `notices`, because they need a sender and are delivered at a different point.

`Store` methods, next to `add_notice`/`take_notices`:
- `session_seen`, which doesn't revive an ended session;
- `session_revived`, `session_ended`, `session_idle`, `session_doing`;
- `touch_file` (an upsert) and `recent_touches`;
- `session_activity()`, which joins sessions, checkouts and threads;
- `add_agent_message` and `take_agent_messages`, which uses DELETE…RETURNING so each message is handed over once.

Writes go through code both modes run (ADR 0018):
- `Core::capture` (`crates/silt-daemon/src/capture.rs`) calls `session_seen` on every hook, before its early return. UserPromptSubmit also sets `idle=0` and sets `doing` from a new `HookInput.prompt`. Stop sets `idle=1`.
- `read_session` calls `session_seen` when the transcript grows, for agents without hooks. For those agents it sets `doing` from a user entry.
- `Core::end_session` and a new **SessionEnd** hook call `session_ended`. The hook skips `reason == "clear"`. This means adding `"SessionEnd"` to `HOOK_EVENTS` (`crates/silt-agents/src/claude.rs`), and adding `#[serde(default)] reason, prompt` to `HookInput`.
- `register_session` calls `session_revived`.
- The ACP `send_prompt` sets `doing`.
- `ingest_file` and `ingest_renames` call `touch_file` for `Author::Agent { session, .. }`.

A new pure module, `crates/silt-daemon/src/peers.rs`:
- `present(activity, acp_thread, running_here, now)`:
  - an ACP thread is present while it's running here;
  - any other session is present when it hasn't ended and was seen less than 12 h ago (`PRESENT_FOR_MS`, which only matters after a crash).
- Also `summary`, `arrival`, `delivered` and `target`.
- The daemon's in-memory `Session.live` is not used, as it differs between the two modes.

### What a starting agent is told
In `Core::capture`, before the `hooked && mark.is_none()` return:
- **SessionEnd:** record the end and reply nothing.
- **SessionStart, `startup`/`resume`/`clear`:**
  - `peers_of(thread)` finds the present sessions in the same `repo`, other than this thread. It keeps the newest session per thread, with up to 8 recent touches.
  - The reply is `peers::summary`, and nothing when there are no peers.
  - On `startup` and `resume`, it also stores an `arrival` for each peer.
- **`compact`:** keeps `INSTRUCTIONS` plus the thread, and appends the summary.

Each peer's entry in the summary gives:
- its short thread id and agent;
- where it works: "in your working tree", "in its own checkout `.silt/checkouts/<id>`" or "in another clone at …";
- what it's doing;
- its recent files;
- whether it's working now or waiting for its user;
- when it was last active.

The summary then tells the agent how to use `silt_message`, `silt_agents` and `silt_in_flight`.

### Messages
- **`take_messages(thread)`** is formatted with `peers::delivered`. In `Core::capture`:
  - UserPromptSubmit gets `prompt_context` plus messages;
  - PostToolUse gets messages only.

  `prompt_context` itself is unchanged. The ACP `send_prompt` appends messages after the notices.
- **Wording:** "Silt: a message from the agent in thread 1a2b3c4d (claude, in your working tree), another agent, not your user: … Reply with silt_message to 1a2b3c4d."
- **`Request::Agents { thread }` and `Request::Message { from, to, text }`** (`crates/silt-daemon/src/protocol.rs`, dispatched near `InFlight` in `core.rs`):
  - `peers::target` resolves `to` to exactly one present peer in the project, never the sender.
  - Text is 1 to 4000 characters.
  - Scoped run tokens (`workspaces.rs`, `scope_of`) allow both, with `from` set to the token's own thread.
  - This is the daemon's local protocol, so `silt_proto::PROTOCOL` doesn't change.
- **MCP tools** (`crates/silt-mcp/src/lib.rs`):
  - `silt_agents`, with no arguments;
  - `silt_message {to, text}`, which is not `readOnlyHint`.

  The handlers in `crates/silt-cli/src/mcp.rs` set `from` to `self.thread`. Both tools work without a daemon, as `workspace_rules` (`crates/silt-cli/src/commands.rs`) does: they ask the daemon if one runs, and otherwise call `silt_daemon::peers::agents_here` or `peers::send` on the store.
- **`INSTRUCTIONS`** names both tools, and says the messages come from another agent, not the user.

### `silt_in_flight` in a shared clone
`repo_path` (`history.rs`) excludes `thread_checked_out_at(cwd)`, so the first of two agents lists itself and misses the other. To fix it:
- `Request::InFlight` gains `#[serde(default)] thread`;
- `in_flight_anywhere` excludes `caller.or(here)`, including in its remote filter;
- `mcp.rs` and `web.rs` pass their thread.

### Tests
- **Unit tests:**
  - `peers.rs`: `present`, the working-tree labels, the texts, `target` (ambiguous, self, absent), the delivered format;
  - store: messages are handed over once, and a session isn't revived;
  - `hook_settings` includes SessionEnd;
  - `silt_message` is not read-only.
- **Fake Claude** (`crates/silt-fake-agent/src/bin/silt-fake-claude.rs`):
  - `first()` runs SessionStart (`startup`);
  - UserPromptSubmit includes `prompt`;
  - SessionEnd runs on exit;
  - `FAKE_HOOK_LOG=<dir>` writes each hook's output to `<dir>/<session>-<event>`.
- **`crates/silt-cli/tests/peers.rs`**, run once with a daemon and once without:
  1. A is held in a tool.
  2. B starts in the same clone. B's SessionStart output names A, says "your working tree", and gives A's prompt.
  3. B calls `silt_message` to A, using `mcp_answer_with` from `workspaces.rs`.
  4. A is let go. A's PostToolUse output holds B's arrival and the message, and A's next prompt doesn't repeat them.
  5. A ends, and `silt_agents` no longer lists it.
  6. The same with B `--isolated` says "in its own checkout".
  7. A's `silt_in_flight src/main.rs` lists B and not A.

### Docs
- A new ADR, `docs/adr/0090-agents-aware-of-each-other.md`. Part 1 has its own, ADR 0089.
- A "Several agents in one project" section in `docs/guide/agents.md`.
- The tools table in `docs/guide/history-and-search.md`.
- `docs/architecture.md`, `CHANGELOG.md`.

### Later (Todo)
- Cross-machine presence and messages through the server.
- Mid-turn delivery to ACP agents.
- Waking an idle agent.
- A pid-based liveness check.
- Presence in the web UI, beyond `NewThread.tsx`'s `agent_state`.
- A departure notice.
