# 0049: The agent's work, grouped by turn or by run

- Status: accepted
- Date: 2026-10-27
- Amends: [ADR 0044](0044-the-standard-agent-workspace.md) (how a thread at work shows tool calls, edits and thinking)
- Amended by: [ADR 0051](0051-subagents-and-background-tasks.md): Subagents and background tasks
- Amended by: [ADR 0107](0107-text-and-layout.md): Text and layout

## Context
ADR 0044 drew a thread at work as tool calls grouped with the running one's output tail, edits as diff cards and thinking folded with how long it took. In practice every command stood on its own card, so a turn that ran a dozen commands became a wall of one-line cards. Each thought between two commands was its own "Thought it through" row, and the agent's words to the person were lost among them. The running command's output also streamed into the thread under its card.

The durations were missing too. A thought was timed by the next entry in its fold, and with commands outside the folds that entry was usually missing. Agents spoken to over ACP, such as Copilot, had no time for their thoughts at all: a thought was recorded only once it ended.

## Decision
**A run is the agent's work between two things it said to the person.** Commands, tool calls, edits and thoughts all join it, in the order taken. It shows as one closed card:
- the card's line counts what the run did, what changed things first ("Edited 2 files, ran 3 commands"), at most three kinds and then "N other steps";
- a run of one step names that step instead, a command by what the agent said it was for ("Run the blob tests");
- "thought for 12s" follows, when its thoughts were timed;
- inside, each thought keeps its own look, a rule beside it, as "Thought for 4s", or "Thought" when untimed;
- each command leads with what it was for, then the command, muted, then how it ended;
- an edit is a row that opens to its diff.

**What needs the person stands outside a run.** A question, a proposed plan and a command that failed each stand on their own.

**A running step shows under its run's line** with the headline of the thought that led to it, in the present tense ("Editing src/main.rs"). A running command says how long it has run ("running · 0:42"). Its output stays in the step, streaming there when opened, rather than in the thread. Silt doesn't try to read how far a command has got from its output: few commands say so in a form worth parsing.

**A finished turn can fold whole.** Grouped by turn, which is the default, a finished turn shows the prompt, one line ("Worked for 42s · Edited 2 files, ran 3 commands, 5 other steps · 1 failed on the way"), the questions and plans the person answered, and the agent's last reply. Grouped by run, every run shows. A turn still running, grouped by turn, shows what the agent said and one activity card for the work since it last wrote: what that work did, the thought that led to the step under way, the last three steps and the running one, with the rest a click away. Questions, plans and the status line above the composer stay outside it. The grouping is chosen in the thread's More menu, apart from the detail: Normal keeps every fold closed, and Verbose opens them all. The former Compact detail is grouping by turn.

**Shift+T opens every thought, and closes them again.** It is in the palette as well.

**An ACP thought keeps when it began.** The daemon stamps the `thought` record with the time of its first words, and the entry after it gives when it ended.

## Consequences
- A busy turn reads as the agent's words with a line of work between them, and a finished one as its prompt, one line and its answer.
- Failures and decisions can't hide in a fold.
- Grouping by run and by turn are both available, since watching a turn live and coming back to it later want different things.
- The live output of a command is one click away rather than in the thread; its line says only that it runs and for how long.
- Threads recorded before this change keep thoughts without a start time, which show as "Thought".
