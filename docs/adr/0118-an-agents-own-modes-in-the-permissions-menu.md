# 0118: An agent's own modes in the permissions menu

- Status: accepted
- Date: 2026-11-24
- Amends: [ADR 0114](0114-one-set-of-permissions-and-agents-by-their-marks.md) (the chip kept for modes that say how an agent works, the lock, and the row's order) and [ADR 0124](0124-an-agents-own-allow-all-follows-the-preset.md) (Copilot's Mode chip)

## Context
ADR 0114 made the lock chip (a lock and a short label) the one permissions control, but kept a chip for modes that say how an agent works rather than what it may do. For Copilot that chip lists Agent, Plan and Autopilot. Without its "Mode" label it reads "Agent", next to the chip that picks the agent, so it looks like a second agent picker. Two of its three choices aren't needed anyway:
- Copilot's Plan and the lock's Plan are one thing: each sets the other.
- Agent is the mode Copilot is in whenever nothing else is chosen; the lock's other presets move it there.

Only Autopilot can't be picked from the lock. It is Full access that also runs on until the work is done, without returning to the person, so Full access never moves Copilot into it.

## Decision
**An agent's own modes are picked from the permissions menu.** Below Silt's five presets, under the agent's name, the menu lists the agent's modes that aren't:
- its default, the mode the presets move it to for Ask before commands, or else the first it lists (Copilot's Agent, OpenCode's Build);
- its plan mode, which the Plan preset is;
- a permission mode, which follows a preset (ADR 0114).

What's left is Copilot's Autopilot and, for example, OpenCode's primary agents a person defined. Choosing one moves the agent into it, and the chip shows its name. The agent's section shows only for agents that have such modes, as it ran last on this machine before it starts. The separate mode chip goes, in the composer and in its folded menu.

**The row starts with the agent.** The composer's row, in threads and on New thread, is the agent's mark, then the permissions, then the model and effort: which agent first, then what it may do.

**The permissions chip drops its lock.** It shows only its label (Plan, Ask, Edits, Auto, Full, or the agent's mode, such as Autopilot); its tooltip and its menu's heading say it's the permissions.

**What Silt answers in an agent's own mode:**
- Autopilot answers as Full access: nothing asks.
- A mode Silt doesn't know answers as Ask for everything: Silt puts every request the agent makes to the person, and the agent's own setup decides what it asks.

**Caps count them as Full access.** Autopilot is Full access, and Silt can't know what a mode it doesn't know lets through, as ADR 0065 already has it. A workspace capped below Full shows them greyed out, with why. An agent's default says how it works, as Copilot's do (ADR 0124), so no cap refuses it.

**Shift+Tab steps through the agent's modes too,** after Full access and before Plan again.

**Choosing a preset leaves the agent's own mode** for the mode that preset moves it to, or else its default. Full access from Autopilot is Agent with Allow All on (ADR 0124).

## Consequences
- Copilot's row is its mark, the permissions and its model; nothing in it repeats another control.
- An agent's modes need no new preset: the thread keeps its preset and its mode as before, so the protocol doesn't change.
- The thread's preset under an unknown mode is Ask for everything, which the composer doesn't show while the mode does.
