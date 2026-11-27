# 0140: A thread chooses Silt's sandbox in the composer

- Status: accepted
- Date: 2026-11-27
- Amends: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md) (the sandbox stays out of the composer), [ADR 0094](0094-silt-s-sandbox-is-its-own-rule.md) (the sandbox is a workspace rule only)

## Context
ADR 0093 kept the sandbox out of the composer: a run's sandbox was set by its workspace's rules when it started, and the composer held only what a person changes mid-run. ADR 0094 made the sandbox the workspace rule `silt_sandbox`, set only in Settings › Workspaces. ADR 0135 then took the thread header's shield into Details › Access.

So running one thread sandboxed meant changing its whole workspace's rules and every other thread's with them, and changing them back afterwards. Nothing in the composer said whether the next run would be sandboxed. A new chat starts with full access, which is where a sandbox matters most, and had no way to ask for one.

## Decision
**The composer has a sandbox chip before the agent's mark,** in a thread, New thread and New chat.
- **The icon** says what the run is in: an outline shield when it isn't sandboxed, a shield with a check, in `ok`, when it is, and a shield with `!`, in `danger`, when this machine can't make the sandbox.
- **Its popover:**
  - **Run in Silt's sandbox**, a switch;
  - the four network modes;
  - devices;
  - the GPU, the Docker socket and `ssh-agent`, with what each hands over;
  - the workspace's own lists (hosts, ranges, ports, folders) and its plugins, for reading;
  - **Make it the workspace's**, **Use the workspace's** and a link to Workspace settings.

**The choice is the thread's.** It's kept with the thread's agent run (`agent_runs.sandbox`) and used each time Silt starts the thread's agent: from the web UI, from a terminal (`silt claude` in the thread), and for an Auto judge's agent rules.
- **It starts as the workspace's own rules.** Until a thread changes something, the workspace's rules apply as before. A choice back to the workspace's is forgotten.
- **It replaces only the person's own rules,** which become defaults. These are `silt_sandbox`, `network`, `devices`, `gpu`, `docker` and `ssh_agent`. The workspace's lists stay its own, and so do its other rules (`sandbox`, `isolated`, the agents, the preset cap).
- **An org's policy still holds it,** as it holds the workspace's: an org that requires the sandbox locks the switch on, its network and device caps grey out what's past them, and what it refuses can't be ticked.
- **A running agent keeps the sandbox it started in.** A change marks the shield with a dot and offers **Restart now**, which stops the agent and starts it again in the thread, or waits for its next start.
- **Phones on the LAN can't change it.** What a sandbox hands over, Docker's root included, is the machine's to give. Setting it, or starting with one, needs the machine's own UI.

**The API:**
- `POST /api/threads/{id}/agent/sandbox` sets the choice, or clears it with `null`.
- A start's `sandbox` sets it for a new thread or chat.
- The live state's `sandbox` says what the thread chose.

The hosted app shows no sandbox, as before.

## Consequences
- **The sandbox is a choice a person makes per thread,** not only a workspace's rule. A workspace that must sandbox every agent says so through its org's policy, which a thread can't loosen.
- **Details' "The next run"** shows the thread's choice.
- **A new migration** adds `agent_runs.sandbox`. The daemon's API changes, the sync protocol doesn't.
- **The popover opens like the composer's other chips,** above the composer when it's docked at the bottom. On a phone it's a popover across the screen's width, not a sheet.
