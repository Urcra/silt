# 0054: The desktop UI review

- Status: accepted
- Date: 2026-10-29
- Amends: [ADR 0039](0039-the-web-ui-as-designed.md) (where the screens now differ from their boards), [ADR 0044](0044-the-standard-agent-workspace.md) (the sidebar's Needs you)
- Amended by: [ADR 0058](0058-new-threads-settings-outside-the-composer.md): New thread's project is the heading's picker again, and Local, Worktree and Cloud a segmented control under the composer

## Context
A review of every desktop screen rebuilt them from screenshots of a release build and proposed nine changes, each with a mockup in the Nous tokens. It also found defects:
- **A mock's label without its value.** `<Mock>` drops its value in builds without mocks, but the label beside it stayed. The project defaults' labels each sat one row off their controls, Devices showed Checkouts and Daemon with nothing beside them, and the graph's On showed a time. The gate builds with mocks on, so the screen references never showed it.
- **The merge request's header** cut the target thread's name under Open the fork.
- **Settings' sections** started at three different left edges, and narrow selects cut their values.

The proposals came from what the screens showed:
- every turn repeated its actions;
- the sidebar's Needs you grew inline buttons and uneven rows, and a thread's state was a dot in one of two colours;
- New thread spread its settings over four separate controls;
- Approve and merge was offered at full strength before a file was seen;
- Needs you was a column of cards the keyboard couldn't move through;
- All threads put each row's facts 450 px from its title;
- what notifies you was set in two places;
- an empty Inbox was three stacked boxes.

## Decision
**A thread:**
- The header's second line is project · branch · machine · agent, the id last.
- A turn's actions show while the pointer or focus is in the turn, and always on touch screens.
- The change bar is the composer's top edge.
- Compaction is a divider.
- The changes panel has one row: its tabs, then a compare menu beside Unified/Split.

**The sidebar:**
- Needs you is one block of equal rows, an icon per kind. Acting happens in the queue or the thread; Q opens the queue.
- A thread's mark is a shape per state (a turning ring while working, a filled ring while waiting, a dot once settled), with its state in words below the title.
- The footer counts what waits and runs.
- Search is an icon beside the workspace picker.

**New thread:**
- Everything is in the composer's row: the project, where it works (Local, Worktree, Cloud, a menu that explains each), the agent and the permissions.
- A line under the composer says where it will run.
- Pick up says each thread's state.

**A merge request:**
- The route, fork → target, has a row of its own under the header.
- The decision panel opens with what's left: files seen, whether both sides changed the same files, the guide.
- Approve and merge stays secondary until every file is seen, but is never disabled.
- The reason field appears with Request changes.

**Needs you** on a desktop is a queue:
- the waiting items grouped by kind on the left, the chosen one on the right with the thread's own answer, permission and plan controls;
- J/K move, A approves, O opens the thread;
- after each answer the next item takes its place.

Phones keep the cards.

**All threads** is a table: thread, state, agent, machine, sharing, merge request and last activity, sortable by thread and by activity. The filters, Needs you among them, are in the header, with counts.

**Settings:**
- One 760 px column from the same edge in every section, the nav grouped into Workspace and You.
- Each project opens to its defaults as label, control and help rows.
- Sync is part of Devices and sync; `/settings/sync` still opens it.
- `Select` draws its own chevron over a native `<select>`, which keeps the keyboard and the e2e specs' `selectOption`.

**Notifications:**
- Settings shows them by subject (agents, merge requests, conversation), with the inbox shown as always on and one browser switch per row.
- Rows that also land in Needs you say so.
- The Inbox links there instead of repeating the panel.

**An empty Inbox** says what will arrive and lists what was read lately.

**A mock takes its label with it.** `Mock` wraps a row's label and value, and `mockRows` drops a key-value row whole (docs/ui-mocks.md).

## Consequences
- The screen references were taken again. The boards of ADR 0039 still hold where this ADR doesn't say otherwise.
- **What the mockups showed that has no data yet**, so it's left out or said in other words:
  - a running time ("Running · 0:41");
  - a thread's machine by name, rather than here, the server or another device;
  - the agent's last line in Pick up;
  - a "Done" state for a thread;
  - who opened a merge request, in the queue;
  - a plan's step count.
- New thread's "on ⟨device⟩", which was a mock, is gone; the line under the composer names the device.
