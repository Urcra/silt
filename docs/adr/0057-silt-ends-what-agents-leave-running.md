# 0057: Silt ends what agents leave running

- Status: accepted
- Date: 2026-10-31
- Amends: [ADR 0051](0051-subagents-and-background-tasks.md) (how long a task shows as running, and stopping one)

## Context
Under ADR 0051 a task runs until news of its end comes. For a thread whose agent the web UI ran, the web UI also counted it as running while the thread had done something in the last hour. Nothing recorded that a task ended with its agent:
- when the person stopped the agent;
- when the daemon stopped, which left its agents to die with their pipes. A command they had left in a process group of its own lived on, orphaned;
- when the machine went down.

So a thread opened again soon after showed its commands as still running, with spinners, and no way to stop them. ACP has no way to ask an agent to stop one command it left in the background. Claude's adapter offers `_session/async_task/stop` on v1 only, behind a flag.

## Decision
**What a web UI agent left running ends with it, and its history says so.** Silt records `stopped` news for every task whose news last said it runs (`silt_agents::acp::left_running`), in the session that recorded the task and inside the subagent that started it, where one did. It does so:
- when the agent exits, whether the person stopped it or it died;
- when the daemon shuts down, after ending each agent with everything it started (`AgentHandle::end_now`);
- when the daemon starts, for each run its store says was running: that agent went with the daemon's last process, or the machine.

Ending an agent ends its process group, then each process it had started, found by parent among the machine's processes (`silt_os::processes`). That also ends commands the agent started in groups of their own.

**The person can stop a background command from the web UI.** A running command's card, and its line above the composer, have **Stop** while the agent that started it runs on this machine (`AgentStatus.stoppable`). The daemon (`POST /api/threads/{id}/agent/tasks/{call}/stop`) looks among the agent's descendants for the topmost process whose command line holds the command the agent gave. Quotes, backslashes and spaces don't count, as the agent's shell quotes the command again. Of several such processes, it takes the one as old in its order as the task is among the agent's tasks running that command. That process and everything under it are killed, and the task's news says `stopped`.

The agent hears that its command ended and goes on by itself. That turn is about something the person saw to, so on v2 Silt cancels it at once, as it does after the person stopped the agent. The turn's notice says the command was stopped. The agent's own news of that end is dropped, as the person's stop came first. On v1 (Copilot) a cancel would be ignored and stopping it would end the agent, so its turn goes on.

A stop can't be passed on through the server to the machine running the agent; the hosted app shows no Stop.

## Consequences
- After a restart, a crash or a power cut, a web UI thread's tasks read as stopped, not running.
- The one-hour guess stays for sessions run in a terminal (`silt claude` and the other CLIs). Their history is the agent's own files, and Silt can't tell whether their tasks still run.
- A command whose process doesn't show what the agent said it runs, such as one the shell rewrote, can't be found; Stop then says so. A process outside the agent's tree with a matching command line is never touched.
- Subagents run inside the agent, so only stopping the agent stops one.
- Listing the machine's processes takes `sysinfo`, in `silt-os`.
