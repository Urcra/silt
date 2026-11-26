# Agents' memory: where each keeps it, and sharing it

Research for a memory viewer, and for sharing what agents remember across agents and machines. Each section names the version it was checked against and how: probed (a run against a scratch home, with `ANTHROPIC_BASE_URL` at a mock that keeps every request), read in the agent's source, or from its docs.

"Memory" here is what an agent writes down by itself to read in a later session. Rules files a person writes (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`) are where several agents put it, so they're covered where they are its store.

## Claude Code 2.1.332 (probed)
- **Auto memory:** `<config dir>/projects/<slug>/memory/` (`<config dir>` is `$CLAUDE_CONFIG_DIR`, else `~/.claude`), where the agent writes with its ordinary Write and Edit tools. Nothing else writes it.
  - `MEMORY.md` is an index, one line a memory: `- [Title](file.md) — one-line hook`, no frontmatter. Its first 200 lines are put in the first user message as a `<system-reminder>`: `Contents of <dir>/MEMORY.md (user's auto-memory, persists across conversations):`. Not in the system prompt.
  - Each memory is its own file, read only when the agent opens it:
    ```markdown
    ---
    name: feedback-testing
    description: one line, used to decide relevance later
    metadata:
      type: feedback        # user | feedback | project | reference
    ---
    Integration tests hit a real database.
    **Why:** a mocked test hid a broken migration.
    **How to apply:** any test touching storage. See [[project-db]].
    ```
  - The system prompt (`# auto memory`) tells the agent the folder, the four types, to keep `MEMORY.md` an index, to update rather than duplicate, to turn relative dates absolute, and to check a remembered path or flag still exists before relying on it.
- **Which folder a checkout gets.** `<slug>` is the project root with every non-alphanumeric character turned into `-`. The root is the *common* git dir's working tree, so worktrees share their clone's memory:

  | cwd | memory folder |
  | --- | --- |
  | a clone, `work/main` | `projects/-…-work-main/memory/` |
  | its `git worktree`, `work/wt` | `projects/-…-work-main/memory/` (the clone's) |
  | a Silt managed checkout, `work/main/.silt/checkouts/t1` (`--separate-git-dir`, alternates) | `projects/-…-work-main--silt-checkouts-t1/memory/` (its own) |

  **So every thread in a managed checkout starts with empty memory, and what it learns is never read again** once the thread ends: its checkout's slug is used by no other thread. Only `--here` threads share the clone's memory.
- **Moving or turning it off:** `--settings '{"autoMemoryDirectory": "<abs dir>"}'` moves it, and Claude Code creates the folder (probed). `{"autoMemoryEnabled": false}` drops the `# auto memory` section entirely (probed). Silt already passes `--settings` on every run (`locked_settings`), so it can point every thread of a project at one folder.
  - In a Claude Code cloud session (`CLAUDE_CODE_REMOTE` set) the section is absent: no auto memory.
- **Rules files:** `~/.claude/CLAUDE.md`, `CLAUDE.md` and `.claude/CLAUDE.md` from the cwd up to `/`, `.claude/rules/*.md`, `@path` imports. A managed checkout sits inside its clone, so it reads the clone's `CLAUDE.md` too, as well as its own.
- **Silt's sandbox** keeps `~/.claude/CLAUDE.md` read-only and lets a run write its own `projects/<slug>/` (`ProjectState::own`), so auto memory works sandboxed; a moved `autoMemoryDirectory` needs to be writable in the sandbox as well.
- **Capture:** a memory write is a Write or Edit tool call outside the checkout, so it's already in the thread's transcript, but not in its files.
