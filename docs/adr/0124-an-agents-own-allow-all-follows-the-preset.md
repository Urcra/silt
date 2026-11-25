# 0124: An agent's own allow-all follows the preset

- Status: accepted
- Date: 2026-10-27
- Numbered 0049 until 2026-11-26, a number another ADR also had.
- Amends: [ADR 0048](0048-an-agents-modes-are-its-permissions.md)
- Amended by: [ADR 0118](0118-an-agents-own-modes-in-the-permissions-menu.md): An agent's own modes in the permissions menu

## Context
ADR 0048 made an agent's modes its one permissions control where they are about permissions, as Claude's are. Copilot's are not: Agent, Plan and Autopilot say how it works. It has its permissions elsewhere, in an Allow All setting tagged `permissions`. Autopilot also turns allow-all on and runs to the end without the person.

The composer showed all three: a Mode chip, Allow All among the settings, and Silt's preset. They could disagree. With Allow All on, Copilot asked nothing, while the preset chip said "Ask before commands". Copilot's mode ids are links (`…/session-modes#autopilot`), so Silt matched none of them. Copilot also lists its mode a second time as a setting, which New thread sent at the start, putting it back into Autopilot.

## Decision
**Silt's preset is the one permissions control of an agent whose modes are not about permissions.** An agent's own on/off switch for asking nothing (a setting tagged `permissions` with choices such as on and off) follows the preset: on for Full access, off otherwise. Off, the agent asks and the preset answers. The composer doesn't show that switch.

**Mode ids are compared by what follows their `#`.** Autopilot maps to Full access. The other presets move Copilot into Agent, which asks as its Allow All says. Full access never moves it into Autopilot, which does more than not ask.

**Before the agent runs, its mode is offered only where its modes are its permissions.** Otherwise it starts in the mode its preset says, and its mode listed as a setting isn't offered or sent.

## Consequences
- Copilot's composer shows its Mode, model and effort, and the preset. Nothing else in it is about permissions.
- An agent that tags a permissions setting with other kinds of choices keeps it as a setting the person sets.
- Leaving Autopilot for Agent keeps Full access, as ADR 0048 does for a mode it doesn't know.
