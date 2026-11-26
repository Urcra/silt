# Agents' memory: where each keeps it, and sharing it

Research for a memory viewer, and for sharing what agents remember across agents and machines. Each section names the version it was checked against and how: probed (a run against a scratch home, with `ANTHROPIC_BASE_URL` at a mock that keeps every request), read in the agent's source, or from its docs.

"Memory" here is what an agent writes down by itself to read in a later session. Rules files a person writes (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`) are where several agents put it, so they're covered where they are its store.

## At a glance
| Agent | Where | Format | Written by | Keyed by | In a managed checkout |
| --- | --- | --- | --- | --- | --- |
| Claude Code | `~/.claude/projects/<slug>/memory/` | `MEMORY.md` index + a file a memory, YAML frontmatter | the agent's Write/Edit, plus background extract and consolidate passes | git root, worktrees joined | **its own folder: lost after the thread** |
| Gemini CLI | `~/.gemini/tmp/<slug>/memory/`, `GEMINI.md`s | `MEMORY.md` index + sibling `*.md`, free-form | the agent's file tools; opt-in extraction to an inbox | cwd | **its own folder: lost after the thread** |
| Codex, Qudiks | `$CODEX_HOME/memories{,_v2}/` + SQLite | markdown with `key: value` headers | background passes only, when enabled | one per home | shared, but names local paths |
| Copilot CLI | GitHub's servers | JSON facts with citations | `store_memory` | repository, person | shared (server-side) |
| OpenCode | rules files only | `AGENTS.md` | the person | — | — |
| Antigravity CLI | rules files only | `AGENTS.md`, `.agents/rules/*.md` | `/learn`, approved | — | — |
| Silt's `local` | none | — | — | — | — |

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
  - The system prompt (`# auto memory`) tells the agent the folder, the four types, to keep `MEMORY.md` an index, to update rather than duplicate, to keep each file under 4 KB (recall shows only that much), to turn relative dates absolute, and to check a remembered path or flag still exists before relying on it.
  - Claude Code adds `modified` and `originSessionId` to `metadata` itself (read in the binary).
- **It writes memory by itself too** (read in the binary, `strings` of the Bun executable): `extractMemories`, a forked pass after a turn that wrote none (`CLAUDE_CODE_POST_TURN_MEMORY*`), and `autoDream`, a consolidation pass at most every 24 h and 5 sessions, holding `.consolidate-lock` in the folder (`autoDreamEnabled`). Both run as background subagents, so their writes may not show in the main transcript.
- **Which folder a checkout gets.** `<slug>` is the key with every character outside `[a-zA-Z0-9]` turned into `-` (cut at 200, with a hash). The key is `findCanonicalGitRoot` of the cwd: the first `.git` upward; a `.git` file pointing at `<common>/worktrees/<id>` whose back-link matches is keyed by the main working tree; any other `.git` file, as `--separate-git-dir` makes, is keyed by its own working tree. `CLAUDE_CODE_PROJECT_DIR_NAME` (with `CLAUDE_CONFIG_DIR` set) fixes the folder's name instead. So worktrees share their clone's memory, and Silt's checkouts don't:

  | cwd | memory folder |
  | --- | --- |
  | a clone, `work/main` | `projects/-…-work-main/memory/` |
  | its `git worktree`, `work/wt` | `projects/-…-work-main/memory/` (the clone's) |
  | a Silt managed checkout, `work/main/.silt/checkouts/t1` (`--separate-git-dir`, alternates) | `projects/-…-work-main--silt-checkouts-t1/memory/` (its own) |

  **So every thread in a managed checkout starts with empty memory, and what it learns is never read again** once the thread ends: its checkout's slug is used by no other thread. Only `--here` threads share the clone's memory.
- **Moving or turning it off:** `--settings '{"autoMemoryDirectory": "<abs dir>"}'` moves it, and Claude Code creates the folder (probed). `{"autoMemoryEnabled": false}` drops the `# auto memory` section entirely (probed). Silt already passes `--settings` on every run (`locked_settings`), so it can point every thread of a project at one folder.
  - `autoMemoryDirectory` is read from policy, `--settings`, then (in a trusted folder) local and project settings, then user settings; it must be absolute or start with `~/`. `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` turns memory off.
  - In a Claude Code cloud session (`CLAUDE_CODE_REMOTE` set) memory is off unless `CLAUDE_CODE_REMOTE_MEMORY_DIR` names a folder (probed: the section is absent here).
- **Subagents' memory:** a subagent with `memory: user | project | local` gets `<config dir>/agent-memory/<name>/`, `<cwd>/.claude/agent-memory/<name>/` or `<cwd>/.claude/agent-memory-local/<name>/`, laid out the same, its `MEMORY.md` in its own prompt. The `project` one is in the checkout, so Silt already captures and syncs it as a file of the thread.
- **Organisation memory:** the binary also has `memory_list`, `memory_read` and `memory_write` tools for a store held by Anthropic (`CLAUDE_CODE_MEMORY_API_BASE_URL`, `CLAUDE_CODE_DISABLE_ORG_MEMORY`). Undocumented; not tried.
- **Rules files:** managed policy (`/etc/claude-code/CLAUDE.md`), `~/.claude/CLAUDE.md` and `~/.claude/rules/`, `CLAUDE.md`, `.claude/CLAUDE.md` and `CLAUDE.local.md` from `/` down to the cwd, `.claude/rules/**/*.md` (`paths:` globs), `@path` imports four deep; subfolders' files load when the agent touches a file there. `AGENTS.md` only where no `CLAUDE.md` is (`instructionFiles`). A managed checkout sits inside its clone, so it reads the clone's `CLAUDE.md` too, as well as its own.
- **Hooks:** `InstructionsLoaded` fires for each rules file (`file_path`, `memory_type`, `load_reason`), not for auto memory. Nothing fires when memory is written: watch the folder, or a `PostToolUse` on Write and Edit under it.
- **Silt's sandbox** keeps `~/.claude/CLAUDE.md` read-only and lets a run write its own `projects/<slug>/` (`ProjectState::own`), so auto memory works sandboxed; a moved `autoMemoryDirectory` needs to be writable in the sandbox as well.
- **Capture:** a memory write is a Write or Edit tool call outside the checkout, so it's already in the thread's transcript, but not in its files.

## Gemini CLI 0.71.1 (source, google-gemini/gemini-cli@v0.71.1)
- **`save_memory` is gone** since 0.44. The system prompt says "There is no `save_memory` tool": the agent edits markdown with its ordinary `write_file` and `replace`, and puts each fact in exactly one of four places, asking when it fits several (`prompts/snippets.ts`):
  - `./GEMINI.md`, the team's, committed;
  - a subfolder's `GEMINI.md`, for that part of the project;
  - **private project memory,** `~/.gemini/tmp/<slug>/memory/MEMORY.md`, an index of one-line pointers to sibling `*.md` files, as Claude Code's (`config/storage.ts` `getProjectMemoryDir`; a `GEMINI.md` there is read when `MEMORY.md` is missing);
  - the person's `~/.gemini/GEMINI.md`.
- **The private memory is keyed by the cwd,** not the git root: `Storage` takes `targetDir`, the cwd, and `projects.json` gives it a slug from its basename ([agents.md](agents.md#gemini-cli-0640-compared-with-0430)). **So a Gemini thread in a managed checkout gets its own private memory too,** under a slug made from the thread's id.
- 0.43 and older: `save_memory {fact, scope: global | project}` appended `- <fact>` under `## Gemini Added Memories` in `~/.gemini/GEMINI.md`, after a diff to confirm. Those lines are still read as part of the file.
- **Loading** (`context/memoryContextManager.ts`, `utils/memoryDiscovery.ts`): `~/.gemini/GEMINI.md`, extensions' files, each `context.fileName` from every workspace folder up to `context.memoryBoundaryMarkers` (`.git` by default; project files only in a trusted folder), and the private `MEMORY.md`. Subfolders' files load when a file tool touches them. Each goes in the prompt in `<global_context>`, `<user_project_memory>`, `<extension_context>` or `<project_context>`, wrapped in `--- Context from: <path> ---`. `@path` imports five deep.
  - `context.fileName` can be `["AGENTS.md", "GEMINI.md"]`; the default is `GEMINI.md` alone.
- **`experimental.autoMemory`** (off by default): a background agent reads sessions idle 3 h with 10 user messages and writes unified diffs to `<memory dir>/.inbox/{private,global}/*.patch`, applied only once accepted in `/memory inbox`; never to a project's `GEMINI.md`. `.extraction.lock` and `.extraction-state.json` beside it (`services/memoryService.ts`).
- `/memory show | reload | list | inbox`. `GEMINI_CLI_HOME` moves `~/.gemini`. Under its Seatbelt sandbox `tmp/`, and so private memory, is in `~/.cache/.gemini`.

## Antigravity CLI 1.5.9 (docs; closed source)
- **No memory of its own, and no `/memory`.** Its docs say "Agents don't remember conversations". An open request asks for project memory (google-antigravity/antigravity-cli#1585).
- **`/learn`** is how something is kept: the agent drafts `learning_proposal.md`, the person comments and approves, and it writes a rule (`AGENTS.md`, `GEMINI.md`, `.agents/rules/<name>.md`, `~/.gemini/config/rules/<name>.md` and the like) or a skill (`.agents/skills/<name>/SKILL.md`) ([docs](https://antigravity.google/docs/commands/learn/)).
- **Rules it reads** ([docs](https://antigravity.google/docs/rules.md)): `AGENTS.md`, `GEMINI.md`, `.agents/{AGENTS,GEMINI}.md` and `.agents/rules/*.md` (frontmatter with a `trigger`) from the cwd up to the repository root; `~/.gemini/{,config/}{AGENTS,GEMINI}.md` and `~/.gemini/config/rules/*.md`. Each is cut at 24,000 bytes, and always-on rules share 20k tokens, past which the largest become pointers read on demand.
- The Antigravity IDE has "Knowledge Items" (`~/.gemini/antigravity/knowledge/<name>/` with `metadata.json`, `timestamps.json` and `artifacts/*.md`, distilled after conversations), according to community write-ups only; whether the CLI makes them is unverified, and `/context` lists a "Knowledge files" section. Not probed.

## Codex CLI 0.172.0 (source, openai/codex@rust-v0.172.0)
- **Memories are a feature, off by default:** `[features] memories = true` (`Feature::MemoryTool`, stable). Settings in `[memories]`: `generate_memories`, `use_memories`, `dedicated_tools`, `max_unused_days` (30), `max_rollout_age_days` (10), `min_rollout_idle_hours` (6), `extract_model`, `consolidation_model` and others (`config/src/types.rs`).
- **One store per `CODEX_HOME`, for every project.** Projects appear only as `cwd:` and `applies_to:` text inside the files, so it doesn't split by checkout, but what it says about a project names that machine's paths.
- **Two layouts.** A home with `memories/` keeps v1; one without gets v2 in `memories_v2/` (`core/src/config/mod.rs`).
  - v1, `$CODEX_HOME/memories/`: `memory_summary.md`, `MEMORY.md`, `raw_memories.md`, `rollout_summaries/<ts>-<hash4>[-slug].md`, `skills/<name>/SKILL.md`, `extensions/<ext>/instructions.md`, `extensions/ad_hoc/notes/*.md`, and a `.git/` that is the consolidation's baseline; `memories.lock` beside it.
  - v2, `memories_v2/`: `memory_summary.md` (under 10,000 bytes), `rollout_summaries/`, `extensions/`.
  - Markdown with `key: value` header lines, not YAML:
    ```
    thread_id: 019c…
    updated_at: 2026-11-20T10:30:00+00:00
    rollout_path: /home/u/.codex/sessions/…jsonl
    cwd: /home/u/repo
    git_branch: main

    <what the session did and learned>
    ```
    `memory_summary.md` starts with a line `v1`, then `## User Profile`, `## User preferences`, `## General Tips`, `## What's in Memory`. `MEMORY.md` is `# Task Group:` blocks with `scope:`, `applies_to: cwd=…; reuse_rule=…` and `## Task n`.
  - SQLite apart from the threads': `memories_1.sqlite` or `memories_v2_1.sqlite` (`stage1_outputs`, `jobs`, `consolidation_progress`), under `$CODEX_SQLITE_HOME`, else `$CODEX_HOME`. `state_5.sqlite`'s `threads.memory_mode` is `enabled`, `disabled` or `polluted`.
- **No agent writes them directly.** On the first turn of a root, non-ephemeral session, a background pass (`memories/write/src/`):
  1. extracts each idle rollout of an interactive session (not `exec`'s) into `stage1_outputs`, secrets redacted;
  2. has a sandboxed subagent, writing only the memory folder, consolidate them into the files, from a git diff against its baseline.
  "Remember" and "forget" become `extensions/ad_hoc/notes/*.md`, applied at the next consolidation.
- **Read back:** `memory_summary.md` goes in the developer instructions, cut at 2,500 tokens; the agent greps the rest with its own tools, or with `dedicated_tools`, `memories.list`, `memories.read`, `memories.search` and `memories.add_ad_hoc_note`. A reply may cite what it used in an `<oai-mem-citation>` block.
- **It already imports Claude Code's:** with `external_agent_memory_import` (in development) it copies `~/.claude/projects/<key>/memory/*.md` to `memories_v2/extensions/external_agent_import/resources/<key>/` with a `scope.json` (`external-agent-migration/src/`).
- **Concurrency:** jobs are leased in the memory database and consolidation holds `memories.lock`, both per machine; files are written in place. **A folder synced between machines would race**: two baselines, two leases, and summaries naming the other's `rollout_path`s.
- `/memories` toggles using and generating and resets them; `codex debug clear-memories`. The app server has experimental `thread/memory/read`, `memory/status`, `memory/reset`. codex-acp (pinned at 0.137) never starts the pass.
- **Rules files,** apart from memories: `$CODEX_HOME/AGENTS.override.md` or `AGENTS.md`, then every `AGENTS.md` (or `AGENTS.override.md`, or `project_doc_fallback_filenames`) from the project root down to the cwd, 32 KiB in all (`project_doc_max_bytes`).

## Qudiks 0.0.12 (source, `fork-off`@975a0f9)
Codex's memory code unchanged, under its home (`$QUDIKS_HOME`, `$CODEX_HOME`, `~/.qudiks`). Its own providers don't set the memory passes' models, so those still ask for OpenAI's unless `extract_model` and `consolidation_model` are set.

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

## Memory that isn't a project's
Every agent has somewhere to keep what holds in every project, but only Codex and Gemini CLI have their agent write there by itself:

| Agent | Person-wide | Written by |
| --- | --- | --- |
| Claude Code | `~/.claude/CLAUDE.md` and `~/.claude/rules/*.md`, which its binary calls "your user-level memory file"; `~/.claude/agent-memory/<name>/` for a subagent with `memory: user` | the person, or the agent when asked; the subagent itself |
| Gemini CLI | `~/.gemini/GEMINI.md`, one of the four places its prompt routes a fact to (0.43 and older: `## Gemini Added Memories` in it); `experimental.autoMemory` writes patches for it to `.inbox/global/` | the agent, by itself |
| Codex, Qudiks | the whole memory store, one per home; `memory_summary.md`'s `## User Profile` and `## User preferences` are about the person; `~/.codex/AGENTS.md` | the consolidation pass; the person |
| Copilot CLI | user-level preferences on GitHub, which follow the person across repositories | `store_memory` |
| OpenCode | `~/.config/opencode/AGENTS.md`; `home_memory`, only for sessions in `$HOME` | the person; a daily pass |
| Antigravity CLI | `~/.gemini/{,config/}{AGENTS,GEMINI}.md`, `~/.gemini/config/rules/` | `/learn`, approved |

**Claude Code's auto memory has no person-wide folder** (probed and read in the binary): its `user` kind, "the user's role, goals, responsibilities, and knowledge", goes in the project's folder like the rest. What it learns about you in one project is learned again in the next, and kept once per project.

So person-wide memory is mostly in rules files the person also writes, where an agent's lines and the person's sit together: `~/.gemini/GEMINI.md` mixes them, and only Gemini CLI 0.43's heading marks which are the agent's.

## What this means for Silt
Proposals for discussion, not decisions.

### 1. Keep a project's memory across its threads (a bug today)
A thread in a managed checkout loses what Claude Code and Gemini CLI remember. Silt can give every thread of a project the clone's own folder:
- **Claude Code:** add `autoMemoryDirectory` to the `--settings` Silt already passes, set to the folder the clone itself gets (`<config dir>/projects/<slug of the clone>/memory/`), so a thread, a `--here` thread and a plain `claude` in the clone share one memory. Silt's sandbox has to let the run write that folder. Probed to work.
- **Gemini CLI:** no setting moves private memory. Candidates, not probed: register the checkout's path under the clone's slug in `projects.json`, or link `tmp/<checkout slug>/memory` to the clone's.
- Threads already ended keep theirs in their own folders; a one-time merge into the clone's folder can bring them back.

### 2. A memory viewer
One reader per agent in `silt-agents`, a pure parse of each store into one shape, read by the daemon and served through `silt_proto::view` so the hosted app shows it too:
```rust
struct MemoryEntry {
    agent: AgentId,
    scope: MemoryScope,        // Person, Project(ProjectId), Subagent(String)
    path: RelativePath,        // within its store
    name: String,
    description: String,
    kind: Option<String>,      // Claude's user | feedback | project | reference
    body: String,
    modified: Option<Timestamp>,
    origin: Option<SessionId>, // Claude's originSessionId: links to the thread that wrote it
}
```
What each agent allows:
- **Claude Code, Gemini CLI:** read, edit and delete their files. Claude's `originSessionId` names the session, and so the Silt thread, that wrote a memory.
- **Codex, Qudiks:** read `memory_summary.md`, `MEMORY.md` and the rollout summaries. Edits go in as `extensions/ad_hoc/notes/`, since the consolidation pass rewrites the rest.
- **Copilot CLI:** only what threads captured, from their `store_memory` calls; the rest is on GitHub with no API to read it.
- **OpenCode, Antigravity CLI:** nothing beyond rules files. The viewer can list the rules files every agent reads (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, `.agents/rules/`, `.github/copilot-instructions.md`) beside memory, since they're where those agents keep what they know.

A memory write is already in the transcript as a tool call, so the viewer can also show, in a thread, what it remembered.

### 3. Across machines
Each agent's memory is a folder on one machine. Two ways to carry it:
- **As themes and design systems sync** ([ADR 0099](../adr/0099-themes-sync-ship-fonts-and-reach-the-window.md)): a file a row on the person's server, the newer copy winning, tombstones for removals. Simple, but two machines adding a line to `MEMORY.md` at once lose one.
- **As a family of its own:** one per person and project, its files the memory files, bridged to each agent's folder as a checkout is to disk. Concurrent edits merge as text, and history and search come with it. More work, and it fits Silt's focus on history (ADR 0006).

Either way:
- **A project is matched by its Silt project, not its path,** which differs between machines.
- **It's the person's own:** memory goes into prompts, so another person's is untrusted input (ADR 0016). Forking someone's thread doesn't bring their memories. What a team shares belongs in committed rules files, which go through review.
- **Redacted before it leaves** (ADR 0032), and file names from a server are checked before they're written.
- **Codex's store shouldn't be synced as files:** its locks and baseline are per machine. Show the other machines' read-only, or carry only ad hoc notes.

### 4. Across agents
No agent reads another's memory, except Codex's import of Claude Code's (in development). Claude Code's format, an index plus a file a memory with frontmatter, is the most complete, and Gemini CLI's is nearly the same, so Silt could keep one store of that shape per person and project:
- **Claude Code** writes it natively through `autoMemoryDirectory`.
- **Gemini CLI** might, if its private memory can be pointed at the same folder (to probe: does it read and keep Claude's frontmatter?).
- **The others** get its index the way they get the skills and rules they don't read natively (`conventions.rs`), and MCP tools to read, search and add entries: `silt_memory_search`, `silt_memory_read`, `silt_memory_write`.
- **Copilot:** optionally pushed with `copilot memories import`, which writes to GitHub, so only when asked.

### 5. Memory about the person, everywhere
Proposed, not yet decided: beside each project's store, Silt keeps one person-wide store of the same shape, synced as a family of its own, holding what applies in every project.
- **Claude Code** gets its index with the project's, through `--append-system-prompt` (or the ACP session's system prompt), since it has no second memory folder to point at. Its `user`-kind memories, which it files per project, belong here; Silt can move them, or offer to.
- **Gemini CLI, Codex and the rest** get it the way they get the project's store: injected, and through the MCP tools.
- **The viewer** shows it as its own entry, "Everywhere", above the projects, with Codex's `## User Profile` and `## User preferences` and the person's own rules files (`~/.claude/CLAUDE.md`, `~/.gemini/GEMINI.md`, `~/.codex/AGENTS.md`, `~/.config/opencode/AGENTS.md`) beside it.
- A memory can move between a project and everywhere either way.

### Decided
[ADR 0135](../adr/0135-agents-memory-follows-the-person.md): personal only; one store all agents share, which the person can turn off; synced as a family of its own; Copilot's GitHub memory written only when turned on.
