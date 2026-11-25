# 0101: The web UI under heavy load

- Status: accepted
- Date: 2026-11-18
- Amends: [ADR 0021](0021-browser-event-bus.md) (what a thread event fetches again), [ADR 0034](0034-performance-budgets.md) (two budgets for the web UI)

## Context
`web.thread` (ADR 0034) times a long thread's first page, and that page was fast. Everything after it was not. Measured in Chromium on a release build, the thread had 400 turns and 7,800 entries: long Markdown replies, a read's output in every step, a screenshot every 25 turns, and 300 files changed.
- **Scrolling stalled.** 150 wheel notches blocked the page for 13 s, with frames up to 480 ms. Every scroll offset drew the whole thread page, the diff panel beside it included, and parsed each row's Markdown again.
- **Every change read everything again.** One change to the open thread, such as a comment or a recorded line, read its whole history again: 54 MB. One reply streamed meant 160–220 MB.
- **Streaming ran at 9 fps.** Each word sent the agent's whole status, with everything said so far. A long reply was 37 MB over the socket, and each word parsed all of it as Markdown.
- **Typing lagged.** Each key drew the thread page and sent a presence request: 40–110 ms a key, and up to 450 ms with a large diff in the panel.
- **A large diff froze the page.** Within the "read whole" limits (300 files, 2 MB), a diff was drawn whole. That was 200,000 elements on opening and half a million once scrolled, a 3.8 s freeze and a 289 MB heap.
- **Pages were heavy.** The history carried every image as base64 and every tool's output whole: 31 MB of JSON for that one thread.

## Decision
**A history is read since a version.**
- A page of history names its `version`, the family's log length it was read at, and where it starts (`from`).
- A request may name the version it holds (`known`). The daemon keeps where each reading of a thread's history first differed from the one before (`KeptHistory`, the last 64 readings). The page then starts at the earliest entry that changed since, or at 0 for a version it doesn't know.
- The web UI keeps the history it read (`historyLoad.ts`). On a change it reads only from there, and entries that didn't change keep their objects, so rows drawn from them aren't drawn again.
- A server's pages name no version and are read whole, as before. The thread's turns are kept with its history and worked out again only when it changes.

**A page of history leaves out what's heavy until it's shown.**
- `cut_for_page` keeps a tool's output only from its last 4 KB, where a command's summary and a test run's result are. It leaves an image's data out. Either way it says what it left out (`cut`: the whole text's lines and bytes).
- The CLI and the history tools still read entries whole. The web UI asks for `whole` when an entry is shown: a thumbnail drawn, a step opened, or all of an output asked for.

**Words stream as words.**
- The daemon sends a word only as its `agent_text` event. It sends the agent's whole status when the state changes, and at most once a second while words stream, which also mends what a page missed.
- The page adds the words up, applies the events it hears once a frame, and parses a streaming reply block by block. Only the block being written parses again.

**The thread page draws what changed.**
- The history's rows are their own memoized component, kept as the same objects while their entries are. Scrolling draws the list, not the page.
- The composer keeps its draft. It tells the others what's typed once typing pauses, and the page draws again only when who's here changes.
- What a finished turn changed is fetched under a key of its own, which a thread's events don't fetch again.

**A large diff draws what's near the view.**
- Past 1,500 rows, a file's lines are drawn once they're within 1,200 px of the view, and given up beyond 4,000 px. Long hunks go 100 rows at a time. A part with a comment being written is kept.
- Highlighting runs 8 ms at a time between frames.

**Two budgets join ADR 0034's**, in `web/e2e/perf.spec.ts`, on its 2,000-message thread:

| Budget | What's measured | Limit |
|---|---|---|
| `web.scroll` | main-thread long tasks over 100 wheel notches up the thread | < 500 ms (1 s on a cloud machine) |
| `web.change` | history bytes read again after one change to the open thread | < 64 KB, on any machine |

## Consequences
- A history page's JSON gains `from`, `version` and each entry's `cut`. Older clients ignore them. This is no change to the sync protocol, so `PROTOCOL` stays.
- Opening a long thread reads about a fifth of what it did: 19 MB rather than 63 MB on the thread above. Going back to one the page holds reads almost nothing.
- Find in page only finds the lines a large diff has drawn, as it already did for rows of a long thread.
- A browser that misses a word sees it within a second, when the next whole status comes.
