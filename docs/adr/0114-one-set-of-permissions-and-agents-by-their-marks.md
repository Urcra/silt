# 0114: One set of permissions, and agents shown by their marks

- Status: accepted
- Date: 2026-11-22
- Reverses: [ADR 0048](0048-an-agents-modes-are-its-permissions.md) (an agent's modes stand in for the preset). Amends [ADR 0044](0044-the-standard-agent-workspace.md) (the composer's row) and [ADR 0065](0065-workspaces.md) (a cap's steps).
- Amended by: [ADR 0118](0118-an-agents-own-modes-in-the-permissions-menu.md): An agent's own modes in the permissions menu

## Context
The row under the composer read `Claude Agent ˅  Mode Auto ˅  Opus 5.5 ˅  Effort High ˅  ···  ◔`. It said the agent's name in full, labelled each chip, and gave every chip a chevron. Its permissions control changed with the agent: Claude showed its own five modes (Manual, Accept edits, Plan, Auto, Bypass) as a Mode chip, while Codex, Copilot and the rest showed Silt's four presets. Moving between agents moved the control and its words, and a workspace's cap had to be explained in both.

Claude's Auto mode had no preset of its own. ADR 0048 mapped it to Full access, though it still asks about what its own check finds risky.

## Decision
**Silt's presets are the one permissions control, the same for every agent.** There are five, by how much goes through without asking: Ask for everything, Plan, Ask before commands, Auto, Full access. The composer shows them first in its row, as a lock and a short label (Plan, Ask, Edits, Auto, Full); its menu gives each one's name and what it does. Shift+Tab steps through them. Choosing one moves the agent to its matching mode, as ADR 0048 already did. Agent modes that are permissions no longer show; modes that say how an agent works (OpenCode's Build, Copilot's Agent and Autopilot) keep their chip, now without the word Mode.

**Auto is a preset.** It moves an agent into its own `auto` mode. The daemon answers what the agent still asks as it does for Ask before commands: edits and reads go through, commands ask. An agent without an auto mode is moved to the mode for Ask before commands, and the menu shows Auto greyed out for it, with why. `--permission-mode auto` given to `silt claude` asks for Auto, not Full access, so a workspace capped at Auto allows it. A kept mode that is a permission no longer outlasts a preset chosen since; one that says how the agent works stays.

**Approving a plan goes on in a preset.** The plan card's "then" offers Ask, Edits, Auto and Full. Claude's plan answers have no Full access, so choosing it approves as Ask before commands and then moves the thread to Full access.

**A cap is five steps.** A workspace's and an org's "most it may do without asking" are the same control, Ask · Plan · Edits · Auto · Full, with Full as no cap. A step the org refuses stays shown, struck through, with why.

**Agents are shown by their marks.** The composer's agent chip is the agent's logo, its name in the tooltip and the menu. The thread header, New thread's list to pick up, Devices, the found sessions to import, the workspace switcher and the rules tables show logos too; where a rule is set, the chip keeps the name beside it. Logos come from Lobe Icons (MIT, see NOTICE), drawn in one colour. Silt's own agent has Silt's mark, and an agent Silt has none for shows its initial. They're keyed by `agentKey`, so an agent's CLI and ACP ids share one.

**Model and effort share one chip.** It shows the model and a bar for each level of how hard the agent thinks, filled up to the chosen one, and none for the agent's default, which isn't a level; its panel lists the models and the effort steps.

## Consequences
- `Preset` gains `auto`. A workspace's or org's cap may name it, which an older machine can't read, so the protocol is 22.
- Devices and the import list need the agent's id as well as its name: `AgentReport` and `DeviceAgent` carry `agent_id`, empty from older daemons, and `FoundSessions` carries the agent's id.
- The browser tests pick agents and presets through the menus (`pick` in `web/e2e/fixture.ts`) rather than `<select>`s.
- The logos are their owners' trademarks, used only to name their tools.
