# 0024: Per-person state on the server

- Status: accepted
- Date: 2026-10-21
- Amended by: [ADR 0067](0067-the-expanded-review.md): The expanded review: a thread's changes across the page, seen files and reviewed turns

## Context
The web UI organizes threads per person ([ADR 0020](0020-next-generation-web-ui.md), plan sections E and G):
- pinned at the top;
- snoozed until a time, or until something happens;
- settled out of the way;
- unread since the person last looked.

Reviewers tick files of a merge request as seen. All of this belongs to one person, and it should follow them to every device.

The family log is the wrong home for it.
- **It's shared.** Everyone who can see a thread reads its log, and nobody else needs to know that someone snoozed it.
- **It's history.** Forks and merges carry it, and it would grow with every glance.
- **It can't be taken back.** A purge is the only way to remove a delta, so "I looked at this at 10:04" would stay forever.

## Decision
**Thread state** is one row per person and thread, `thread_state` on the server, holding a `silt_proto::ThreadState`:
- `pinned`;
- `snooze`: until when, and whether a new turn or merge request activity wakes it early;
- `settled`;
- `seen_ms`: when the person last opened the thread.

Each field carries when it was set (`Stamped { value, at_ms }`). A change is a state with only the fields it sets, and `ThreadState::merge` keeps the newer of each field, the larger value on a tie. Two devices changing different fields offline both keep their change, and merging is the same in any order, on the server and on each daemon. The row stores the merged state as JSON, so the rule lives in one place.

**Seen** is one row per person, merge request, file path and content hash.
- A file is seen at the version it had in the fork's head, so a later change to it makes it unseen without anyone resetting anything.
- The hash is BLAKE3 over the file's text as the daemon renders it.

**API.**
- `GET /v1/me/threads` returns the caller's state for every thread they have state for and still see.
- `PATCH /v1/me/threads/{thread}` merges a change and answers with the state as it now stands.
- `GET /v1/me/seen/{mr}` and `PUT /v1/me/seen/{mr}` read and set seen files.
- Only the caller reads or writes their own rows. The thread must be one they can see.

**On the daemon.**
- The daemon keeps a copy in its store and serves it to the web UI.
- Offline, it writes there, keeps the changes, and sends them when it's back. The server's per-field rule makes the replay safe.
- Without a server, the store's copy is all there is: one device, which is all a local-only person has.

**Sections** are derived in the UI, not stored:
- *Needs you*: an agent awaiting permission or input, a merge request awaiting your review, or an unread mention;
- *Working*: an agent working;
- *Active*: activity in the last three days;
- *Snoozed*;
- *Settled*: settled by hand, or merged.

A snooze that runs out, or wakes on activity, puts the thread back where it belongs.

## Consequences
- Two server tables and a store table, each with a migration. A purge removes the purged thread's rows.
- "Unread" compares the time of the thread's latest delta with `seen_ms`, so it works without a server too. Delta times come from their authors' clocks, so a badly wrong clock can mark a thread read or unread wrongly; that is a display matter only.
- Personal state never reaches other people, and isn't in backups of the family log. Server backups include it.
