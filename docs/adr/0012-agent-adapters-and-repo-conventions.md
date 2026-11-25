# 0012: Agent adapters, cross-agent handoff and repo conventions

- Status: accepted
- Date: 2026-10-20

## Context
ADR 0007 planned one adapter per agent CLI, with Claude Code first. M6 adds Codex, Gemini CLI, OpenCode and GitHub Copilot CLI. It also adds the repository conventions agents share: `.agents/prepare`, `.agents/linked`, skills and rules files.

The research in docs/research/agents.md, checked against each real CLI, found the agents differ in four ways:
- **Session ids.** Claude, Gemini and Copilot accept an id at start. Codex and OpenCode choose their own.
- **Storage.** Four agents append to a JSONL file. OpenCode keeps sessions in SQLite, reachable through `opencode export`/`import`.
- **Hooks.** Only Claude Code takes per-run hooks without asking the user to trust them. Codex needs hook trust, Gemini needs a trusted folder, and Copilot's hooks don't name tool calls.
- **Per-run configuration.** Claude takes flags. Codex takes `-c`. Copilot takes flags and a plugin dir. OpenCode takes an environment variable. Gemini takes none at all.

## Decision
**Adapters.** `silt_agents::Agent` is the trait; `AGENTS` lists the five agents. Each adapter provides:
- **Ids:** how a new session gets its id (`Chosen` or `Discovered`) and how to `discover` it by cwd and start time;
- **Reading:** where its transcript and side files are, and a `Reader` that turns records into normalized entries and turn marks. Readers may keep state, because Gemini writes a message again as it grows;
- **Restore:** writing a recorded session back under a new path and id;
- **Translation:** an optional `translate` from normalized entries;
- **Launch:** `launch`, which builds the command from a `LaunchPlan`: Silt's MCP server, the hook command, instructions, skills and a scratch dir.

**Capture.**
- Tailing files is the default.
- Agents read by snapshot (OpenCode) are exported in the background, and each finished message or part is stored once, by key.
- A session whose id the agent picks waits as pending until it is discovered; its edits meanwhile count as the agent's.
- Turn marks come from Silt's hooks for Claude Code and from the transcript for the others.
- **Edit timing.** Transcripts trail the edits: Copilot flushes about 100 ms late, and Gemini writes a tool call only once it has finished. So edits in a checkout with such an agent running wait 400 ms before they are attributed, and a tool read as finished keeps owning edits for one second.

**Handoff between agents.** `silt resume|fork --agent X` picks the restore path:
1. **Same agent:** native restore, as ADR 0007 describes.
2. **Another agent with a translation:** a native session is written from the thread's normalized conversation.
   - Claude Code, Codex, Gemini and Copilot all have one, so Claude Code ↔ Codex (and any pair among the four) continues natively.
   - Tool calls and results become text, since the other agent has different tools, and thinking is dropped.
   - The translated lines are stored as a blob (`ConvOp::SessionTranslated`), so restoring that session again includes them without repeating them in the history.
   - `SessionContinued` links only sessions of the same agent.
3. **Otherwise (OpenCode as the target):** a new session starts primed with a rendered transcript. The transcript keeps the end of the conversation within 24k characters and points to `silt_history_read` for the rest.

**Per-run configuration.**
- **Claude Code:** `--mcp-config`, `--settings` (hooks), `--append-system-prompt` and `--plugin-dir`, the last holding the given skills.
- **Codex:** `-c mcp_servers.silt.*` and `-c developer_instructions`.
- **Copilot:** `--additional-mcp-config`, and a `--plugin-dir` holding the skills, a `sessionStart` hook that returns the instructions, and a `userPromptSubmitted` hook for Silt's notices.
- **OpenCode:** `OPENCODE_CONFIG_CONTENT`, carrying the MCP server and an instructions file.
- **Gemini:** an overlay `GEMINI_CLI_HOME`. It links everything from the real `~/.gemini` except `settings.json` (merged with Silt's MCP server), `GEMINI.md` (with the instructions appended) and `projects.json`. Gemini renames its registry into place, so Silt registers the project in the real home before launch.

**Repo conventions.**
- **Prepare and linked.** `.agents/prepare` and `.agents/linked` are read from the checkout; a file in `.silt/` replaces its `.agents/` counterpart.
  - On every mount of a managed checkout, the CLI links the listed git-ignored paths to `<data>/linked/<project>/` and adds them to the checkout's `info/exclude`.
  - The shared copy is seeded once from the source clone.
  - The CLI then runs `prepare` in the user's terminal, before the agent starts. Adopted clones (`--here`) are left alone.
- **Skills.**
  - Discovered from `.silt/skills`, then `.agents/skills`, then `~/.agents/skills`, in nested folders; earlier names win.
  - Frontmatter: `name`, `description`, `user-invocable`, `disable-model-invocation`, `metadata`.
- **Rules.**
  - The repository root's first non-empty rules file among `AGENT.md`, `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.rules`, `.cursorrules`, `.windsurfrules`, `.clinerules` and `.github/copilot-instructions.md`.
  - Nested `AGENT(S).md` files on the way to the cwd.
  - `~/.agents/AGENTS.md`.
- **Sharing them.** Each adapter declares what its agent reads natively, and Silt passes on only the rest:
  - **Rules** go in the session instructions.
  - **Skills** go to the agent natively where it takes them (Claude Code and Copilot plugins), otherwise into the instructions with the `silt_use_skill` tool.
  - **Silt's MCP server** offers `silt_skills` and `silt_use_skill` to every agent, and serves user-invocable skills as MCP prompts, which agents show as slash commands.

## Consequences
- Each adapter depends on its CLI's current format; docs/research/agents.md records the versions checked. Fixture tests cover each reader, and the end-to-end tests use fakes that write each format, including the late writes.
- **Turn and tool boundaries outside Claude Code are approximate.**
  - An edit lands under its tool call when the transcript names the call within about a second.
  - OpenCode's edits are only known to be the agent's.
  - An edit made by a long-running tool before the transcript mentions it counts for the turn.
- **Translation loses detail.** Another agent's tool calls become text.
  - The source session's side files are written under `<data>/side-files/<session>/`, and the translated text points there, so saved tool outputs it mentions stay readable.
  - The full record stays in Silt and can be read with the history tools.
- The Gemini overlay creates its own `installation_id` per run, and it drops settings Gemini changes during the run. Settings the user changes in `~/.gemini` are picked up on the next run.
