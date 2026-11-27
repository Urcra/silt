# Agent CLI session formats (M6 research)

Notes on where each agent CLI keeps its sessions and how Silt captures and restores them (ADR 0007). Each section names the version it was checked against. Sources are the agents' own open-source code and runs against scratch homes without logging in.

## Gemini CLI 0.64.0 (compared with 0.43.0)
- **Location:** `$GEMINI_CLI_HOME or ~`/.gemini/tmp/<slug>/chats/session-<YYYY-MM-DDTHH-mm>-<id8>.jsonl`.
  - The slug comes from the registry `~/.gemini/projects.json` (`{"projects":{"<abs path>":"<slug>"}}`). It is the cwd basename, lowercased, with runs of non-`[a-z0-9]` turned into `-`; collisions get `-1`, `-2`.
  - `tmp/<slug>/.project_root` holds the owning path. A slug directory without that marker is claimed by the first project that wants it.
  - Subagent sessions are written to `chats/<parentId>/<id>.jsonl`.
- **Format:** append-only JSONL. A reader folds the records in order:
  1. First line, metadata: `{sessionId, projectHash, startTime, lastUpdated, kind:"main"|"subagent", directories?, summary?}`. `projectHash` is `sha256(projectRoot)` and is never validated.
  2. Message: `{id, timestamp, type:"user"|"gemini"|"info"|"error"|"warning", content: string|Part[], displayContent?, toolCalls?, thoughts?, tokens?, model?}`.
     - A message is re-appended under the same `id` when tokens or tool results arrive; the last copy wins.
     - A reply's tool calls are written only once the tools ran, so nothing in the file marks a tool starting. Silt ends a turn at a reply that has its tokens and asks for no tool, and a tool call read after that reopens the turn.
     - From 0.64, tool responses are also separate `user` messages whose content is `functionResponse` parts.
  3. `{"$set":{…}}` merges metadata.
  4. `{"$patch":{updates:[{id,content?,toolCalls?}], removeIds?, orderIds?}}` (0.64): updates, removals and reordering. Compaction removes the old ids this way, but the old lines stay in the file.
  5. `{"$rewindTo":"<id>"}` drops that message and everything after it.
  - Synthetic user messages that start with `<session_context>` or `<hook_context>` are not conversation.
- **Resume:** `--resume <id|index|latest>`, searching only the current project's `chats/` directory. The file name doesn't matter for `--resume <id>`.
  - `--session-id <[A-Za-z0-9_-]+>` picks the id of a new session; it fails if that id already exists.
  - `--session-file <path>` imports a file under a new random id.
  - **Restore recipe (verified):**
    1. Rewrite the paths in the file.
    2. Set the new `sessionId` on the metadata line.
    3. Write it as `tmp/<slug>/chats/session-<ts>-<id8>.jsonl`.
    4. Run `gemini --resume <id>`.
- **Hooks:**
  - **Events:** `SessionStart` (`source`: startup | resume | clear), `SessionEnd`, `BeforeAgent` (`prompt`), `AfterAgent`, `BeforeTool` / `AfterTool` (`tool_name`, `tool_input`, `tool_response`), `PreCompress`, `BeforeModel`, `AfterModel`, `Notification`.
  - **Stdin:** `{session_id, transcript_path, cwd, hook_event_name, timestamp}` plus the per-event fields.
  - **Output:** `hookSpecificOutput.additionalContext` from SessionStart, BeforeAgent and AfterTool is injected into the conversation.
  - **Trust:** hooks only run in trusted folders; `GEMINI_CLI_TRUST_WORKSPACE=true` trusts the folder.
  - **Resume quirk:** on resume, the `SessionStart` hook's `transcript_path` is a throwaway file; take the path from `BeforeAgent` on.
- **Per-run configuration:**
  - The system settings path env vars are ignored unless the settings file's directory is owned by root.
  - What works is an extension (`gemini-extension.json` with `mcpServers`, plus `hooks/hooks.json`), selected with `-e`, whose hooks add to the user's.
  - No flag adds an MCP server.
- **Rules and skills:**
  - **Rules:** `GEMINI.md` (global, upward to the git root, and subdirectories). `AGENTS.md` only when listed in `context.fileName`.
  - **Skills:** read from `~/.gemini/skills`, `~/.agents/skills`, `<project>/.gemini/skills` and `<project>/.agents/skills`.

## Antigravity CLI 1.5.8
Google moved most people from Gemini CLI to Antigravity CLI (`agy`) on 2026-06-18: Gemini CLI now serves only paid API keys and Gemini Code Assist Standard and Enterprise. Checked with a scratch home, `GEMINI_API_KEY` and `GOOGLE_GEMINI_BASE_URL` pointing at a mock of the Gemini API, and `--settings '{"modelProvider":"gemini"}'`, which takes agy past sign-in.
- **Layout:** `~/.gemini/antigravity-cli/` (agy ignores `GEMINI_CLI_HOME`; `--gemini_dir <dir>` moves the whole `.gemini`, and is hidden from `agy --help`).
  - `brain/<conversation id>/.system_generated/logs/transcript_full.jsonl`: the steps, one JSON line per finished step. `transcript.jsonl` beside it is a compact copy that cuts long fields (`truncated_fields`), and `chunks/` holds both again. A tool output too long for a step is in `steps/<n>/output.txt`, which the step names.
  - `conversations/<id>.db`: SQLite, tables `steps`, `trajectory_meta` and others whose rows are protobuf. This is what agy resumes from.
  - `conversation_summaries.db`: SQLite, one row per conversation, written as it starts: `conversation_id`, `workspace_uris` (a JSON list of `file://` URIs, percent-encoded, the cwd first), `parent_conversation_id` and `nesting_depth` for a subagent's, `status`, `app_data_dir`.
  - `cache/last_conversations.json` maps each cwd to its last conversation, but is written only as agy exits.
- **Steps:** `{step_index, source: USER_EXPLICIT|MODEL|SYSTEM, type, status: DONE|ERROR|INVALID…, created_at, content?, thinking?, tool_calls?, input_tokens?, output_tokens?, error?}`.
  - `USER_INPUT`: `content` wraps the prompt in `<USER_REQUEST>…</USER_REQUEST>`, followed by `<ADDITIONAL_METADATA>` and settings changes.
  - `PLANNER_RESPONSE`: the reply, its thinking, and `tool_calls: [{name, args}]`, with no call ids. It is written before the tools run. A reply that calls no tool ends the turn.
  - `GENERIC`: one per call, in order, once it ran. `content` starts with `Created At:` and `Completed At:` lines; a failed call has `status` `ERROR` or `INVALID` and an `error`.
  - `SYSTEM_MESSAGE`: notices, such as `Subagent <id> has gone idle. This was the subagent's most recent response: …`, after which the agent goes on by itself, and on resume, that background work was stopped.
  - **Background commands:** a `run_command` still running after `WaitMsBeforeAsync` answers at once with a `GENERIC` step of `status` `RUNNING`: `Tool is running as a background task with task id: <conversation>/task-<n>`, `Task Description: <command line>` and its log, `.system_generated/tasks/task-<n>.log`. Its end is a notice, `Task id "<id>" finished with result:` then `Command exited with code <n>.` and its output, after which the agent goes on by itself. `manage_task` with `kill` answers `Task "<id>" cancelled.`
- **Tools:** `read_file` and `write_file` and `edit_file` (`Path`), `run_command` (`CommandLine`, `toolSummary`, `WaitMsBeforeAsync`, `IsDaemon`), `search_web`, `invoke_subagent` (`Prompt`, `Role`, `TypeName`: `self`, `research`…), `ask_question`, and MCP servers' tools behind `call_mcp_tool`. `invoke_subagent` answers with the new conversation's id: `{"conversationId": …, "logAbsoluteUri": …}`.
- **Resume:** `--conversation <id>`, or `-c` for the cwd's last. It needs `conversations/<id>.db`: given only the transcript, or an unknown id, agy starts a new conversation without a word. A copied `.db` resumes in another home and from another cwd, and agy then appends only the new steps to the transcript. `-i <prompt>` starts interactively with a prompt; `-p` runs one prompt headless.
- **Per-run configuration:** none for MCP servers. `--gemini_dir` pointed at a directory linking everything of the real `.gemini` but `config/mcp_config.json` (`{"mcpServers": {name: {command, args}}}`) and `GEMINI.md` works: agy loads the MCP server and the rules from there, and writes its conversations through the linked `antigravity-cli`. `--settings` takes `settings.json` keys for one run.
- **Hooks:** `~/.gemini/config/hooks.json` or `.agents/hooks.json`, `{"<name>": {"<Event>": [{matcher, hooks: [{type: "command", command, timeout}]}]}}`. Events: `SessionStart`, `PreInvocation`, `PostInvocation`, `PreToolUse`, `PostToolUse`, `Stop`. Stdin has `conversationId`, `cwd`, `transcriptPath` (the `transcript_full.jsonl`), `workspacePaths`, `modelName`, and for tools `toolCall: {id, name, args}`; hooks run in `~/.gemini/config` with `ANTIGRAVITY_CONVERSATION_ID` set. `PreToolUse` must answer a `decision`, or the call is denied. Silt reads the transcript instead.
- **Interactive:** a first run asks for a colour scheme and the terms (`antigravity-cli/cache/onboarding.json`), and each new folder whether to trust it (`trustedWorkspaces` in `antigravity-cli/settings.json`); both are kept in the real store through a `--gemini_dir` overlay. Started with `-i <prompt>` in a folder it doesn't trust yet, agy makes the conversation before the question is answered and lists it with an empty `workspace_uris`; started without a prompt, or in a trusted folder, it lists the folder.
- **Rules and skills:** `GEMINI.md` and `AGENTS.md` in the workspace, `~/.gemini/GEMINI.md`; skills in `.agents/skills`, `~/.gemini/skills` and `~/.gemini/antigravity-cli/skills`.
- **ACP:** `agy acp` answers a v2 `initialize` with v1, with `loadSession`, `session/resume`, modes `default`, `accept-edits` and `plan`, every model of the account, and sign-in methods for a Google account, Gemini Enterprise, a Gemini API key and Agent Platform. To a client whose v1 `clientCapabilities.auth.terminal` is `true` it also lists `terminal-login` (`"type": "terminal", "args": ["--login"]`): `agy acp --login` prints `Open this link in a browser to log in with Google:`, the link, and `Then paste the authorization code, or the full URL of the page you land on, and press Enter:`, and a refused code ends it with `couldn't log in: …`. It reads a v2 `initialize` as v1, so it never sees v2's `capabilities`, and refuses v2's `auth.terminal: {}` outright (`cannot unmarshal object … of type bool`); a second, v1 `initialize` on the same connection is answered. The ACP registry's `antigravity-acp` is a different server: a Python one on Antigravity's SDK, where `agy acp` is Go inside the CLI. Since 1.5.0, `agy acp` takes another prompt into a running turn, which the registry's server holds until the turn ends (steering-and-backgrounding.md).

## Codex CLI 0.163.0
- **Layout:** `$CODEX_HOME` (default `~/.codex`; when set, the directory must already exist).
  - Rollouts are `sessions/YYYY/MM/DD/rollout-YYYY-MM-DDThh-mm-ss-<uuid>.jsonl`, with the date and time in local time.
  - `session_index.jsonl` only holds thread names.
  - `state_5.sqlite` holds a `threads` table (`rollout_path`, `cwd`, `history_mode`).
  - `thread-writer-locks/<id>.lock` is flocked while a session is live.
- **`codex resume <uuid>`** looks for the session in this order:
  1. the live writer;
  2. the SQLite `rollout_path`;
  3. a scan of filenames under `sessions/`.
  - It then repairs the database. **A file written into `sessions/` is enough** (verified with `thread/read` and `thread/resume`).
  - **Caveat:** a database row pointing at a different existing file with the same id wins, so a restore keeps the session's file name.
  - The TUI asks which directory to use when the recorded cwd differs from the current one, so a restore rewrites every cwd.
- **Line envelope:** `{"timestamp", "ordinal"?, "type", "payload"}`.
  - **History modes:**
    - New threads default to `paginated`: every line has an `ordinal`, there are no `user_message` / `agent_message` events, and `item_completed` events are written instead.
    - A file without `history_mode` is legacy and resumes as legacy, so restores and translations write legacy files.
  - **`session_meta`:**
    - Required: `id`, `timestamp`, `cwd`, `originator`, `cli_version`.
    - `source` should be `"cli"`, because the picker filters by source.
    - Optional: `model_provider`, `base_instructions`, `git`, `forked_from_id`, `history_mode`.
  - **`turn_context`:** `{turn_id?, cwd, approval_policy, sandbox_policy:{type}, model, effort?, summary?}`.
  - **`response_item` payloads:**
    - `message{role:user|assistant|developer, content:[input_text|output_text|input_image], phase?}`
    - `reasoning{summary:[{type:summary_text,text}], content?, encrypted_content}`
    - `function_call{name, arguments:string, call_id}` and `function_call_output{call_id, output: string|[…]}`
    - `custom_tool_call{call_id, name, input}` (freeform apply_patch) and `custom_tool_call_output`
    - `local_shell_call`, `web_search_call`, `compaction{encrypted_content}`
  - **`event_msg` payloads:**
    - Always written: `task_started|turn_started{turn_id}`, `task_complete|turn_complete{turn_id, last_agent_message}`, `turn_aborted`, `token_count`.
    - Legacy only: `user_message{message}`, `agent_message{message}`, `agent_reasoning{text}`, `context_compacted`.
    - Paginated only: `item_completed{item}`.
  - **The model context is rebuilt from `response_item` lines; the UI transcript from `event_msg` lines.** Write both.
  - **`compacted{message, replacement_history?}`:** resume starts from the newest one only when it has a `replacement_history` (plus `window_number` and resume metadata). Otherwise the whole file is replayed.
- **Absolute paths** appear in:
  - `session_meta.cwd`, `turn_context.cwd` and the sandbox writable roots;
  - shell `function_call.arguments.workdir` and apply_patch inputs;
  - `local_shell_call.action.working_directory`;
  - the `<environment_context><cwd>` user message;
  - `# AGENTS.md instructions for <dir>` messages.
- **Encrypted content** (`reasoning`, `compaction`, `encrypted_function_args`) is bound to the OpenAI account. Dropping `reasoning` items is safe; Codex only insists that tool calls and outputs pair up.
- **Per-run flags:**
  - MCP: `-c 'mcp_servers.silt.command="silt"' -c 'mcp_servers.silt.args=["mcp",…]'`.
  - Extra instructions: `-c developer_instructions="…"` adds a developer message.
- **Hooks (on by default):**
  - **Events:** SessionStart (`source` startup | resume | clear | compact | fork), UserPromptSubmit, PreToolUse, PostToolUse, PreCompact, PostCompact, Stop, SessionEnd.
  - **Stdin:** `session_id`, `transcript_path`, `cwd`, `hook_event_name`.
  - **Config:** `hooks.json` in the config folder or `[hooks]` in config.toml.
  - **Trust:** hooks you configure yourself need trust (`hooks.state.<key>.trusted_hash`) or `--dangerously-bypass-hook-trust`.
  - **`notify`:** `notify=["prog"]` gets `{"type":"agent-turn-complete","thread-id","turn-id","cwd","last-assistant-message"}` as its last argument.
- **Session id:** it can't be chosen at start. A new rollout is found by its `session_meta.cwd` under today's `sessions/` folder, and the file only appears once the first turn is persisted.
- **Natively read:**
  - **Rules:** `AGENTS.override.md` / `AGENTS.md` (global in `$CODEX_HOME`, then from the project root down to the cwd).
  - **Skills:** `~/.agents/skills`, `$CODEX_HOME/skills`, `<repo>/.codex/skills`, and `.agents/skills` from the root down to the cwd.

## Qudiks 0.0.8 (`fork-off`, 7b7e820)
[Qudiks](https://github.com/viterkim/qudiks) is a fork of Codex that adds GitHub Copilot, OpenAI API credits and OpenAI-compatible endpoints as providers (`--github`, `--codex`, `--openai-api`, `--endpoint <url>`).
- **Sessions** are Codex's rollouts, read and written by Codex's own crates: `sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, with the same `session_meta` (`originator` `codex_cli_rs`), so nothing in one says which of the two wrote it.
- **Home:** `$QUDIKS_HOME`, else `$CODEX_HOME`, else `~/.qudiks` (`codex-rs/utils/home-dir`). Project config stays `.codex/`, rules `AGENTS.md`, skills `.agents/skills`.
- **Launch:** the same CLI as Codex: `-c key=value`, `resume <id>`, `--yolo`. The installer writes `approval_policy = "never"` and `sandbox_mode = "danger-full-access"` into its config and installs `qudiks` as a launcher script that passes `-c` with both, and trusts the working tree, ahead of the person's arguments. Later `-c` settings win, so Silt's come after them.
- **ACP:** none of its own, but `codex-acp` (TypeScript since 2.x) drives `codex app-server` and runs whatever `CODEX_PATH` names. Every request codex-acp 2.7 sends is in Qudiks' app-server protocol. Qudiks' Copilot provider authenticates with a command (`qudiks login github-copilot token`) and doesn't require OpenAI auth, so `account/read` gives `requiresOpenaiAuth: false` and the adapter asks for no login. Not yet run end to end.

## GitHub Copilot CLI 1.0.100
- **Layout:** `$COPILOT_HOME` (default `~/.copilot`). Each session is `session-state/<uuid>/`, holding:
  - `events.jsonl`: append-only, except that `/rewind` truncates it;
  - `workspace.yaml`: `id`, `cwd`, `git_root`, `branch`, `created_at`, …;
  - checkpoints, `files/` and snapshots.
  - `session-store.db` is an index that resume doesn't need: a hand-copied session directory resumes and gets its row.
- **Events:** `{"type","data","id","timestamp","parentId"}`, chained by `parentId`. Streaming `*_delta` events are never written.
  - `session.start{sessionId, copilotVersion, selectedModel, context{cwd, gitRoot, branch}}`
  - `session.resume{context}`
  - `user.message{content, transformedContent, turnId}`
  - `system.message`
  - `assistant.turn_start|turn_end{turnId}`
  - `assistant.message{messageId, content, toolRequests[{toolCallId, name, arguments}], reasoningText?, encryptedContent?}`
  - `assistant.reasoning{content}`
  - `tool.execution_start{toolCallId, toolName, arguments}`
  - `tool.execution_complete{toolCallId, success, result{content}, error?}`
  - `hook.start|end`
  - `session.compaction_start|compaction_complete{summaryContent}`
  - `session.truncation`, `session.snapshot_rewind`, `session.context_changed`, `session.shutdown`
- **Resume** rebuilds the model history from `user.message.transformedContent`, `assistant.message.toolRequests` and `tool.execution_complete.result`.
- **Absolute paths:** `context.cwd` / `gitRoot`, hook inputs, `transcriptPath`, tool arguments and results, and `workspace.yaml` `cwd` / `git_root`. Resume changes into `workspace.yaml`'s `cwd`.
- **Ids:**
  - `--session-id <uuid>` names a new session or resumes an existing one.
  - `--resume <id|prefix>` and `--continue` resume.
  - **Restore (verified):** copy `events.jsonl` and `workspace.yaml` into `session-state/<new>/`, rewrite the id and cwd, then run `copilot --resume <new>`.
  - `copilot sessions import` reads a neutral JSONL (`session`, `message`, `tool-call`, `tool-result`) into a new random id.
- **Hooks:**
  - **Per run:** `--plugin-dir DIR`, with `DIR/.plugin/plugin.json` and `DIR/hooks/hooks.json` shaped `{"version":1,"hooks":{"<event>":[{"type":"command","bash":"…","timeoutSec":30}]}}`.
  - **Events:** `sessionStart{source}`, `userPromptSubmitted{prompt}`, `preToolUse{toolName, toolArgs}`, `postToolUse`, `agentStop{transcriptPath}`, `sessionEnd`. Each gets `sessionId`, `timestamp` and `cwd` on stdin.
  - **Context:** `{"additionalContext": …}` from `sessionStart` and `userPromptSubmitted` reaches the model.
- **MCP for one run:** `--additional-mcp-config '{"mcpServers":{"silt":{"type":"local","command":…,"args":[…],"tools":["*"]}}}'`.
- **Natively read:**
  - **Rules:** `.github/copilot-instructions.md`, `AGENTS.md` (and nested ones), `CLAUDE.md`, `GEMINI.md`, `.github/instructions/*.instructions.md`, `$COPILOT_HOME/copilot-instructions.md`.
  - **Skills:** `.github/skills`, `.agents/skills`, `.claude/skills`, `~/.copilot/skills`, `~/.agents/skills`, plus plugin skills.
- **Tests without a login:** `COPILOT_HOME=<tmp> COPILOT_OFFLINE=true COPILOT_PROVIDER_BASE_URL=<mock>/v1 COPILOT_MODEL=gpt-4.1`, with a mock that streams chat completions as SSE.

## OpenCode 1.15
- **Storage:** sessions live in SQLite (`$XDG_DATA_HOME/opencode/opencode.db`, tables `session`, `message` and `part` with JSON `data`). Older JSON storage is migrated into it on first run.
- **Stable interface:** `opencode export <id>` prints `{"info": session, "messages": [{"info": message, "parts": [part…]}]}`, and `opencode import <file>` loads that shape back.
  - **Part types:** `text`, `reasoning`, `tool` (`callID`, `tool`, `state{status, input, output}`), `step-start`, `step-finish`, `file`, `compaction`.
- **CLI:**
  - `opencode session list --format json` gives `id`, `directory`, `created` and `updated`.
  - `opencode --session <id>` continues a session; `--prompt` sets the first prompt.
- **Per-run config:** the `OPENCODE_CONFIG_CONTENT` env var holds a config JSON with `mcp` and `instructions` (file paths).
- **Natively read:**
  - **Rules:** `AGENTS.md` (with `CLAUDE.md` as a fallback).
  - **Skills:** `.opencode/skills`, `.claude/skills` and `.agents/skills`.

## Verified against the real CLIs (2026-10-20)
Silt was run with each real agent CLI, using scratch homes and no logins. Models were stubbed with local mock servers, Gemini's `--fake-responses`, or the real Claude Code in `-p` mode.

| Agent | Captured through Silt | Native fork / resume | Translated in |
|---|---|---|---|
| Claude Code 2.1 | yes (M2) | yes (M3) | from Codex: the real Claude answered from the translated history, tool calls included |
| Codex 0.163 | a real paginated rollout, discovered and recorded | new id, every id and cwd rewritten; `thread/resume` and `exec resume` work | from Claude: `thread/read` / `thread/resume` load it, and a resumed turn sends the history |
| Gemini CLI 0.64 | yes, after registering the project in the real home (Gemini renames `projects.json`, which replaced the overlay's link) | yes | from Claude, once `projectHash` is non-empty |
| Copilot CLI 1.0.100 | yes, edits credited to their tool call once edits wait for the late-flushed `events.jsonl` | yes | from Claude, once event ids are UUIDs and `session.start` has `copilotVersion` |
| Antigravity CLI 1.5.8 (2026-11-13) | a real conversation against a mock Gemini API: prompt, edit credited to its call, command | resumed with `--conversation` where agy keeps the `.db`; elsewhere primed | n/a: Antigravity starts primed |
| OpenCode 1.15 | a real session, through `opencode export` | `opencode import` of a fork with fresh session, message and part ids | n/a: OpenCode starts primed |

Findings that shaped the code:
- `opencode export` truncates its output at 81,920 bytes when stdout is a pipe, so Silt reads its output through a file.
- Codex writes what the user typed in `event_msg` `user_message` (legacy) or `item_completed` `UserMessage` (paginated). User-role response items also carry injected context, so the reader takes user text from the events.
- Gemini repeats the last turn's tool results on resume under `<message id>_response`; the reader skips those records.
