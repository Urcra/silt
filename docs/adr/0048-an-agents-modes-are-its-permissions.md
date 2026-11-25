# 0048: An agent's modes are its permissions

- Status: superseded by 0114
- Date: 2026-10-25
- Amends: [ADR 0044](0044-the-standard-agent-workspace.md) (the composer's permissions chip and the plan card)
- Amended by: [ADR 0065](0065-workspaces.md): Workspaces: a server, defaults, rules and history apart for each kind of work
- Reversed by: [ADR 0114](0114-one-set-of-permissions-and-agents-by-their-marks.md): One set of permissions, and agents shown by their marks
- Amended by: [ADR 0124](0124-an-agents-own-allow-all-follows-the-preset.md): An agent's own allow-all follows the preset

## Context
ADR 0044 gave every thread a Silt preset (Ask for everything, Ask before commands, Full access, Plan) that answers an agent's permission requests for the person. Agents spoken to over ACP, such as Claude's, also have modes of their own (Manual, Accept edits, Plan, Auto, Bypass permissions) that decide what they ask in the first place. The composer showed both as two chips, and the two drifted apart:
- choosing Plan left the agent's mode on Manual until a prompt went;
- approving a plan moved the agent's mode while the preset still said Plan;
- a later visit showed a preset nobody had chosen.

An end-to-end review also found the plan card offering the agent's own five answers ("Yes, and bypass permissions", "auto mode") where ADR 0044 has the preset to go on with.

## Decision
**While an agent with modes runs, its mode is the thread's one permissions control.** The composer shows the Mode chip alone, and Shift+Tab moves to the next mode.

**The preset follows the mode.** Each mode Silt knows maps to a preset:
- `default` and `read-only` → Ask for everything;
- `acceptEdits` → Ask before commands;
- `auto`, `bypassPermissions` and the like → Full access;
- the agent's plan mode → Plan.

A mode Silt doesn't know keeps the preset chosen, except that leaving the plan mode leaves Plan. The one exception is Silt's own switch into the plan mode for a single Plan first turn, which it undoes itself.

**Choosing a preset moves the agent to the matching mode, when it offers one.** A run whose mode was never chosen starts in the mode of the thread's preset.

**A plan the agent asks to approve is answered in presets.** The card shows Approve with "then" one of the presets whose answer the agent offers, plus Keep planning. The agent's other answers, such as clearing its context first, stay under Other choices.

New thread, before any agent runs, keeps the preset chip. It also offers the settings the agent last ran with (its model, its thinking level), applied once the agent is up.

## Consequences
- The composer never shows two controls that can disagree.
- Silt's automatic answers (ADR 0022) stay consistent with what the agent itself will ask.
- An agent without modes keeps the preset chip as before.
- The map of mode ids is the adapters' (Claude's and Codex's) and may need new entries as agents add modes.
