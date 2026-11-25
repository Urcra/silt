# 0120: Agents ask other agents

- Status: accepted
- Date: 2026-11-24
- Builds on: [ADR 0010](0010-forks-pulls-and-merge-requests.md) (forks and pulls), [ADR 0051](0051-subagents-and-background-tasks.md) (subagent cards), [ADR 0089](0089-a-checkout-per-thread-in-a-shared-clone.md) (a checkout per thread), [ADR 0090](0090-agents-aware-of-each-other.md) (agents message each other), [ADR 0114](0114-one-set-of-permissions-and-agents-by-their-marks.md) (one set of presets)

## Context
Each agent's subagents run on its own harness and its own vendor's models. People want one agent to get a second view from another: Claude Code on Opus asks Antigravity on Gemini 4 Neon to review its change, or asks Codex to write the tests. Today they do it by hand: they copy the diff into another tool and paste the answer back.

Silt can offer this as an MCP tool that starts a child thread on another provider, whose permissions only narrow, with the parent woken by the result. Isolating the child in a worktree from HEAD would leave the parent's uncommitted edits out. ACP has no way for one agent to ask another, so this is the client's job.

Silt runs every agent it starts over ACP, gives every agent its MCP tools, and records every edit, committed or not, in the family's log.

## Decision
**An agent asks another through Silt's MCP tools:**
- `silt_askable` lists the agents and models it may ask here;
- `silt_ask {agent, model, effort, mode, checkout, prompt, wait}` asks one, or follows up on an earlier ask with `to`;
- `silt_ask_wait` and `silt_ask_stop` wait for an ask and stop it;
- `silt_ask_take` takes the changes of an ask that worked in its own checkout.

Agents in a terminal and agents the web UI runs have the same tools. The asked agent is started by the daemon over ACP, so an ask needs the daemon.

**The asked agent gets a thread of its own, and the caller chooses where it works,** as it does for its own subagents:
- `here` (the default): in the caller's working tree, beside it. Its edits are on disk at once, recorded as its thread's own, as two agents in one clone are (ADR 0089).
- `own`: in a checkout of its own, forked at the caller's head, so it has the caller's uncommitted edits.

Either way, its thread is a fork of the caller's, in the same family. The agent starts with the model and effort asked for, chosen from the settings it last listed here. Its first prompt is only what it was asked, not its caller's conversation, worded as from the calling agent rather than the person.

**`mode` is `review` or `work`.** A review's prompt says to change nothing; nothing else holds it to that. An `own` ask's changes stay in its thread until the caller or the person takes them, as a pull within the family.

**The report is the asked agent's last message when its turn ends,** and it's all the caller gets back, as with its own subagents. The transcript stays in the asked thread.
- `silt_ask` waits up to 50 s for it, below MCP clients' timeouts.
- After that, it's handed over as a message from the asked agent, as ADR 0090's messages are:
  - an ACP caller waiting for its person is woken with it, by a prompt Silt sends;
  - a busy ACP caller is woken once its turn ends;
  - an agent in a terminal gets it at its next `PostToolUse` or prompt.

A report a waiting call took isn't handed over again.

**An ask counts as a command.** The caller's preset decides as it would for any command: on Ask before commands (Edits) or stricter, the agent's permission request for `silt_ask` waits for the person; on Auto or Full access it goes through. An agent never answers another's permission request.

**Rules decide who may ask whom.** They're workspace rules, each capped by an org's:
- agents asking other agents: Never or Allowed (the default);
- which agents may be asked (any the workspace allows by default);
- the most an asked agent may do (its caller's preset by default). It never gets more than its caller;
- asks at once per thread (2);
- whether an asked agent may ask further (no).

A refused ask fails at once with the rule's reason, and nothing is started.

**The caller's thread shows the ask as a card,** led by the asked agent's mark, name and model, with its steps as it takes them, then its report. It shows above the composer while it runs, with Stop. The asked thread nests under its caller in the sidebar and says who asked it. Its composer works, so the person can steer it too.

**Stopping the caller's turn stops its asks.** A steer, which ends a turn to say more, doesn't.

## Consequences
- The daemon keeps this machine's asks in `asks.json`; one a stopped daemon left running reads as stopped. Nothing new enters the family log or the wire format: the asked thread is an ordinary fork, so another machine or the hosted app shows it as a fork, not as an ask.
- The rules are new optional fields of a workspace's and an org's policy. An older machine ignores them, and has no asks to apply them to.
- An ask spends another agent's plan.
- Not done yet:
  - what an ask cost, on its card;
  - Take over on the asked thread;
  - a person asking from the UI ("a second opinion");
  - asking an agent on another machine.
