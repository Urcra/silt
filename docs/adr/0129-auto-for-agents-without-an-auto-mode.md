# 0129: Auto for agents without an auto mode

- Status: accepted
- Date: 2026-11-25
- Amends: [ADR 0114](0114-one-set-of-permissions-and-agents-by-their-marks.md) (Auto moves an agent into its own auto mode)
- Builds on: [ADR 0022](0022-agent-runtime.md) (the daemon answers permission requests by the preset), [ADR 0106](0106-silt-s-sandbox-guards-against-an-eager-agent.md) (what the sandbox guards against), [ADR 0111](0111-steering-without-stopping.md) (steering), [ADR 0120](0120-agents-ask-other-agents.md) (asked agents, and reports handed over later)

## Context
ADR 0114 made Auto one of the five presets. An agent with an auto mode of its own, such as Claude Code, is moved into it, and its own check lets safe actions through. An agent without one, such as GitHub Copilot, is moved to the mode for Ask before commands, and the menu shows Auto greyed out: "GitHub Copilot has no auto mode". On those agents every command waits for the person.

Every permission request from an agent Silt runs over ACP already reaches the daemon (`permission_asked`). The daemon answers by the thread's preset what it can, and leaves the rest for the person. So the daemon can put a check of its own where an agent has none.

A check needs a model. Silt can't name a cheap one: whatever it names today may be gone in two years, or be one the person has no access to. The only model surely there is the one the thread already runs on.

Silt's gateway holds a model key only when the agent was given one, such as `OPENAI_API_KEY`. Sign-ins kept by the agent, such as Codex's ChatGPT sign-in, Copilot's GitHub one or Claude's subscription, leave Silt nothing to call, and Copilot's models have no API reached with a key. Calling providers' APIs directly would also mean one request format per provider, to keep up to date.

## Decision
**Auto works for every agent Silt runs over ACP.**
- An agent with an auto mode of its own keeps it, as ADR 0114 says.
- An agent without one is moved to the mode for Ask before commands, as now. Reads and edits go through as they do there. Each request the preset would leave for the person goes to a judge first.

**The judge is the same agent, on the same model, in a session of its own.**
- The daemon starts it as it starts an asked agent (ADR 0120). It runs over ACP, with the agent's own sign-in, in a sandbox with the thread's reach, and without Silt's MCP tools.
- It isn't a thread. Nothing of it enters the family log but its verdicts.
- It may read and search, for example to look at a script before judging the command that runs it. The daemon refuses anything else it asks.
- Each check starts the agent in a session of its own and stops it once it answers, so one verdict doesn't sway the next. Starting it takes a second or two; keeping one ready per thread is later work.

**Which agent and model judges is a setting of the workspace, from the agents it allows.**
- The judge runs in the thread's workspace, with its sandbox, credentials and rules. So only an agent that workspace's rules and its org's allow may judge there. Otherwise a workspace could have its commands judged by an agent it refuses.
- Settings › Auto, in the Workspace group beside Plugins, has a card per workspace:
  - who checks there: "Same as the thread's agent and model" by default, which follows the thread's model when that changes, or an agent and model the workspace allows;
  - what Auto does for each agent the workspace allows: its own auto mode, or checked by Silt.
- An agent the workspace refuses stays listed in the picker, greyed, with why, as a refused preset is (ADR 0065).
- An org may fix the judge for its workspaces. The card then shows it locked, with "Set by" the org.
- A thread may choose another judge in the composer's permissions menu, as "Checked by" under Auto, from the same list. On a phone it opens in place of the menu.
- The daemon checks the judge against the workspace's rules before each check, as it checks an agent before starting it. One no longer allowed, because the rules changed since it was chosen, gives way to the thread's own agent and model. That agent is allowed, or the thread couldn't run.
- The default judge is a new optional field of a workspace's defaults (`auto_judge`), and the org's fixed one a new optional field of its policy.

**The judge is given the call and the person's intent, not the agent's words.**
- The call as the agent asked for it: its kind, its title, its raw input and its working directory.
- The thread's preset and cap, and what the sandbox reaches.
- The person's last three prompts in the thread.
- Never the agent's messages or tool output. Those are where a prompt injection would sit.

It answers with one JSON object, `{"verdict": "allow" | "refuse", "reason": "…"}`.

**An allowed call goes through. A refused one goes back to the agent, with the reason.**
- Allow: the daemon answers with the agent's "allow once".
- Refuse: the daemon answers with "reject once", then tells the agent: "Silt's Auto check refused `<call>`: <reason>. Do it another way, or, if it's needed, ask the person with `silt_escalate`."
  - It's sent as a steer where the agent takes steers (ADR 0111).
  - Otherwise it's sent as a message ahead of the queue, once the turn ends.
- No verdict: the request waits for the person, as it does today, and its card says why. That covers a judge that can't start, takes more than 60 seconds, answers something unreadable, or hits the agent's plan limit. Silt falls back to the person, not to a refusal.
- A call the judge allowed goes through again without asking it, while the thread stays on Auto. That is the same kind, input and working directory, by their hash.

**The agent escalates with `silt_escalate`, which shows the person a card to approve.**
- `silt_escalate {call, why}` is a tool on Silt's MCP server, given to every agent the web UI runs. It names a call Auto refused in the agent's run, exactly as the refusal named it; anything else is refused at once, so the agent can't use it to ask for more than Auto refused.
- The card shows the call, the judge's reason, and the agent's `why`, with Allow and Refuse. The person may add words to either.
- It waits up to 50 seconds for the answer, as `silt_ask` does. After that the answer is handed over later, as ADR 0120's reports are.
- An allowed escalation is bound to the call's hash. The agent's next request for that exact call goes through without the judge, once.
- A refused escalation is told to the agent with the person's words.
- Escalations show in Needs you, as permission requests do.

**Verdicts show on the call's card.**
- "Auto allowed" or "Auto refused", the reason, and the agent and model that judged.
- While the check runs, the card says so.
- Verdicts are recorded in the thread's ACP lines under `_meta.silt.auto`, so the wire format doesn't change. `silt_proto::view`'s `ToolView` gains `auto`, for both serving modes.

**The judge is a convenience, not a boundary.**
- The sandbox is the boundary (ADR 0106).
- The judge reads text the agent wrote, which an injection can shape. So it only answers requests the preset would leave for the person.
- It never widens what the sandbox reaches.
- Auto stays the same step under a cap.

## Consequences
- Auto is no longer greyed out for an ACP agent without an auto mode. The menu says the check uses the judge's model. Settings › Chats' line on Auto points to Settings › Auto.
- Each command the preset would have asked about costs a judge turn on the person's plan: seconds, and tokens. The card shows how long the check took.
- Agents run in a terminal, captured by their hooks, aren't covered. A pre-tool hook could ask the daemon's judge, where a CLI's hook can return a decision and a daemon runs. Without a daemon, the hook leaves it to the CLI's own prompt (ADR 0018). That's later work.
- Where the code goes:
  - `silt-access` (pure): the judge's prompt, reading its verdict, and a call's hash.
  - `silt-workspace` (pure): the judge in a workspace's defaults and an org's policy, and checking a judge against the rules.
  - `silt-daemon`: the judge processes, the check in `permission_asked`, the hand-back and the escalation's grant.
  - `silt-mcp`: `silt_escalate`.
  - `web`: the setting in the permissions menu and Settings › Auto, the verdict on the call's row, and the escalation card.
- A workspace's default judge and an org's fixed one are new optional fields. An older machine ignores them, and has no judge to apply them to, so `PROTOCOL` stays.
- An escalation is answered on the machine running the agent, or a phone paired with it. The hosted app doesn't relay the answer yet.
