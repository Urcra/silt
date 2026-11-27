# 0137: ⏎ steers mid-turn

- Status: accepted
- Date: 2026-11-27
- Amends: [ADR 0111](0111-steering-without-stopping.md) (what a refused steer does, and how Claude is steered), [ADR 0062](0062-the-composer-plus-and-enter.md) (what ⏎ does while a turn runs)
- Builds on: [ADR 0022](0022-agent-runtime.md) (the queue)

## Context
While a turn runs, ⏎ queues a prompt until the whole turn ends, and Steer now (mod+⏎) either adds it to the turn, for an agent that serves `_session/steering`, or cancels the turn. A turn can run for many minutes, so a correction typed early waited for all of it, or stopped it.

The agents' own interfaces deliver a message typed mid-turn at the next safe point instead: after the running tool call returns, before the next model request, without interrupting. Claude Code, Codex, Copilot CLI, OpenCode, Cursor and Amp all do this on Enter, and keep queueing on a modifier (docs/research/steering-and-backgrounding.md).

Claude's adapter (`claude-agent-acp` 0.109.0, ACP v2) serves no `_session/steering`, which is why ADR 0111 left Claude cancelling. But it advertises `capabilities._meta.claudeCode.promptQueueing`: a `session/prompt` sent while Claude works goes into Claude Code's queue with no priority, which Claude Code takes as `next`. That is the next step, the same point as Enter in its terminal. Checked against Claude Code 2.1.332: the running command ran to its end, and the message joined the same turn. The adapter answers the prompt when Claude Code takes it in, and sends its `user_message` just before, in order with the agent's output.

## Decision
**⏎ steers an agent that takes a prompt mid-turn; mod+⏎ queues.** The composer's button beside + reads **Queue** (mod+⏎) instead of Steer now, and ⏎'s hint says "steer". For an agent that doesn't, nothing changes: ⏎ queues and Steer now cancels the turn to say it.

**Claude is steered with a prompt.** `silt-acp` counts `claudeCode.promptQueueing` on a v2 agent's capabilities as taking steers. A steer there is a `session/prompt`, taken in when its `user_message` arrives, or when the prompt is answered, whichever comes first. The echo is in order with what the agent says, so the steer is recorded before the agent's answer to it. Agents that serve `_session/steering` keep using it.

**A steer shows where it will land until the agent takes it in.** The daemon lists the steers on their way in the agent's status (`waiting_steers`), and the thread shows each as a dashed bubble after the turn so far, with **Send now**. Once taken in, it's a message in the turn like any other.

**Send now stops the turn, and the steer goes as it ends.** Stopping a turn while a steer waits, with Send now, Esc or the stop button, sends the steer once the turn ends, ahead of the queue, rather than dropping it. Claude Code drops a queued prompt with the turn it cancels, so it isn't heard twice. If the agent took it in anyway, it's recorded there and not sent again.

**A refused steer waits in the queue.** `failed`, or an error answering a steer, puts it at the front of the queue for when the turn ends, rather than cancelling the turn as ADR 0111 did. The turn and the command it runs go on.

## Consequences
- Claude no longer needs the patched adapter in `docs/research/claude-agent-acp/` to be steered without stopping. The patch's `_claude/background_tasks` is still the way to move a command to the background on request.
- Adapter 0.113.0 sends a prompt that arrives while Claude works at priority `now`, which ends Claude Code's current cycle early, though not the running tool. Before Silt pins it, check how such a steer is recorded (issue #167).
- An agent on another of the person's devices shows today's queue: this machine doesn't know whether that agent steers.
- Antigravity's `agy acp` (1.5.0 and later) steers a running turn when sent another prompt, by its changelog, and Gemini CLI and Copilot can be reached mid-turn through hooks. Those are follow-ups, each needing a check on a signed-in machine.
