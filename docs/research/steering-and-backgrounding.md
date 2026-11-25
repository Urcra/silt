# Talking to an agent while its command runs

While an agent's command runs, you may want to keep talking to the agent: to
steer it, or to move the command to the background so the agent gets control
back. This note covers whether Silt's web UI can do that for Claude, Codex and
GitHub Copilot.
Researched 2026-11-20. The adapter versions and evidence are listed under each
agent.

## The short version

There are two features here, and they're worth building separately:

1. **Steering without stopping.** Today a steer cancels the turn and re-prompts
   (`acp_steer` in `crates/silt-daemon/src/acp.rs`), and that cancel ends the
   running command. Codex's and Claude's ACP adapters both have
   `_session/steering`, which adds a message to the running turn instead. Only
   Silt needs to change. This is the bigger win, so build it first.
2. **Moving a command to the background on request.** Claude's Agent SDK can do
   this, but its ACP adapter doesn't expose it yet, so it needs an upstream
   change. Copilot can do it only on its SDK protocol, not over ACP. Codex
   doesn't need it, because its commands give control back within 30 s anyway.

| | Steering without cancel | Background on request | Backgrounds on its own |
| --- | --- | --- | --- |
| Codex (`codex-acp` 2.7.0) | yes, `_session/steering` | no, and not needed | every command, after at most 30 s |
| Claude (`claude-agent-acp` 0.108.1/0.109.0) | on its ACP v1 core only; its v2 core, which Silt speaks, has none (checked in 0.109.0) | SDK has `backgroundTasks(toolUseId?)`; the adapter doesn't expose it | when the model asks, or on the Bash timeout |
| Copilot (`@github/copilot` 1.0.114, `--acp`) | no ("Method not found") | SDK protocol only (`session.tasks.promoteToBackground`) | a `sync` command after `initial_wait` (30 s by default) |

In the terminal (`silt claude` and the others), Silt doesn't stand between you
and the agent's TUI. Claude's Ctrl+B and Copilot's Ctrl+X b already work there.

## Silt today

- **Steering cancels.** `acp_steer` sends `Command::Cancel`, keeps the text in
  `run.steering`, and `next_prompt` sends it as "I interrupted your last turn to
  say: …" once the turn ends. A question pending at that moment gets the steer
  as its answer.
- **Silt doesn't run the agents' commands.** It doesn't advertise the ACP
  `terminal` capability (`v2.rs` `ClientCapabilities`), so each agent runs its
  own commands. Silt only watches `ToolCallContent::Terminal` ids to match
  output to calls.
- **Background work is already shown and stoppable.** It appears as cards
  (ADR 0051, `TaskNews` in `silt-acp/src/host.rs`). Silt ends what an agent left
  running itself, by finding the agent's child processes
  (`AgentHandle::stop_command`, `silt-acp/src/processes.rs`, ADR 0057).
- **Prompts queue** (ADR 0022, `crates/silt-daemon/src/queue.rs`).

## Codex

Sources: `agentclientprotocol/codex-acp` at `ad09e0e` (package 2.8.0; Silt's
registry pins 2.7.0), and `openai/codex` `codex-rs` at HEAD on 2026-11-20.

- **Unified exec is stable and on by default** (`features/src/lib.rs`:
  `unified_exec`, `Stage::Stable`, `default_enabled: true`).
  - Each `exec_command` returns to the model after its `yield_time_ms`, clamped
    to `MIN_YIELD_TIME_MS = 250` and `MAX_YIELD_TIME_MS = 30_000`
    (`core/src/unified_exec/mod.rs`).
  - A process still running at that point stays alive as a "background
    terminal". The app-server lists and ends those with
    `thread/backgroundTerminals/{list,terminate,clean}` (experimental, in
    `app-server-protocol/src/protocol/common.rs`).
- **There's no "yield now".** Neither the TUI nor the app-server can move a
  command to the background on request. The worst case is 30 s before the model reads a steer.
- **`_session/steering`** (`codex-acp/docs/steering.md`, `src/AcpExtensions.ts`):
  - It's advertised in the `initialize` response as
    `_meta.steering = {supported: true}`.
  - Request: `{sessionId, prompt: ContentBlock[], _meta?: {steering?: {idleBehavior?: "promptRequired"}}}`.
  - Answers: `{outcome: "injected" | "startedNewTurn" | "failed"}`, or
    `{outcome: "promptRequired", reason: "noRunningTurn"}` when opted in.
  - Use `idleBehavior: "promptRequired"`, so an idle steer comes back to Silt and
    goes through the normal `session/prompt` path, which owns usage and
    failures. Silt's queue already covers that case.
  - Steers to one session are handled in arrival order.
  - It's a pre-standard extension; agent-client-protocol PR #1261 proposes
    standardizing it.
- **Other extension methods.** `codex-acp` also has an async-task stop method
  (`ASYNC_TASK_STOP_METHOD`, handled in `CodexAcpServer.ts`). Silt's own process
  stopping already covers this.

## Claude

Sources: the npx cache's `@agentclientprotocol/claude-agent-acp` 0.108.1 and
`@anthropic-ai/claude-agent-sdk` 0.3.326. **Main now pins
`claude-agent-acp@0.109.0`, so recheck these findings against it.**

- **The SDK can background on request.** In `sdk.d.ts`, on `Query`:
  ```ts
  /** Background in-flight foreground tasks (Bash commands and subagents).
   *  With toolUseId, targets the single task started by that tool_use block;
   *  without it, backgrounds all foreground tasks — equivalent to pressing
   *  Ctrl+B in the terminal. Each blocking tool call returns immediately with a
   *  "running in the background" tool_result and the turn continues. */
  backgroundTasks(toolUseId?: string): Promise<boolean>;
  ```
  It throws when `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS` is set. `stopTask(taskId)`
  sits next to it.
- **The adapter never calls `backgroundTasks`.** Grepping 0.108.1's `dist`
  finds no call to it and no ACP method that leads to it. Its own extensions
  include `_claude/askUserQuestionOption` and `_session/steering`. It reports
  background Bash tasks (`async-tasks.js`, `backgroundedBashToolCall`). Some of
  that is gated on JetBrains AIR capabilities (`air-extension.js`:
  `asyncTasks`, `backgroundSubagents`, …), which a client opts into.
- **ACP tool call ids are Claude's tool_use ids.** `v2.rs` relies on this
  already, since Claude names terminals after their calls. So a card maps
  straight onto `backgroundTasks(toolUseId)`.
- **Steering may not reach Claude through Silt yet.** In 0.108.1,
  `_session/steering` is in `dist/v1/steering.js` and `dist/v1/acp-agent.js`
  only, and Silt talks ACP v2 to Claude. Check whether 0.109.0 serves it on v2.
  If it doesn't, either ask upstream or keep cancel-and-reprompt for Claude.
  - **Checked 2026-11-20 against 0.109.0: v2 still lacks it.** `dist/v1/steering.js`
    and `dist/v1/acp-agent.js` are its only homes. The v2 core (`dist/v2/agent.js`,
    which serves v2 "straight from Claude Code, without `ClaudeAcpAgent`") registers
    no `_session/steering` handler, and its `initialize` (`V2Agent.initialize`)
    puts no `steering` in `_meta`: only `capabilities._meta.claudeCode.promptQueueing`,
    meaning a prompt sent while Claude works waits in Claude Code's own queue. So
    Silt keeps cancel-and-reprompt for Claude until the patched adapter (below)
    serves it on v2. Silt isn't switched to v1 for this.
  - 0.109.0's v1 `steer` notes that a steer during a foreground tool call moves
    that tool (Bash, Agent, MCP) to the background rather than aborting it
    (Claude Code 2.1.286). So for Claude, steering without stopping also
    backgrounds the running command.
  - The adapter delivers a steer at priority `now`, which pre-empts the current
    generation, or `later`, when a permission or elicitation is pending.
- **An interrupt can kill background tasks.** The SDK's `perTaskStopAffordance`
  option says that without it, an interrupt kills background tasks; with it,
  Stop only aborts the turn. The adapter decides this, not Silt. Test whether a
  cancel in Silt ends a command Claude backgrounded.
- **The adapter change.** Add an extension method to `claude-agent-acp`,
  for example `_claude/background_tasks {sessionId, toolCallId?}` →
  `{backgrounded: boolean}`, that calls `query.backgroundTasks(toolCallId)`,
  and advertise it in `initialize` `_meta`. ACP v2 has no such request: its
  per-item requests are `toolCall/cancel` and `terminal/cancel`, which the
  adapter registers in `src/v2/agent.ts`. The new method's handler goes beside
  them.
- **Patch it ourselves, decided 2026-11-20.** The adapter is Apache-2.0, so a
  fork is fine as long as it keeps its LICENSE and notes what changed. Silt
  pins the adapter itself, as `@agentclientprotocol/claude-agent-acp@0.109.0`
  in `crates/silt-acp/src/registry.rs`, so switching to the fork is a one-line
  change once it's published.
  - The patch: the background method, and `_session/steering` on the v2 path if
    0.109.0 serves it only on v1 (port `src/v1/steering.ts`).
  - Silt gates both on what `initialize` advertises, so the stock adapter keeps
    working with the fallbacks.
  - Offer the same patch upstream, and go back to the stock package once it's
    merged.
  - The cost: rebasing the patch for every upstream release Silt wants
    (0.106 → 0.109 took a few weeks), and owning a package Silt's users run.
  - Publishing (npm scope, version naming such as `0.109.0-silt.1`, 2FA,
    provenance) is the owner's job; agents prepare everything up to it.
  - Until it's published, test end to end with a custom agent in
    `agents.json` that runs the patched build with `node <path>/dist/index.js`.

## Copilot

Sources: the npx cache's `@github/copilot` 1.0.114 (one 209 MB binary whose
JavaScript is compressed inside, so plain string search finds nothing useful),
recorded sessions in `~/.copilot/session-state/*/events.jsonl`,
`github/copilot-cli`'s `changelog.md`, and `github/copilot-sdk`'s
`nodejs/src/generated/rpc.ts`.

- **Shell tools.** `bash` takes `command, description, mode ("sync" | "async"),
  initial_wait, detach, shellId`, alongside `read_bash`, `write_bash` and
  `stop_bash`.
  - A `sync` command still running when `initial_wait` runs out (30 s by
    default) moves to the background, and the model is told when it finishes.
  - Main's "follow Copilot's async shells, not only detached ones" already
    tracks these.
- **The TUI can do it on request.** The changelog says "Ctrl+X b now promotes
  attached sync shell tasks to the background."
- **So can the SDK protocol** (`copilot --headless --stdio`, as the SDK starts
  it in `client.ts`). It has these methods, all `@experimental`:
  - `session.tasks.promoteToBackground {id}`;
  - `promoteCurrentToBackground`;
  - `getCurrentPromotable`;
  - `list`, `cancel` and `readOutput`.

  Shell tasks are `TaskShellInfo` (`type: "shell"`, `attachmentMode: "attached" | "detached"`).
- **`--acp` has neither.** Probing 1.0.114 directly (`initialize`,
  `session/new`, then each method with the session's id) got
  `-32601 Method not found` for:
  - `session.tasks.list`;
  - `_session.tasks.list`;
  - `session/tasks/list`;
  - `_session/tasks/list`;
  - `session.tasks.promoteCurrentToBackground`;
  - `_copilot/tasks/list`;
  - `_session/steering`.

  `initialize` doesn't advertise steering either.
- **A cancel should leave background shells running.** The changelog says "ACP
  session/prompt and session/cancel stop only the active turn, keeping
  background tasks and shell commands running." So once Copilot has backgrounded
  a command, today's cancel-style steer shouldn't end it. This hasn't been
  tested.
- **The ways forward.** File a feature request on `github/copilot-cli` to expose
  promotion and steering over ACP (it's closed source). Driving Copilot through
  the SDK protocol instead of ACP would mean a second protocol for one agent, on
  experimental methods, so it isn't recommended.

## Proposed work

### 1. Steer without stopping (all agents that advertise it)

- **`silt-acp`:**
  - read `_meta.steering.supported` from `initialize` (v1 and v2) and keep it
    on the session;
  - add a `Command::Steer(text)` sending `_session/steering` with
    `idleBehavior: "promptRequired"`;
  - report the outcome back as an `Event`.
- **`silt-daemon` `acp_steer`:**
  - when supported and the agent is working, send the steer and record it in
    the transcript as the person's message at that point;
  - on `promptRequired`, fall back to `next_prompt`'s path, the same text sent
    as a normal prompt once the turn ends, without the "I interrupted…"
    framing;
  - on `failed` or an error, fall back to today's cancel-and-reprompt;
  - keep today's behaviour for agents that don't advertise it.
- **Pending questions.** When a question is pending, today's behaviour (the
  steer answers the question) should probably stay. Decide and record it.
- **The UI.** The composer probably needs no new screen; check what it says
  while steering (`"steering"` is what `acp_steer` answers). The working
  agreement asks for mockups before any UI change.
- **The hosted app.** Steers relayed from the server (`/app/api`) go through the
  same `acp_steer`, so check them.
- **Tests.**
  - Teach `silt-fake-agent` `_session/steering`, behind a `FAKE_*` switch
    (`injected`, `promptRequired`, `failed`, not advertised).
  - Add `silt-cli` tests showing that a steer during a long tool call leaves
    the call running.
- **Paperwork.** Write an ADR, since this reverses "ACP can't add to a running
  turn". Update `CHANGELOG.md`.

### 2. "Move to background" (Claude for now)

- **The adapter patch** (above). Prepare it against the v0.109.0 tag, with the
  adapter's own tests, as a patch file the owner applies to the fork. Don't
  publish it, and don't open PRs or issues upstream.
- **Silt.** When the agent advertises the method, a running `execute` call's
  card gets a "Move to background" action, and Ctrl+B in the composer
  backgrounds every foreground task. The result then shows as the background
  task cards that already exist.
- **Mockups first:** the card action, the Ctrl+B hint, and the card once moved,
  in light, dark and phone widths.
- **Generic shape.** A per-agent capability, so Copilot can switch it on if
  GitHub exposes promotion over ACP.

### Not recommended

Taking ACP's `terminal` capability so Silt runs the agents' commands itself.
The agent stays blocked in `terminal/wait_for_exit`, which Silt could only end
by reporting an exit that didn't happen. It would also take execution out of
the sandbox (ADR 0093).

## Probing an ACP agent's methods

A probe script, run outside the repository: it starts the agent, calls
`initialize` and `session/new`, then one method, and prints the answers. It
sends no prompts.

```python
import json, subprocess, sys, threading, queue, time
proc = subprocess.Popen(sys.argv[2:], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
q = queue.Queue(); threading.Thread(target=lambda: [q.put(l) for l in proc.stdout], daemon=True).start()
n = 0
def call(method, params, wait=60):
    global n; n += 1; mine = n
    proc.stdin.write((json.dumps({"jsonrpc": "2.0", "id": mine, "method": method, "params": params}) + "\n").encode()); proc.stdin.flush()
    end = time.time() + wait
    while time.time() < end:
        try: msg = json.loads(q.get(timeout=end - time.time()))
        except Exception: break
        if msg.get("id") == mine and "method" not in msg: return msg
init = call("initialize", {"protocolVersion": 1, "clientCapabilities": {}}); print(json.dumps(init))
sid = call("session/new", {"cwd": ".", "mcpServers": []})["result"]["sessionId"]
print(json.dumps(call(sys.argv[1], {"sessionId": sid, "prompt": [], "_meta": {"steering": {"idleBehavior": "promptRequired"}}}, 15)))
proc.kill()
```

`python3 -I probe.py _session/steering copilot --acp` is an example. Signing in
isn't needed for `initialize`, but `session/new` may need it.
