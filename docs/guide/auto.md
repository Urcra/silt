# Auto for every agent

Auto is the permissions preset between Ask before commands and Full access: it lets what's safe through and asks about the rest. Claude Code has an auto mode of its own, and Auto moves it into that mode. Most agents have none, so Silt checks their commands itself. The decision behind this is [ADR 0129](https://github.com/Urcra/silt/blob/main/docs/adr/0129-auto-for-agents-without-an-auto-mode.md).

This works for agents the web UI runs. An agent in your terminal keeps its own prompts.

## What Silt checks
On Auto, edits, reads and searches go through, as on Ask before commands. Each command, and anything else the agent asks permission for, goes to a judge first:
- The judge is an agent of its own, by default the thread's agent on the model the thread runs. It runs in a session of its own and in the thread's sandbox, with the agent's own sign-in, so it costs a turn on that agent's plan, usually a few seconds.
- It's given the call (its command, title and working directory), the workspace's cap, what the sandbox reaches, and your last three prompts in the thread. It isn't given the agent's messages or what its tools printed, as that's where a prompt injection would sit.
- It may read files in the checkout, such as a script the command runs, and nothing else.

What it decides:
- **Allowed:** the call goes through. The same call, with the same command and working directory, goes through again without a check while the thread stays on Auto.
- **Refused:** the call is turned down and the agent is told why, so it can do the work another way. An agent that takes steers hears it in its turn; any other hears it from Silt when its turn ends.
- **No verdict:** when the judge can't start, takes more than 60 seconds, or answers something that isn't a verdict, the request waits for you, as on Ask before commands, and its card says why.

In the thread, each call carries Auto's verdict: **Auto allowed**, **Auto refused** with the reason under the call, **allowed before** or **allowed by you**. Opening a call shows the reason, which agent checked it, and how long that took. While a check runs, the call says **Auto is checking**.

## When the agent needs what Auto refused
The agent can ask you to allow a refused call with `silt_escalate`, saying why it's needed. A card shows the call, why Auto refused it and the agent's reason, with a box for words of your own:
- **Allow once** lets that exact call through the next time the agent makes it, without the check.
- **Refuse** tells the agent no, with your words.

The thread waits on you meanwhile, in Needs you and on your phone. If you take longer than 50 seconds, the agent hears your answer later, as a message from Silt.

## Who checks
Who checks is set per workspace, as only agents a workspace allows may judge there. **Settings › Auto** has a card for each workspace:
- **Who checks:** the thread's own agent and model, or one of the agents and models the workspace allows. An agent the workspace refuses is listed greyed out, with why.
- **Its agents:** for each agent the workspace allows, whether Auto uses its own auto mode or Silt checks it.

An org can set who checks for its workspaces; the card then shows it locked. A thread can choose another of its workspace's agents under Auto in its permissions menu, as **Checked by**. If the rules change so the agent chosen is no longer allowed, the thread's own agent checks instead.

## What it's not
The judge reads what the agent wrote, which a prompt injection can shape, so it's a convenience that saves you answering, not a boundary. Silt's sandbox is the boundary: see [Silt's sandbox](sandbox.md). Auto's check only answers what you'd otherwise be asked, never lets the agent reach more than its sandbox does, and is the same step as Auto under a workspace's cap.
