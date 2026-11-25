# Agents asking other agents

An agent working through Silt can hand part of its work to an agent on another harness or model. Claude Code on Opus can ask Antigravity on Gemini 4 Neon to review its change, or ask Codex to write the tests. The asked agent works in a thread of its own, and its last message comes back to the agent that asked, as its report. The decision behind this is [ADR 0120](https://github.com/Urcra/silt/blob/main/docs/adr/0120-agents-ask-other-agents.md).

It works for an agent in your terminal (`silt claude` and the others) and for one the web UI runs. The asked agent is always started by Silt's daemon over ACP, so it needs the daemon running and the agent installed and signed in on the same machine.

## Asking
Silt gives every agent four tools for this, beside its history tools:
- `silt_askable` lists the agents it may ask here, with the models and effort levels each offers.
- `silt_ask` asks one. It says:
  - the agent, and optionally its model and effort, which Silt chooses for it before it starts;
  - the prompt. The asked agent starts with only this, not the caller's conversation;
  - `review` (the default) or `work`: a review is told to change no files, a work ask may edit;
  - `here` (the default) or `own`: where the asked agent works, as below.
- `silt_ask_wait` waits again for an ask's report.
- `silt_ask_stop` stops one.
- `silt_ask_take` takes an `own` ask's changes.

You can ask your agent to do it in words, such as "when the tests pass, have Antigravity review it". It picks the agent and writes the prompt.

`silt_ask` waits up to 50 seconds for the report. A longer ask carries on, and its report reaches the agent by itself:
- an agent the web UI runs is woken with it if it's waiting, or gets it when its turn ends;
- an agent in a terminal gets it after its next tool call, or with your next prompt.

`silt_ask` with `to` set to an earlier ask's id follows up with the same agent, in the same thread.

## Where the asked agent works
The calling agent chooses, as it does for its own subagents:
- **`here`**: in its own working tree, beside it. What the asked agent edits is on disk at once, recorded as the asked thread's edits, as two agents in one clone are ([Your agents](agents.md#two-agents-in-one-clone)). Keeping out of each other's way is the caller's job, as with its own subagents.
- **`own`**: in a checkout of its own, forked from the caller's thread with its uncommitted edits. Its changes stay there until they're taken: by the agent with `silt_ask_take`, or by you with **Take into this thread** on the ask's card. Taking them is a pull within the family.

## Permissions
Asking another agent counts as a command. Your agent's preset decides as it would for any command:
- On **Ask for everything**, **Plan** or **Ask before commands**, the ask waits on a permission card in the thread.
- On **Auto** or **Full access**, it goes ahead.

The asked agent runs at the same preset as its caller, or lower if a rule caps it. It never gets more than its caller.

## In the web UI
- The call shows as a card in the caller's thread, led by the asked agent's mark, name and model. Opened, it says where the agent works and what it was asked, shows its steps as it takes them, then its report.
- While it runs, it shows above the composer with **Stop**. Stopping the caller's turn stops what it asked too.
- A thread's asks fold into one row under it in the sidebar: how many, whose, and whether one needs you, failed or is still working. Open it to list them, with the asked agent's mark; one that waits on you, and the one you have open, show even folded. Its page says who asked for it, and what you say there reaches the asked agent.

## Rules
A workspace's rules, in Settings › Workspaces, decide what agents may ask. An org's policy can tighten each one, never loosen it.

| Rule | By default |
|---|---|
| Agents asking other agents: Never or Allowed | Allowed |
| Agents they may ask | Any agent the workspace allows |
| Most an asked agent may do | What its caller may |
| Asks at once, per thread | 2 |
| Asked agents may ask further | No: one level |

A refused ask fails at once, with the reason, and nothing is started. The agent can then ask another agent or do the work itself.
