# 0051: Subagents and background tasks

- Status: accepted
- Date: 2026-10-28
- Amends: [ADR 0049](0049-the-agents-work-grouped-by-turn-or-by-run.md) (what stands on its own in a turn)
- Amended by: [ADR 0057](0057-silt-ends-what-agents-leave-running.md): Silt ends what agents leave running

## Context
Agents hand work to subagents and leave commands running in the background, and every agent records it in its own way:
- **Claude Code:**
  - a subagent's transcript is `subagents/agent-<id>.jsonl` beside the session;
  - the tool's structured result names the subagent (`toolUseResult.agentId`) or the background command (`backgroundTaskId`);
  - a `<task-notification>` naming the call reports how it ended. It arrives as a user line when Claude is idle, or as a `queued_command` attachment mid-turn.
- **Gemini CLI:**
  - a subagent's session is `chats/<session>/<agentId>.jsonl`, and the call names it as `agentId`;
  - `run_shell_command` with `is_background` answers with the process's PID and never reports its end.
- **Codex:**
  - a subagent is a rollout of its own, whose `session_meta` names `parent_thread_id`, and `spawn_agent` answers with its `agent_id`;
  - unified exec answers "Process running with session ID n", and a later `write_stdin` sees "Process exited with code n".
- **Copilot CLI:**
  - a subagent's events are written into the session's own `events.jsonl`, each tagged with `agentId`, between `subagent.started` (naming the call) and `subagent.completed` or `subagent.failed`;
  - a shell left running ends with a `system.notification`.
- **OpenCode:**
  - a subagent is a child session in its database;
  - the `task` call's metadata names it, and synthetic `<subagent …>` and `<shell …>` messages report how background work ended.
- **ACP v1**, as Claude's adapter speaks it, tags a subagent's updates with `_meta.claudeCode.parentToolUseId`. It passes the tool's structured result as `_meta.claudeCode.toolResponse`, and a background command's exit as `_meta.terminal_exit`, with no exit code.
- **ACP v2** (ADR 0050) runs each subagent in a session of its own:
  - a `subagent_update` names the child session, its type and its description, and later its end;
  - the parent's `session_message` to it carries its prompt, under the id of the call that started it;
  - the child's updates come on its own session, and its report comes back as a `session_message`.
  - The Agent call itself is never announced.
  - A command left in the background completes with "Command running in background with ID: …", and its terminal ends, with no exit code, once the command does.

Silt captured Claude's and Gemini's subagent files, but the history showed their entries as the thread's own: a subagent's prompt read as the person's and could start a turn row. Nothing showed what was still running, or how it ended.

## Decision
**One agent-independent record of news.** Each adapter reads its agent's records into `Entry::Task`:
- the agent's id for the task, and the call that started it when the record names it;
- its kind, `subagent` or `background`, when known;
- its status, `running`, `completed`, `failed` or `stopped`;
- a subagent's transcript, by the name its entries are marked with;
- what the agent said of how it went.

News comes in any order, and later news of the same task says more. The history shows each piece as a `task` entry with a `TaskView`.

**A subagent's entries are marked, not mixed in.** Every history entry from a subagent carries `side`, the name of its transcript. That is the side file for agents that keep one. For agents that write the subagent into the session's own file (Copilot, ACP), a reader names it per record (`Read.side`). `silt_agents::history::HistoryReader` reads a log's lines for both serving modes. When a subagent's first entry comes before the session's own news has said which call started it, as with a subagent that is still running, the reader ties it to a waiting subagent call:
1. the one named like the transcript;
2. else the one that gave it its first prompt;
3. else the oldest.

**The thread's own conversation leaves subagents out.** Turns, titles, first prompts, `silt show`, handoff to another agent and merge summaries read only the session's own file. Search and `silt_history_read` cover both, with each subagent entry marked.

**In the web UI, a call that started a subagent or a background command stands on its own** as a card. The card shows:
- its kind (the subagent's type, or "In the background");
- what it is;
- how long it took or has run;
- how it stands.

Opened, a subagent's card shows the subagent's own transcript and what it reported back. A background command's card shows the command, what it answered when it started, and what was said when it ended. While a task runs, its card stays outside the finished turn's "Worked for" fold. It then folds with the rest of the work.

**What is still running shows above the composer**, each line a link to its card. That holds only while the session may still run it: the web UI's agent runs here, or the thread did something in the last hour. Silt can't see whether a CLI session is still alive. A task that never reported back shows as "no word since" on its card rather than as running for ever.

**Over ACP v1, Copilot's subagent is read from its task call.** Copilot announces the call with an `agent_type` in its input, tags the subagent's tool calls with `_meta["github.com/copilot"].agentId`, and leaves its words untagged. The first tagged update ties that id to the oldest subagent call not yet tied to one. While Copilot waits on a subagent, its untagged words are the subagent's. The call's completion ends the task, and its result is the report. Claude's adapter on v1 sends a subagent's own steps only to clients that declare the `subagents` capability. Silt's v1 `initialize` must say what its v2 one does, so a v1 Claude subagent shows its status and report but not its steps.

**Over ACP v1, a shell Copilot detached is read from its answers.** "<command started in detached background with shellId: N>" starts the task, and "<detached command with shellId: N completed with exit code C>", when Copilot reads it later, ends it. The news is the starting call's.

**Over ACP v2, Silt records a subagent as v1 shows it.** The parent's prompt to the child becomes an `Agent` call named by that message's id, with the child's type and description as its input. The child's updates are recorded inside that call, its end becomes task news, and its report becomes the call's result. A background command's end, with no exit code, counts as completed, on v1 too.

**Codex subagent rollouts are their parent's side transcripts.** They are no longer discovered or imported as sessions of their own.

## Consequences
- What a subagent did is readable where it was asked for, and searchable, without crowding the thread or being taken for the person's words.
- Agents that never report a background command's end (Gemini, or Codex's legacy rollouts until something reads the exit) show it as running until the hour passes, then as "no word since".
- OpenCode's subagents show as cards with their status and report, but not their transcripts: those live in OpenCode's database as other sessions, which the snapshot capture doesn't read yet.
- Copilot's background shells are known only once they end, as its records don't name the shell when it starts. They show as news without a card.
- The protocol is unchanged: `side` and `task` are new optional fields of the views, and the log keeps the agents' own lines.
