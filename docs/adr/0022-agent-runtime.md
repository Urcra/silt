# 0022: The agent runtime

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0014](0014-live-collaboration-and-silt-driven-agents.md) (agents started from the web UI)
- Amended by: [ADR 0044](0044-the-standard-agent-workspace.md): The standard agent workspace: open on the composer, one shell for every screen
- Amended by: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md): Silt sandboxes the agents it starts, and production is reached through plugins

## Context
Today an ACP agent started from the web UI is an `AcpRun` held only in the daemon's memory:
- its state is one of `starting`, `ready`, `working` or `exited`;
- the permission requests it is waiting on are parked;
- a prompt sent while it works is refused.

A restart loses all of it. Nothing lists which threads have an agent, or what each is waiting for, without opening the thread. The workspace ([ADR 0020](0020-next-generation-web-ui.md)) needs:
- every thread's agent state in the sidebar;
- a queue of prompts;
- steering;
- permission presets.

## Decision
**States.** An agent in a thread is in one of these:
- `starting`;
- `idle`: ready for a prompt;
- `working`;
- `awaiting_permission`: a permission request is open;
- `awaiting_input`: it stopped to have a plan approved, see below;
- `error`: it couldn't start, or failed, with the error kept;
- `stopped`: it exited, was stopped, or the daemon restarted.

**What the store keeps:**
- **Each thread's last state and error, and its preset and mode.** The sidebar reads them for every thread. After a restart a working agent reads as `stopped`, since its process went with the daemon. The state is personal to the machine, so it stays out of the family log.
- **The queue.** Each item has its author, its text, its place and when it was added.
  - When a turn ends, the next item is sent.
  - "Send now" sends an item at once, steering if the agent works.
  - Items can be edited, reordered and removed until they are sent.
  - The queue outlives the agent. An agent started later in the thread takes the queue up.
  - ADR 0020 has the server hold the queue so a phone can add to it. Until the control channel exists (M14), the daemon holds it, behind the same routes.

**Steering.** ACP can't add to a running turn. Steering cancels the turn (`session/cancel`), then sends the steer as the next prompt. That prompt says the previous turn was interrupted to tell the agent this. ADR 0111 reverses this for agents that advertise `_session/steering`: their steer goes into the running turn, and only the others are cancelled.

**Permission presets** are the daemon's policy, chosen per thread:
- **Ask before commands** (the default): a request whose tool is an edit, a read or a search is allowed once without asking. Anything else asks.
- **Ask for everything:** every request asks.
- **Full access:** every request is allowed without asking. It is meant for a machine the person trusts. The composer says so, and says whether the agent's own sandbox is on.

A request the preset doesn't answer goes to the person, as today. The person's answers are the agent's own options, among them "Allow for this session" where the agent offers an allow-always option. Silt sandboxes nothing itself ([ADR 0017](0017-acp-agents-only.md)). An agent's sandboxed mode is chosen through its ACP modes.

**Modes and Plan first.**
- The composer offers the agent's ACP session modes, and its models where it offers them.
- "Plan first" uses the agent's plan mode where it has one. Otherwise the prompt asks for a plan before any edit, and the turn's end leaves the agent `awaiting_input` until the plan is approved or changed.

## Consequences
- The sidebar shows, for every thread with an agent, what it is doing and whether it waits on the person, without opening the thread.
- Prompts aren't refused while the agent works: they queue.
- A preset that allows without asking is a choice made per thread, on the machine that runs the agent.
