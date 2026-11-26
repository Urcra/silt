# 0132: A thread's header holds its title and actions, and its menu is a right-click

- Status: accepted
- Date: 2026-11-26
- Amends: [ADR 0044](0044-the-standard-agent-workspace.md) (the thread header and its overflow menu), [ADR 0114](0114-one-set-of-permissions-and-agents-by-their-marks.md) (the thread header no longer shows the agent's mark), [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md) (the header's sandbox shield)

## Context
The thread header had grown to two lines and up to eleven things: the title, a state pill, This machine only, a meta line (project, branch, machine, agent, fork of, asked by, id), the sandbox shield, Fork, Offer as merge request, Share, a More menu, Expand and the panel's toggle. Most of it was said again elsewhere: the sidebar shows the project and whether the agent is working, the panel's Details and Access tabs show the machine, lineage and sandbox, the conversation shows the agent, and This machine only is a project's setting. More held two transcript settings that apply to every thread, terminal commands, Join live, the history graph and Details. On a phone the title got one letter.

A bar over the composer repeated the changes' counts, a Review that the panel's toggle already is, a fork's Offer as merge request, and Create PR on the forge.

## Decision
- **The header is one line:** the title, a pill only when the agent waits on the person or stopped with an error, who else is here, then Fork, Offer as merge request on a fork, Share and the panel's toggle. No meta line, no This machine only, no shield, no More.
- **A thread's menu is a right-click** on it in the sidebar or on All threads (the row's ⋯ on hover opens the same menu): pin, snooze and settle, Copy thread ID (what `silt resume` and `silt fork` take), Fork, Move to a project… for a chat, History graph, Join live, and Erase from this machine… for a Local only thread.
- **The transcript's grouping and detail** are set in Settings › Text and layout, where they were already kept for the machine, and from the palette.
- **What the agent's sandbox reaches** is in the panel's Access tab, which also says what the workspace's rules give the next run.
- **Expand** sits in the Changes panel's header; the branch goes beside Details' commits.
- **The bar over the composer is gone,** and Create PR on the forge with it.

## Consequences
- The title keeps the header's width, on a phone too.
- Each fact has one place. Someone looking for the thread's machine or agent finds it in the panel, not at a glance.
- The daemon's pull request endpoints have no caller in the UI.
- A phone has no right-click; the row's ⋯ and a long press, where the browser fires it, open the menu.
