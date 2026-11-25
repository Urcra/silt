# 0117: Conversations open at their end

- Status: accepted
- Date: 2026-11-24

## Context
Holding only the families in use (ADR 0116) left a long conversation an agent works in as costly as before. One with 44,000 deltas and hundreds of screenshots in its agent's tool results held 466 MB of compressed log, about 650 MB of text once decoded, and a Windows daemon with two such chats open held 2.1 GB.

- **The web UI read a thread from its first entry on,** drawing each page as it came. A long thread replayed: text filled in from the top, background tasks came and went as the pages naming their start and end arrived, and the person scrolled down to its end.
- **The daemon read a viewed thread's whole history again at each change,** parsing every line of its agents' transcripts, compared it whole with the reading before, and kept it uncut, images and all, beside the log.
- **The thread list's summaries and the merge requests read the family's whole log at each change too,** and so did a push and a thread's last commit. The log an agent worked in was never let go of.
- **Switching back to a thread let go of after five minutes** waited for its family to load and its history to be read again.

## Decision
**A thread opens at its end.** A page of history can ask for its last entries (`tail`) before an entry or the end (`before`), from the start of the turn they begin in, so it opens on a prompt. Every page carries the cut entries before it with news of a task (`tasks`). The web UI reads the last turns first and lands at the end. It reads earlier pages as the person scrolls up, keeping the rows in view still, and reads back to an entry a link names. Its tasks are folded from the page's news and the entries held. The daemon and the server both answer.

**A thread's history is read on from where it was.** What the log adds is read with the transcripts' readers kept from the last reading, and the turns with it. Only a rewind, a merge or a fork, which change what came before, read it again whole. The entries are kept cut, as a page carries them. An entry asked for whole reads its delta from the store again, alone; one that reads otherwise alone comes from the history read again whole.

**The views across families fold what the log added.** Each thread's summary and the merge requests' deltas are folded on from where they were, one code path for a first reading and for what came after. Pushes and a thread's last commit read what came since, and a snapshot keeps the ids and fingerprints it took. Nothing an active chat does on each change reads its log whole, so the log is let go of when unused (ADR 0116) while the agent works.

**A family is held for 30 minutes unused,** not five: its files and log are let go of after five, so holding it costs little. A page of a history kept as its family is now is answered without loading the family.

## Consequences
- Opening a long thread draws its last turns at once, at its end; nothing replays.
- A conversation's screenshots stay in the store, and in memory only while a page or a whole entry shows them.
- A subagent whose transcript began before the entries held shows the part held until earlier pages are read.
- Reading a thread's history the first time after its family loads still reads its log whole, once.
