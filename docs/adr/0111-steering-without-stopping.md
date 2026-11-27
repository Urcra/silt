# 0111: Steering without stopping

- Status: accepted
- Date: 2026-11-20
- Reverses part of: [0022](0022-agent-runtime.md) ("ACP can't add to a running turn")
- Amended by: [ADR 0137](0137-enter-steers-mid-turn.md): ⏎ steers mid-turn, Claude is steered with a prompt, and a refused steer waits in the queue

## Context
ADR 0022 had a steer cancel the agent's turn (`session/cancel`) and send the steer as the next prompt, framed as "I interrupted your last turn to say: …". The cancel also ends the command the agent is running, so steering an agent in the middle of a build or a test run threw that run away.

ACP now has a pre-standard extension for adding to a running turn, `_session/steering` (agent-client-protocol PR #1261 proposes standardizing it):
- An agent advertises it in its `initialize` response as `_meta.steering = {supported: true}`.
- The request is `{sessionId, prompt: ContentBlock[], _meta?: {steering?: {idleBehavior?: "promptRequired"}}}`.
- The answer's `outcome` is `injected`, `startedNewTurn`, `failed`, or, for a client that opts in with `idleBehavior: "promptRequired"`, `promptRequired` when no turn is running.

Who serves it (docs/research/steering-and-backgrounding.md):
- **Codex's adapter** (`codex-acp` 2.7.0) serves it.
- **Claude's adapter** (`claude-agent-acp` 0.109.0) serves it on its ACP v1 core only. Its v2 core, which Silt speaks to Claude, neither advertises nor answers it. Claude keeps the fallback until the adapter serves it on v2.
- **GitHub Copilot** (`--acp`, 1.0.114) answers it with "Method not found" and doesn't advertise it.

## Decision
**Silt steers with `_session/steering` where the agent advertises it.** `silt-acp` reads `_meta.steering.supported` from `initialize`, over v1 and over v2, and says so in `Event::Ready`'s `steers`. `Command::Steer(text)` sends `_session/steering` with `idleBehavior: "promptRequired"`, and `Event::Steered` reports the outcome: `injected`, `startedNewTurn`, `promptRequired`, `failed`, or a JSON-RPC error.

**The daemon acts on each outcome:**
- `injected`: the steer is in the turn. It's recorded as the person's message at that point (after what the agent said so far), and the turn goes on. `startedNewTurn` is recorded the same way, as the agent took the text.
- `promptRequired`: the turn ended before the steer reached it. The same text is sent as a normal prompt once the agent is idle, with no "I interrupted" framing, ahead of the queue.
- `failed` or an error: the turn is cancelled and the steer sent once it ends, as for an agent that doesn't advertise steering.

**Agents that don't advertise it keep ADR 0022's cancel and re-prompt.** So does every agent if its adapter stops advertising it.

**A pending question is still answered by the steer, without a cancel.** Today a steer while the agent asks a question answers it "in your own words". That stays: the steer is sent framed as `to your question "…": <steer>`, the question is declined, and its card records the steer as the answer. The steer goes first, so an adapter that holds a steer until its question is settled (Claude's does) has it as the agent goes on. A pending permission request is left for the person to answer; the agent hears the steer when it can.

**Steers from the hosted app go through the same path.** `/app/api/.../agent/steer` relays `RemoteCommand::Steer`, which the machine running the agent turns into the same `acp_steer`. The wire format doesn't change, so `PROTOCOL` stays.

**The web UI doesn't change.** The steer endpoint answers `steering` as before, and the composer shows the steer as sent.

## Consequences
- A steer no longer ends the command an agent runs, for Codex today, and for Claude once its adapter serves `_session/steering` on v2 (the patch prepared under `docs/research/claude-agent-acp/` adds it).
- The history of a steered turn reads prompt, work, steer, more work, one end, rather than a cancelled turn followed by a new prompt.
- Steer now's tooltip still says it interrupts the turn. Rewording it to suit both kinds of agent is a UI change, left for its own mockups.
- A steer an adapter holds until a permission is answered isn't shown as waiting. If that confuses people, a later change can show it.
