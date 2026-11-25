# 0050: ACP v2 where the agent speaks it, and turns the agent opens itself

- Status: accepted
- Date: 2026-10-28
- Amends: [ADR 0014](0014-live-collaboration-and-silt-driven-agents.md)

## Context
Claude Code can go on by itself after its turn has ended. When a background task it started finishes, it reads the result and answers, with no prompt from anyone. Claude's ACP adapter streams that work live. ACP v1 has no way to say that such work started or stopped, because a turn there is a prompt request and its answer. Silt took the work as more of the turn before, which led to three problems:
- The turn read as finished while the agent still worked.
- A finished turn shows its last message as its reply, so a message from the middle of the work was shown under steps that came after it.
- The agent's last message stayed unrecorded until the next prompt and landed after it, because nothing ended the turn to write it out.

In ACP v2 the agent reports its session's state: `running`, `requires_action`, and `idle` with the stop reason and what the turn cost. That covers work the agent starts by itself. A client that drives the Claude Agent SDK itself can make each such wake a run of its own, opened by the agent, with prompts sent meanwhile queued behind it.

v2 is a draft. The Rust crate keeps it behind `unstable_protocol_v2`, and Claude's adapter (0.98) serves it alongside v1. Copilot (1.0.104) answers a v2 `initialize` with v1.

## Decision
**Silt asks every agent for v2 and speaks whichever version the agent answers with.** `silt-acp` uses the crate's protocol connector. Silt's v1 `initialize` says the same as its v2 one, so an agent that answers v1 continues on the same connection and is not started again. Both versions come out as the same events. A v2 agent says so in `Ready` (`reports_work`) and sends `Working` when it works.

**Work an agent takes up by itself is a turn it opened.** The ACP record gains a `wake` line. Like a prompt, it carries the `PromptSubmitted` mark, so turns are counted, diffed, reverted and searched as before, and the wire format stays the same. It reads back as a `wake` entry, which opens a turn in the history in place of a prompt.
- **v2:** a `running` state while no prompt is out opens the turn. The daemon shows the agent as working and queues prompts sent meanwhile. The `idle` that follows ends the turn like a prompt's.
- **v1:** the agent says nothing when it starts or stops such work. Words, thoughts or a tool call arriving while no prompt is out open the turn. The agent then counts as working, so prompts queue as on v2. The turn ends once the agent has been quiet for five seconds, with no tool call running and nothing asked.

**A turn the agent opened says what set it off,** when something the agent left running ended between turns: a command in the background, or a subagent (ADR 0051). The `wake` line carries that as its `cause`.

**Stop holds until the next prompt.**
- A Stop also cancels each subagent still running, which v2 can stop by its session.
- v2 can't stop a command in the background. If such a command sets the agent going before the person prompts again, Silt cancels that work at once and says so.
- An agent on v1, as Copilot, ignores a cancel when no prompt is out. Silt stops it as Stop agent does, and the next prompt starts it again on its session.
- On v1 an agent can't say why it went on. Copilot's first step is to read the shell whose end set it off, so a wake whose first call reads a shell Silt saw it detach names that shell.

**What the agent said is recorded before a prompt or a turn's cost.**

## Consequences
- Claude's follow-up after a background task reads as its own turn, with its own reply and cost. While that turn runs, the thread shows the agent as working.
- On v1 the end of such a turn is a guess. A prompt sent meanwhile waits up to five seconds after the agent's last word. An agent that pauses longer without a tool running has its work split into two turns, still in order. Copilot goes on by itself too once a detached shell it started ends, so this applies to it.
- A thread recorded before this keeps its old order. An older Silt reading a `wake` line skips it and counts its turn by the mark.
- Silt follows the v2 draft as Claude's adapter serves it. A change to the draft means changing `silt-acp` and moving the pinned adapter version together.
- v2 sends subagents' work as sessions of their own, and Silt leaves it out. Under v1, Claude's adapter didn't send it either.
