# 0135: Agents' memory follows the person, across agents and machines

- Status: accepted
- Date: 2026-11-26

## Context
Agents keep notes for later sessions, each its own way (docs/research/agent-memory.md):
- **Claude Code** keeps a `MEMORY.md` index and a file a memory, with frontmatter, in `~/.claude/projects/<slug>/memory/`. It keys the folder by the git root, so each managed checkout gets an empty folder of its own, and what a thread learns is lost when it ends (#279).
- **Gemini CLI** keeps a nearly identical layout in `~/.gemini/tmp/<slug>/memory/`, keyed by the cwd, so it has the same problem.
- **Codex and Qudiks** consolidate sessions into one store per home, in background passes, when enabled.
- **Copilot CLI** keeps its memory on GitHub, which can be written with `copilot memories import` but not read back.
- **OpenCode, Antigravity CLI and Silt's `local`** keep only rules files.

Each store sits on one machine and is read by one agent, and no screen shows what any of them remembers.

## Decision
**Memory is the person's own.** It follows them across their machines and agents, and never comes from another person: memory goes into prompts, so someone else's is untrusted input (ADR 0016). Forking someone's thread doesn't bring their memories. What a team shares goes in committed rules files, which are reviewed.

**One store per person and project, shared by every agent, unless the person turns sharing off.** The store keeps Claude Code's shape: an index plus a file a memory, with frontmatter.
- **Claude Code** reads and writes it natively, through `autoMemoryDirectory` in the `--settings` Silt passes. Every thread of a project, managed or `--here`, gets the same folder.
- **Gemini CLI** is pointed at it if its private memory can be (to probe). Otherwise it gets the same access as the agents below.
- **The other agents** get its index as they get the skills and rules they don't read natively, and MCP tools to search, read and add entries.
- **Turning sharing off** gives each agent back its own native store, with the folder kept per project as above. The viewer still shows every store.
- **Copilot's memory on GitHub** is written only when the person turns that on for a project.

**The store is a family of its own,** one per person and project, its files the memory files. The daemon bridges it to the agents' folders as it bridges a checkout to disk, through `silt_checkout::is_writable`. So concurrent edits on two machines merge as text, and a memory's history and search come as a thread's do.
- A project is matched by its Silt project, not its path.
- It syncs only to the person's own servers, redacted (ADR 0032).
- **Codex's own store isn't synced as files,** as its locks and baseline are per machine. Its memories are read for the viewer, and its edits go in as ad hoc notes.

**A memory viewer** lists a project's memories from every agent and the shared store: type, description, when each was last changed, and the thread that wrote it (Claude's `originSessionId`). It can edit and delete them, as each agent allows, and lists the rules files every agent reads beside them. Mockups come before code.

## Consequences
- Fixing #279 comes first and stands alone: it needs only the `--settings` change and the sandbox's write rule.
- Memory families add to a person's families. They are small, and they unload like the others (ADR 0116).
- A memory written on one machine reaches the agents of the others only after a sync, so an agent working on two machines at once can see stale memory.
- What a background pass writes, such as Claude's `autoDream` or Codex's consolidation, enters the family as edits with no tool call to credit.
