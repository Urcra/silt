# History and search

Silt keeps everything a thread did: every message, thinking, tool call and file edit, including what the agent compacted out of its own context. This page covers reading it back, searching it, linking to it, and taking a thread back to an earlier turn.

## Reading a thread
```sh
silt log                 # the threads this machine has
silt show <thread>       # its conversation, and the files each tool call changed
silt history <thread>    # its full history as numbered entries, across compactions and forks
silt diff <thread>       # what it changed
```

`silt history` pages with `--offset` and `--limit`. Its entry numbers are what `silt comment --entry` takes.

`silt diff` compares from the thread's start by default. Two options change that:
- **`--turn N`** shows only what that turn changed.
- **`--base`** picks what to compare with:
  - `merge-base`: the fork's source;
  - `last-commit` or `last-turn`;
  - `commit:<sha>`;
  - `main` or `branch:<name>`, as git has it now.

**`--hide-git`** leaves out what git itself rewrote, such as a `git pull`, `git checkout`, rebase, reset or stash the agent ran, or you ran between turns, so only the agent's own edits show. `silt comment --hide-git` numbers lines as that diff does. A commit isn't counted, as it commits files the agent wrote. The diff then may not match the disk.

**`--hide-others`** leaves out the edits of other agents that worked in the same clone at the same time, which the thread records as theirs ([Two agents in one clone](agents.md#two-agents-in-one-clone)). An edit that may have been this thread's agent's stays. `silt comment --hide-others` numbers lines as that diff does.

`silt open` shows all of it in the browser: the thread's turns, with the files each changed, and a changes panel with the same bases.

## Searching
```sh
silt search "token refresh"
silt search retry --file src/net --since 7d --status active
```

When this machine is signed in to a server, search covers every thread you can see there, your team's included. Otherwise it searches what this machine has. You can narrow it down by:
- `--repo`, `--author` and `--branch`;
- `--status`: `active`, `idle` or `merged`;
- `--file`: a file, or anything under a directory;
- `--since` and `--until`: a date, or a number of days ago, such as `7d`.

Hits are grouped by thread, with the turn they're from and whether they came before the agent compacted its context. The web UI's search shows the filters as chips.

To see who else is working on something right now:

```sh
silt in-flight src/auth.rs
```

It lists the other threads changing that file or directory in this repository, with their owner and when they last worked on it.

## What the agent can read
Agents run through Silt get its history tools over MCP, and the tools that tell them about each other ([Several agents in one project](agents.md#several-agents-in-one-project)):

| Tool | What it reads |
|---|---|
| `silt_history_search` | This thread and the threads it was forked from |
| `silt_history_read` | A page of the full conversation, including what was compacted away |
| `silt_file_history` | The prompts and tool calls that changed a file |
| `silt_thread_search` | Every thread this machine can see |
| `silt_fork_read` | Another thread's conversation, such as a fork or its source |
| `silt_in_flight` | Other threads changing a path |
| `silt_agents` | The other agents working in this project now |
| `silt_message` | Sends one of those agents a message; the only tool that changes anything |

After a compaction, Silt reminds the agent that these tools still reach its earlier history.

## The graph
The web UI's Graph page draws every thread's turns, commits, forks, merge requests and merges, newest first, in lanes like `git log --graph`. Narrow it to your own threads ("Mine") or to one kind, and pick a row to go to its thread and turn. Served by the server, it shows every thread you can see there.

## Commits and turns
A thread's page lists the commits it made, each with the turn it came in. To go the other way, from a commit to the thread and turn that made it:

```sh
silt log --commit 3f2a9c1
```

Silt doesn't add anything to commit messages; the link is kept in its own history.

## Going back to a turn
To take a thread's files back to how a turn left them:

```sh
silt revert --turn 4
```

The web UI does the same with "revert to turn N" on each turn. The later turns stay in the history, greyed out as rewound, so nothing is lost. An agent started afterwards picks up from that turn, and agents reading the history see what was rewound. To keep the later work going and try again from turn 4 as well, fork from it instead: `silt fork --turn 4 <thread>` (see [Forks and merge requests](forks-and-merge-requests.md)).

## Links
"copy link" next to a turn, a comment, a file or a merge request's file copies a link that opens the page scrolled to it, highlighted:
- **A file's link** keeps showing the version it was copied at. Click line numbers to pick lines, and shift-click for a range.
- **On a machine signed in to a server**, links open in the server's hosted app, so a teammate can follow them. The app shows files through one of your machines that's online.

The design is in [ADR 0011](../adr/0011-history-tools-search-and-web-ui.md), [ADR 0029](../adr/0029-fork-from-and-revert-to-a-turn.md), [ADR 0030](../adr/0030-commits-and-turns.md) and [ADR 0031](../adr/0031-permalinks.md).
