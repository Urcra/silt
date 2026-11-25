# 0067: The expanded review: a thread's changes across the page, seen files and reviewed turns

- Status: accepted
- Date: 2026-11-14
- Amends: [ADR 0055](0055-reviewing-a-threads-changes.md) (how a review is finished), [ADR 0024](0024-per-person-state.md) (what a person keeps of a thread), [ADR 0044](0044-the-standard-agent-workspace.md) (the thread's page)

## Context
The thread's Changes panel is the inspector beside the conversation, 480 px wide by default. Reviewing an agent's work there is cramped:
- a split diff wraps most lines;
- there is no list of the files to work through, and no way to say which were looked at;
- nothing says which turn wrote a hunk, so tying a change to the conversation that asked for it means scrolling both;
- the review bar above the diff takes room from it.

A merge request already has a Seen list and a wide diff. A thread's own work, still under way, has neither, and it changes as it's looked at, as the agent keeps writing.

## Decision
**The review expands over the conversation's column and the panel's, at `/t/<id>/review`.**
- The sidebar stays. The conversation, its composer and the agent's status don't show: the sidebar already says when the agent runs, and the review is about the code.
- A toolbar runs across the top: the turns, compare, Unified/Split, then what's pending and **Finish review**. Esc or **Conversation** in the header goes back, and ⇧D, the panel's expand button or **Review** over the composer comes in.
- The file list is the merge request's Seen list. The diff is split by default.
- The review is a desktop's. A phone keeps the panel's sheet, and the hosted app, which has no thread's diff, keeps the panel.
- Its keys take over the app's while it's open: j/k files, n/p hunks, ] the next file not seen, t the hunk's turn, c a comment, v seen, s the layout, / the filter, ⌘↵ Finish. Esc goes back. A view registers keys `over` the app's own; a menu or popover that takes Esc stops it there.

**It's always live.** It shows every change as it lands, not those there when it opened, as the panel does.

**Finish review is a popover**, in the panel and the review alike. It holds the note and the comments, Discard and Copy for a terminal, and asks what the agent gets:
- **Request changes**: one prompt, sent at once or queued for the end of the running turn;
- **Request changes, stopping the turn**: the daemon's steer, which cancels the turn and sends the review first;
- **Mark reviewed**: nothing to change, and the turn reviewed through is recorded;
- **Keep as comments**.

**Each hunk names the turn that wrote it.**
- The daemon answers `GET /api/threads/{id}/diff/turns` with, for each file of a diff, runs of new-side lines and the turn that last wrote them.
- A line's turn is how many of the thread's prompts came before its newest character. That counts turns as `turn=N` and the turn list do, inherited turns included.
- A line from git, or a hunk that only deleted, names none.
- A diff can cover a range of turns, `turn=a&to_turn=b`: from turn a's prompt to where turn b ended.

**Seen marks and the turn reviewed through are per person and synced (ADR 0024).**
- A thread's file is marked seen at a version of the thread: the mark keeps the BLAKE3 of the file's lines there and the version.
- The file shows as seen while its text is unchanged, and half-ticked once it changes. The lines new since are those whose newest character isn't in the version seen, so the review marks them.
- `ThreadState` gains `reviewed_turn`, merged as its other fields are. The turns picker starts after it.
- The server keeps a thread's marks beside a merge request's, in a table of its own, and `SeenMark` gains the version. An older server can't keep them, so `silt_proto::PROTOCOL` goes to 11.

## Consequences
- Finish review no longer opens inline in a bar above the diff; the panel shows "N pending" and the button instead.
- The review refetches its diff, the turns of its lines and its seen marks on each delta, as the panel does its diff. Turn runs are computed from the lines at the diff's head and the thread's prompt marks, not by reading the whole log.
- A seen mark from another device may name a version not synced here yet. Its lines then read as new until it arrives.
- Every daemon and the server update together across the protocol bump.
