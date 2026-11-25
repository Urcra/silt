# 0001: Eg-walker-style CRDT based on git versions

- Status: accepted
- Date: 2026-10-20, revised the same day.

## Context
Silt needs file contents (and later comments) to converge across replicas. That covers a user's laptop and home PC, a teammate's fork, and a merge request coming back. Work must be rebasable onto new git commits, and anchors must survive edits.

Edits based on either a git version or a frontier of earlier edits, loosely based on diamond-types, meet that. The design needs:
- one Lamport timestamp per history
- one actor per replica or mount
- run-length-encoded delta nodes with a parent frontier
- a persistent fragment B-tree
- an inode-based move tree for the file system

Under ADR 0006, live co-editing is no longer a goal. The CRDT now exists mainly so that **forks, pulls, merge requests and cross-device resumes merge correctly**. It also remains the base for adding live editing later.

## Decision
We write our own CRDT in `silt-crdt`, following Eg-walker (Gentle & Kleppmann, arXiv 2409.14252).

**Identity**
- `DeltaId { actor: ActorId(u128), lamport: u64 }` identifies **one character or unit** (ids must be per character, as in diamond-types). An event of length n takes the contiguous range `[t, t+n)`. There is **one Lamport clock per family history**: a new event starts at `max seen + 1`.
- Actors are given small per-history ordinals for storage.
- Every replica gets its own actor: each checkout or mount, each daemon, each server-side writer. An identity record maps actors to users.

**Versions**
- `Version::Git(oid) | Version::Delta(Frontier)`.
- The text encoding is `ord:ts,ord:ts`. Versions are interned per history.

**Causal graph**
- Run-length-encoded runs `(actor, lamport_start..=lamport_end, parents)`.

**Text**
- Events are positional (`Ins{pos, content}` / `Del{pos, len}`) and are interpreted relative to their `parents` frontier. No character IDs go on the wire.
- Each document's replica state is a persistent fragment index: `SumTree<Fragment{insertion: (DeltaId, is_base), offset, len, deletions}>`. It is vendored from Zed's Apache-2.0 `sum_tree`. Tombstones are kept forever.
- Applying an event: the fast path is used when the parents equal the current frontier. The slow path is version-vector-filtered integration with YjsMod ordering. Concurrent siblings with the same origins are ordered by lower actor first, then lower id, matching diamond-types, which is the differential oracle (git master, dev-dependency).
- `text_at(version)` and anchors `(insertion DeltaId, offset, bias)` are answered from the fragment index.
- File contents stay lazy (a git blob pointer) until the file is first edited.

**File tree: an inode move-CRDT** (this replaces the earlier path-LWW design)
- An inode is `{id: (DeltaId, ordinal), kind: Dir | File{content, mode: LWW, eol: LWW} | Symlink, locations: [Root | Parent{inode, basename}], is_deleted}`.
- Concurrent moves add candidate locations. The active location is the winning candidate, chosen by deterministic ordering, skipping any candidate that would create a cycle (Kleppmann's move-op approach). Inodes with a pending cycle break are tracked explicitly.
- Path entries map `(parent, basename)` to owners. When there are several owners, it is a name conflict: the losers are shown with a deterministic suffix, and the conflict is surfaced for review.

**Git interop**
- `Git::Advance` plus a diff whose parents are the import point. Merging it rebases thread work onto the new commit.

## Consequences
- We control git interop, anchors and attribution. In return, correctness is ours to prove. We rely on proptest convergence suites, plus a differential oracle against diamond-types for text (see docs/research/probes.md), plus move-tree property tests: no cycles, convergence, no lost inodes.
- The move tree costs more than path-LWW, but directory renames in two forks merge correctly, which matters a lot for merge requests.
- Overlapping concurrent text edits merge silently. We detect overlapping concurrent regions on the slow path and show them as conflict hints in merge-request review.

## Implementation notes (M1, 2026-10-20)
**Fragment index**
- Run-length `Fragment`s live in the vendored `sum_tree`, ordered by local fractional `Locator`s.
  - Appends at the end take a small step; inserts between two fragments take the midpoint, so locators stay within the inline `SmallVec`.
- An index maps each insertion's start id to its char data and its fragment locators, so any character id resolves to its fragment in O(log n).

**Prepare state (Eg-walker)**
- Each fragment has a *prepare* state: not inserted, visible, or deleted n times. `move_prepare_to(parents)` retreats or advances only the events that differ.
- Positions of an incoming event are read from the `PrepareLen` dimension. Sequential edits leave the prepare version where it is.

**Fast paths**
- Typing that continues an insertion extends its fragment in place (`SumTree::update_item`).
- Inserts and deletes with no concurrent region splice a single leaf in place (`SumTree::splice_item`) when the leaf has room.
- Everything else slices and re-appends the tree.

**Visibility at an old version**
- A fragment can hold characters from several insert events of one actor. Visibility at a version is therefore a prefix by lamport.
- Deletes are always split to whole fragments.

**Results**
- Every unit, property and differential test from the per-character prototype passes unchanged.
- automerge-paper (260k edits) replays in 585 ms, about 2.3 µs per edit.
