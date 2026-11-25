# 0035: A family's deltas stored once, compressed in chunks

- Status: accepted
- Date: 2026-10-22
- Amends: [ADR 0008](0008-local-capture-architecture.md) (the store)

## Context
The store keeps each delta as a row of `deltas`. Since M10, every thousand rows are also copied into one `delta_chunks` blob, so that a family loads in a few reads. But the rows stay, so each delta is stored twice: 110 MB of rows and 110 MB of chunks for a 100 MB transcript, plus its search index. That's 2.9 times the transcript, where ADR 0034's budget is 1.5.

It's slow too. On the reference machine, writing the rows costs 2 to 4 s of fsync when SQLite checkpoints its WAL, while importing the transcript has a budget of one second. The transcript's lines are JSON with a lot of repetition: zstd at level 1 shrinks the chunks of a real-sized session 7.4 times, in 67 ms for all 110 MB, and reads them back in 23 ms.

A row's other columns are still needed:
- `(family_id, actor, lamport)` is unique, which keeps the store from holding two deltas under one id (snapshots rely on it).
- `family_positions` follows the rows through their triggers.
- Catching up with other writers (ADR 0018) asks for the deltas after a position.

## Decision
**A row keeps a delta's place, a chunk keeps its bytes.**
- Every delta still has its `deltas` row, with its position, id and length.
- Once a chunk holds the delta, the row's `data` is empty.
- Rows after a family's last chunk keep their bytes, until a thousand of them make a chunk.

**Chunks are compressed.** `delta_chunks.codec` says how a chunk's blob is encoded:
- `0`: the plain packing that chunks had until now.
- `1`: that packing, compressed with zstd at level 1.

New chunks are compressed. Chunks already stored stay as they are, and both kinds read back.

**Writing goes to chunks directly.** An append that completes chunks writes them, compressed, in the same transaction, from:
- the rows still waiting;
- the new deltas.

Only what's left over is stored in rows with its bytes. Importing a long transcript then writes each delta's bytes once, compressed. Recording one line at a time writes a row, and that row's bytes move into a chunk with the next thousand.

**Rows go in many at a time.** An append inserts its rows and their search rows in multi-row statements, rather than one statement per row.

**Reading by position reads chunks too.** `deltas_after(position)` reads the chunks that end after the position, then the rows after the last chunk. So a writer catching up finds every delta, packed or not.

**Migrating:** a migration adds `codec`, and empties the rows that the existing chunks already hold. The space they free is reused by SQLite; it isn't returned to the disk until a `VACUUM`. The migration can't be undone in SQL, because the bytes now live only in chunks. Going back means restoring a backup of the data dir.

## Consequences
- **Measured** with `silt-bench` on the reference machine, for 100 MB of transcript:
  - the data dir holds 0.56× the transcript, down from 2.90×;
  - tracking it takes 0.79 s, down from 8 s.

  Both budgets are enforced from here on.
- **On a real store:** a store of 73,069 deltas read back the same after the migration, byte for byte, and loads in 93 ms rather than 139.
- A family's deltas take about a seventh of the space they took. The store is then mostly the search index and the snapshots.
- Importing writes about 15 MB for a 100 MB transcript, rather than 220 MB.
- Reading a family decompresses its chunks: about 25 ms for 100 MB.
- An older build can't read a store this one migrated, since it doesn't know compressed chunks.
- The server's store, which holds rooms rather than a daemon's families, is unchanged here, and has budgets of its own.
