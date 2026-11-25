# 0116: Memory that follows the work

- Status: accepted
- Date: 2026-11-23

## Context
A daemon on a machine with 69 families held 7.2 GB, all of it heap, from its first minutes on. Four families were most of it: one of 805 MB of files when stored, three of 236 to 264 MB.

- **The daemon loads every family at start and keeps it.** `Core::load` projects each family a thread belongs to, log and files, and nothing unloads one. A family nobody opened for weeks costs as much as the one an agent works in.
- **Files are 97 to 99% of a long family's snapshot.** The tree and the rest are a few MB.
- **Text takes four bytes a character.** An insertion is a `Vec<char>`, and a `TextDoc` keeps every character ever inserted, deleted ones too.
- **Every file is decoded at load,** though a session touches a few.
- **A git base bigger than the text limit becomes text when edited.** `materialize` copies the base in whole; only content read from disk is checked against `TEXT_LIMIT`.
- **Paths captured before they were ignored stay in the thread.** One thread captured a 12 GB virtualenv in the 39 minutes before its `.gitignore` named it: 39,890 files.

The views that read every family (the thread list, merge requests, the graph, cloud launches) read only facts of the log, never the files.

## Decision
**The store is the truth, and what the daemon holds of it is a cache.** It holds a digest of every family, the families in use, and of those the files in use.

**A digest of every family.** The daemon's `Digest` holds what the views across families read of a family: the log's length, each thread's summary, cloud session, commits and the files it changed, the merge requests and the graph's rows. Those views' own code takes it from the family just before the family is let go of, so it says what they would; a family changes only once held again. It is stored in `family_digests` with the store's position in the family then, and kept for the families held when the daemon stops. At start, a digest in another `DIGEST_FORMAT`, or older than what was stored since, as by a hook while the daemon was down, is taken again from the family, loaded for it, a family at a time between other events; requests wait for them, but hooks don't, as the views across families would otherwise load every family at once. While a family held hasn't grown since its digest, the views read the digest, so a family held for a look needn't read its log. It lives in the daemon rather than `silt-log`, as the summaries read the agents' transcripts.

**Families load when something names them.** Startup loads threads, checkouts, sync states and digests, and no family. A request or a watcher's change naming a family that isn't held waits while the loader's thread loads it, from a store connection of its own; the core goes on meanwhile, and what came after for that family waits behind it. Anything else reading a family not held loads it there and waits, so no reader finds a family missing that was only let go of. A load that something else's overtook is dropped.

**Families unload when unused.** Memory freed is handed back to the system (`malloc_trim` under glibc, which otherwise keeps it). After 5 minutes unused (30 since [ADR 0117](0117-conversations-open-at-their-end.md)), or least recently used first when the families held pass `SILT_FAMILY_BUDGET_MB` (1024 by default, as the store holds them), a family unloads after a snapshot and its digest are saved. It doesn't while a session or an ACP run is in it, while it has deltas not pushed or a push or pull in flight, or while a live room or an import uses it.

**Deltas pulled for an unloaded family load it.** They are applied as before, never stored unapplied, as sync's positions count what applied. So do deltas another writer stored for one.

**Files decode when used.** A snapshot (format 6) keeps each file's state apart, by hash, in `$SILT_HOME/files/`, and an index of them. States are written without syncing each: one missing or damaged makes the family replay instead. A family reads a file's state the first time something reads the file, through a reader its shell gives it, and lets go of one unused for 5 minutes and unchanged since the snapshot. A loaded family's log goes back to unread the same way.

**Text is UTF-8.** An insertion holds a `String` and the byte offset of every 128th character; positions stay in characters, and its encoding stays the `Vec<char>`'s.

**Text files over 5 MiB are blobs.** One `TEXT_LIMIT` in `silt-checkout`'s write policy. The bridge writes text edits only when what the file holds and what it will hold are both within it, a git base included, and `SetContent { Blob }` otherwise. Whoever writes a delta applies the limit, never whoever applies it, so every replica projects the same.

**Paths that become ignored already leave the thread** ([ADR 0060](0060-ignored-files-leave-the-thread-and-large-diffs-are-read-by-file.md)): a changed `.gitignore` rescans the checkout. A clone that is the person's own again keeps them until an agent works there, and files decoded only when used keep them out of memory meanwhile.

## Consequences
- The daemon's memory follows what is open and working, not the history it knows.
- Opening a thread whose family isn't loaded waits for the load; the rest of the daemon doesn't.
- Format 5 snapshots are converted to format 6 at their first load, not replayed. The first start after updating takes every family's digest and converts its snapshot: on one machine with 148 families, 4.5 minutes and 2.5 GB at most, after which the daemon held 230 MB idle where it held 7.2 GB.
- A text file over 5 MiB merges whole, as git merges a binary file.
- `silt-log` stays pure: files and the log are read through readers the shell passes in, and without one a family holds them all, as the browser's replica does.
