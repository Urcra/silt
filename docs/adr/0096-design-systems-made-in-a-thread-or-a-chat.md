# 0096: Design systems made in a thread's own words, or in a chat

- Status: accepted
- Date: 2026-11-17
- Amends: [ADR 0088](0088-design-systems-in-settings.md) (the thread's panel, what Settings lists)

## Context
ADR 0088's panel beside a thread showed a design system only when New design system or Change it with an agent started the thread, as only those record which thread makes which system. People also just ask an agent for one, often in a chat. A chat has no project yet, but its design system is usually meant for a project started later. The agent followed the skill and wrote the system into the chat's own folder. There, the thread showed no panel, Settings didn't list it, and no project could use it.

The thread's file changes can't tell what it wrote either: Silt records nothing under `.silt/`, and a person's own systems are outside the checkout.

## Decision
- **Loading the skill marks the thread.** When a thread's agent loads the `design-system` skill, `silt mcp` tells the daemon, or without one writes it down itself. `design-skill-threads.json` keeps when each thread first did.
- **The panel shows what the thread wrote since.** That's the system in the thread's folder or the person's whose files were written last since that time. It counts as changed, not made, when it held files from before. A chat with no such mark shows the system its own folder last had written, as that folder is the chat's alone.
- **A chat's agent keeps it as the person's own.** The skill says so, and `silt mcp` adds where this machine keeps the person's systems, which the skill can only name by variable, and whether the run is a chat.
- **Settings lists chats' systems.** Each chat that keeps one in its folder gets a group, newest first. **Keep as mine**, on the system's page and beside the chat, copies it to the person's own, holding what a synced copy holds. A project then uses it as any of theirs, and it syncs.

## Consequences
- A design system shows beside its thread however it was asked for, once the agent loads the skill.
- A person's own system that another thread changes at the same time can show in a marked thread's panel. Both threads were working on systems, so it's rarely wrong.
- A chat's systems aren't offered to projects until kept as the person's own: a project's run looks in its repository and the person's folder only, so which of several chats' same-named systems applies is never in question.
