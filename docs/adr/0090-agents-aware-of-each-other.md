# 0090: Agents in one project know about each other

- Status: accepted
- Date: 2026-11-17
- Builds on: [ADR 0089](0089-a-checkout-per-thread-in-a-shared-clone.md) (each agent's edits in its own thread), [ADR 0018](0018-daemon-optional-capture.md) (capture without a daemon)

## Context
People run more than one agent in a project: two `silt claude` in one clone, or one there and one in its own checkout. Each agent works as if alone. It rewrites a file the other is halfway through, or redoes what the other just did. The person finds out at merge time, or not at all.

Silt already records each agent's session, its thread, where it works and the files it edits. Nothing told one agent about another.

## Decision
**Silt keeps whether each session's agent is still around, the same with or without a daemon.** The store's `sessions` gain:
- `seen_ms`: set by every hook, and by a transcript growing for an agent without hooks;
- `ended_ms`: set by the new `SessionEnd` hook (but not for `/clear`, which starts the same agent over) and when the daemon or the wrapper ends the session;
- `idle`: set after `Stop`, cleared by the next prompt;
- `doing`: the first line of the last prompt, at most 120 characters.

`session_touches` keeps the files each session's own edits touched. The daemon's in-memory `live` isn't used, as the two modes differ there.

An agent counts as present while its session hasn't ended and was heard from in the last 12 hours, which only matters after a crash. An agent the web UI runs (ACP) counts while it runs here.

**A starting agent is told who else is present in its project.** At `SessionStart` (`startup`, `resume` or `clear`), the hook's reply lists the present agents of other threads in the same project on this machine. Each entry gives:
- the agent's thread and agent;
- where it works: "in your working tree", "in its own checkout `.silt/checkouts/<id>`", or "in another clone at …";
- whether it's working now or waiting for its user, and when it was last active;
- what it was asked;
- its recent files.

The reply then says how to coordinate, and gives the agent its own thread id. After a compaction, the same list follows Silt's instructions. With no one else present, nothing is said.

**The agents already there hear of the newcomer.** On `startup` and `resume`, each present agent gets an arrival message describing the newcomer from where it itself works.

**Agents message each other.**
- `silt_agents` lists the present agents again.
- `silt_message {to, text}` sends one of them a message:
  - `to` is part of a thread id, and must name exactly one present agent, never the sender;
  - the text is 1 to 4000 characters.
- Both are MCP tools. They ask the daemon (`Request::Agents`, `Request::Message`) or, without one, the store directly, as capture does.
- A run's token lets an agent the web UI runs use them for its own thread only.
- `silt_message` is the first of Silt's tools not marked read-only.

**A message reaches a busy agent after its next tool call.** Messages and arrivals wait in the store's `agent_messages`, apart from `notices`, since they have a sender and a different moment of delivery. They are taken once (DELETE…RETURNING) and handed over:
- through `PostToolUse`'s `additionalContext`;
- with the next prompt for an idle agent, after the thread's notices;
- for an ACP agent, with its next prompt.

Each is worded as from another agent, naming its thread and where it works, "not your user", with how to reply. Silt's notices (pulls, merge request outcomes) still come only with the next prompt.

**Silt tells only the agents.** A second `silt claude` in a busy clone starts as before, with no warning in the terminal.

**`silt_in_flight` leaves out the asking agent's thread.** Before this, it left out the thread checked out at the working directory, which in a shared clone could be the other agent's. `Request::InFlight` takes the caller's thread, and the MCP tool and the web UI pass theirs.

## Consequences
- Awareness is per machine. Agents on other machines, and presence in the web UI beyond the agent's state, come later.
- An agent that crashed without `SessionEnd` and without its wrapper noticing is listed for up to 12 hours. A pid-based check would end that sooner.
- An idle agent gets messages only when its person prompts it again. Silt doesn't wake it.
- Every hook writes `seen_ms`, and every `PostToolUse` looks for messages: a small write and a read per hook.
- The `doing` text is the person's own prompt, shown to the other agents of the project on the same machine.
