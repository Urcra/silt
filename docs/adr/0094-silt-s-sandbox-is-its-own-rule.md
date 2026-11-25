# 0094: Silt's sandbox is a workspace rule of its own

- Status: accepted
- Date: 2026-11-17
- Amends: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md) (Silt runs every agent it starts in its sandbox)

## Context
ADR 0093 said every agent Silt starts runs in Silt's sandbox. As built, it ran there only where a workspace required the agent's own sandbox (`sandbox`, or `isolated`), and only where bubblewrap happened to work. Otherwise the agent's own sandbox stood in, with nothing said.

That confused three things:
- **Nothing in Settings turned it on.** The switch was a rule named "Agent's own sandbox".
- **What a sandbox reaches looked in force when it wasn't.** Settings showed network, devices and the machine's extras for every workspace, so one without a sandbox seemed to keep USB devices from its agents, while its agents ran as the person.
- **A requirement could quietly not be met.** A machine without bubblewrap fell back to the agent's own sandbox, and Copilot or an ACP agent was refused for a sandbox Silt never tried to give.

## Decision
**Silt's sandbox is its own rule, `silt_sandbox`,** off unless a workspace or its org turns it on.
- **Turning it on:** `silt workspace policy --silt-sandbox on`, `silt org policy --silt-sandbox`, or **Run its agents in Silt's sandbox** in Settings › Workspaces, at the top of "What its agents can reach".
- **Where it can't be made, agents don't start** in that workspace. The refusal names the rule's source and what `silt access doctor` would say. Settings says so beside the switch.
- **The reach rules apply only with it on.** While it's off, Settings greys them out and says why.
- **`sandbox` and `isolated` ask for the agent's own sandbox again,** as in ADR 0065. Silt's sandbox counts for both when it's on.
- **The backend is a rule too, `backend`,** bubblewrap when unset, and an org's choice holds on its members' machines. bubblewrap is the only one so far. Settings still shows the choice, **Isolation**, with containers and VMs listed as not yet there. Showing it now means adding them later changes nothing else on the page. Each backend says whether it can run on this machine, and why not.

## Consequences
- **Workspaces set up since ADR 0093 that relied on `sandbox` turning Silt's sandbox on** need `--silt-sandbox on`. Nothing is released yet, so the protocol stays at 19.
- **The thread's shield** says "The next run is sandboxed" only where the rule is on.
- **The default stays off for now.** Turning it on by default waits for macOS and for bubblewrap on every Linux install, from source included.
