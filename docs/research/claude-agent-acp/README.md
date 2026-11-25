# Silt's patch to Claude's ACP adapter

`0001-steering-and-background-tasks-on-v2.patch` is a change to
[`@agentclientprotocol/claude-agent-acp`](https://github.com/agentclientprotocol/claude-agent-acp)
(Apache-2.0), made for Silt's own fork of the adapter
([research note](../steering-and-backgrounding.md), [ADR 0111](../../adr/0111-steering-without-stopping.md)).
It isn't published yet and Silt doesn't run it: Silt pins the stock
adapter in `crates/silt-acp/src/registry.rs`.

It applies to tag **`v0.109.0`**.

## What it adds to the ACP v2 core

Silt speaks ACP v2 to Claude. The stock adapter serves neither of these
methods there.

- **`_session/steering`**, with the v1 core's wire contract, advertised in
  `initialize` as the top-level `_meta.steering = {supported: true}`.
  - Request: `{sessionId, prompt: ContentBlock[], _meta?: {steering?: {idleBehavior?: "promptRequired"}}}`.
  - Answer: `{outcome: "injected"}`, `{outcome: "startedNewTurn"}`, or
    `{outcome: "promptRequired", reason: "noRunningTurn"}` when no turn runs
    and the request opted in. Errors come back as JSON-RPC errors; nothing
    answers `failed`.
  - A steer during a turn goes into Claude Code's input at priority `now`,
    or `later` while a permission request or question of the session or a
    subagent is open. A turn that a `session/cancel` is stopping counts as
    not running.
- **`_claude/background_tasks`**, which calls the Agent SDK's
  `Query.backgroundTasks(toolUseId?)`, moving the session's foreground
  tasks (Bash commands and subagents) to the background. It's advertised as the top-level
  `_meta.claudeCode.backgroundTasks = {supported: true}`.
  - Request: `{sessionId, toolCallId?}`; answer: `{backgrounded: boolean}`.
    ACP tool call ids are Claude's `tool_use` ids.
  - When Claude Code refuses (`CLAUDE_CODE_DISABLE_BACKGROUND_TASKS`), the
    error is -32603: "Claude Code did not move the work to the background: …".

The request parser moves from `src/v1/steering.ts` to a shared
`src/steering.ts`; v1's behaviour doesn't change. The patch also touches
the adapter's README and `docs/air-extensions.md`, adds tests in
`src/tests/v2/`, and regenerates the v2 scenario snapshots, whose
`initialize` line gains the new `_meta`.

One difference from v1: the v2 core holds no turn open of its own, so a
`now` steer that ends Claude Code's running cycle shows as the turn going
idle, then running again for the steered message, then idle.

## Applying it

```bash
git clone --branch v0.109.0 https://github.com/agentclientprotocol/claude-agent-acp.git
```

```bash
cd claude-agent-acp && git am ../0001-steering-and-background-tasks-on-v2.patch
```

```bash
npm ci && npm run build && npm run check && npm run test:run
```

`npm run test:run` passed in full (76 files, 4378 tests, 51 skipped) in a
clean environment. Four tests also fail on an untouched v0.109.0 when
`ANTHROPIC_BASE_URL` or other `CLAUDE_*` variables are set, so run it
without them (`env -i PATH="$PATH" HOME="$HOME" IS_SANDBOX=1 npm run test:run`
as root).

## Trying it with Silt before it's published

Add a custom agent to `$SILT_HOME/agents.json` that runs the build:

```json
[{"id": "claude-patched", "name": "Claude (patched adapter)", "command": "node", "args": ["/path/to/claude-agent-acp/dist/index.js"]}]
```

Its v2 `initialize` then says `_meta.steering.supported`, so Silt steers
it with `_session/steering` rather than cancelling its turn.

## Publishing

Publishing needs a fork, an npm scope with 2FA and provenance, and a
version such as `0.109.0-silt.1`. Then Silt's pin and `NOTICE` change,
and the patch is offered upstream. Keep the adapter's LICENSE in the
fork, and say there what changed.

## Licence

The patch is offered under the Apache License 2.0, the licence of the
adapter it applies to. Copyright 2026 Silt contributors.
