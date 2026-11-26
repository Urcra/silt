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

## GitHub Copilot CLI 1.0.116 (docs; the CLI is closed source)
- **Copilot Memory lives on GitHub, not on the machine.** Two stores: repository facts, shared with the repository's collaborators, not copied to forks, each cited to code that Copilot checks against the current branch; and the person's own preferences, which follow them across repositories ([concepts](https://docs.github.com/en/copilot/concepts/agents/copilot-memory)).
  - A memory expires 90 days after it was made or last upvoted (7 to 180 for repository facts, set by an org). Asking Copilot to forget one only downvotes it. "Copilot Dreaming" merges duplicates in the background and proposes changes to checked-in instructions as pull requests.
  - The root agent has `store_memory` and `vote_memory`; subagents only `read_memories`. The CLI's approval prompt for `store_memory` names the scope.
  - Interactive sessions follow the person's GitHub setting; `/memory [show|on|off]` changes it on that machine; programmatic runs need `--enable-memory`; `"memory": false` in the CLI's settings turns it off. Business and Enterprise plans have it off until an owner turns it on.
- **Format,** as `copilot memories import FILE [--dry-run] [--on-conflict skip] [--output json]` takes it, one JSON object a line ([create-memories](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/copilot-memory/create-memories)):
  ```json
  {"type":"memory","version":1,"externalId":"repo-testing-1","scope":{"type":"repository","repository":{"host":"github.com","owner":"octo-org","name":"octo-repo"}},"subject":"testing","fact":"Run npm test before opening a pull request.","reason":"The repository requires this validation workflow.","citations":[{"type":"file","path":"CONTRIBUTING.md","lineStart":10,"lineEnd":12}]}
  ```
  A preference has `"scope":{"type":"user"}` and `citations: [{"type":"text","text":"User input: …"}]`.
- **Reading them back:** no public REST or GraphQL API. github.com/settings/copilot/memory lists the person's, a repository's Settings › Copilot › Memory its facts, and org admins can export JSONL. So Silt can **write** to Copilot's memory (import) but can only **read** what it sees in sessions: the `store_memory` calls in a captured transcript.
- **Rules files:** `$COPILOT_HOME/copilot-instructions.md` and `instructions/**/*.instructions.md`, `.github/copilot-instructions.md`, `.github/instructions/**/*.instructions.md` (`applyTo` globs), and `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, `GEMINI.md` from the repository root to the cwd; `COPILOT_CUSTOM_INSTRUCTIONS_DIRS` adds folders. `/instructions` lists them.
- VS Code's Copilot differs: an experimental repository-local `.github/copilot-memories.jsonl` (`chat.copilot.memory.local.enabled`).

## OpenCode 2.4.2 (source, anomalyco/opencode@fc31a48)
- **No memory tool.** Memory is rules files:
  - `$XDG_CONFIG_HOME/opencode/AGENTS.md` (or `OPENCODE_CONFIG_DIR`), else `~/.claude/CLAUDE.md`;
  - `AGENTS.md` in each folder from the cwd up to the project root, else that folder's `CLAUDE.md`; reading a file or `cd` loads the `AGENTS.md` near it;
  - `instructions` in `opencode.json`: paths, globs and URLs.
  - `OPENCODE_DISABLE_CLAUDE_CODE_PROMPT=1` stops it reading `CLAUDE.md`.
- **Experimental `home_memory`** (`OPENCODE_FLAGS=home_memory`, off in releases): only for sessions in `$HOME`, `$XDG_DATA_HOME/opencode/memory/<project id>/{memory,docs,scripts}/`, offered but not loaded, and rewritten by a daily "Memory maintenance" session from that day's transcripts (`packages/core/src/plugin/home-memory.ts`).
- People add memory with plugins: `opencode-mem` (libSQL with vectors under `~/.opencode-mem/`), `opencode-supermemory` (a hosted service, keyed by the git origin, shared with Claude Code and Codex).

## No shared convention yet
Nothing does for memory what `AGENTS.md` does for rules or `.agents/skills` for skills. The closest common ground is MCP's reference `memory` server (modelcontextprotocol/servers@c799c85, `src/memory/index.ts`): a JSONL knowledge graph at `MEMORY_FILE_PATH`, with tools `create_entities`, `add_observations`, `search_nodes` and others.
```json
{"type":"entity","name":"Ada","entityType":"person","observations":["prefers tabs"]}
{"type":"relation","from":"Ada","to":"Silt","relationType":"works_on"}
```
