# 0140: Agents run at the versions Silt checked

- Status: accepted
- Date: 2026-11-27
- Amends: [ADR 0005](0005-acp-registry.md) (where an agent's version comes from)
- Builds on: [ADR 0138](0138-enter-steers-mid-turn.md) (what broke)

## Context
Silt started each registry agent at the version the public ACP registry named that day: it fetched `registry.json` at most once a day, and the copy built into Silt stood in only offline. So a new release of an adapter reached everyone within a day, untested by Silt.

That broke steering. ADR 0138 steers Claude with a prompt sent while it works, detected by `claudeCode.promptQueueing` in the v2 `initialize` of `claude-agent-acp` 0.109.0, the version Silt's copy names and the one tested. The public registry moved to 0.113.0, which no longer advertises that flag on v2 and sends such a prompt at priority `now` rather than `next`. Silt then took Claude for an agent that can't steer, and ⏎ queued again.

## Decision
**An agent Silt checked runs at the version Silt checked it at.** The registry built into Silt (`crates/silt-acp/registry-snapshot.json`) is the list of checked agents and their versions. A newer version in the public registry is never started; it waits until it's checked and the built-in copy updated.

**An agent only the public registry lists is still offered, marked unverified.** Its description in the agent menu starts with "Not checked by Silt yet.", and it runs at the version the public registry names, as before. No new UI is added for it.

**The public registry is still fetched daily**, for those agents, and to learn that a newer version exists. With Agent registry off in Settings › Network, only the built-in copy is used.

**Updating a version is a checked change.** A scheduled check compares the public registry with the built-in copy, runs a newer version through Silt's checks of the behaviour Silt relies on (what `initialize` advertises, steering mid-turn, cancel, permissions), and proposes the bump with what it found. Agents that need a sign-in are checked where one is available, or marked for a check by hand.

## Consequences
- Claude runs `claude-agent-acp` 0.109.0 again, so ⏎ steers it at its next step.
- The built-in copy has to be kept up: an agent's fixes and features reach Silt's users only once its version is checked and bumped.
- The versions in the built-in copy today are the baseline: they are the ones Silt has shipped and run.
- A custom agent in `agents.json` runs as the person configured it, unchecked as before.
