# 0058: New thread's project and place outside the composer

- Status: accepted
- Date: 2026-11-03
- Amends: [ADR 0054](0054-the-desktop-ui-review.md) (New thread)
- Amended by: [ADR 0059](0059-chats-without-a-project.md): Chats without a project

## Context
ADR 0054 put all of New thread's settings in the composer's row: the project, where it works, the agent and the permissions, with a line under the composer saying where it would run. In use the row read as overloaded. The project, the choice that frames the rest, became one small chip among five. Local, Worktree and Cloud, a choice of three always worth seeing, was hidden behind a menu.

## Decision
- The project is the heading's picker again: "What should we work on in ⟨project⟩?".
- Local, Worktree and Cloud are a segmented control under the composer, with the base branch beside Worktree. A line under it says where the thread will run, naming the device.
- The composer's row keeps what the agent is: attachments, the agent, its mode and settings, and the permissions.
- Pick up still says each thread's state.

## Consequences
- The screen references of New thread were taken again.
- The e2e specs pick where a thread works by its radio again, rather than with `selectOption`.
