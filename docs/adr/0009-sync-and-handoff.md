# 0009: HTTP sync, git bundles, thread leases and resume

- Status: accepted (refines ADR 0003 for the async-first direction of ADR 0006)
- Date: 2026-10-20
- Amended by: [ADR 0015](0015-secret-overlay.md): Secret redaction with a local overlay
- Amended by: [ADR 0032](0032-several-servers-and-local-projects.md): Several servers per daemon, and projects kept on this machine
- Amended by: [ADR 0121](0121-videos-and-images-without-a-size-limit.md): Videos, and images without a size limit

## Context
M3 has to move a thread to another machine: the code, the conversation, the agent session and any git commits the thread depends on. Without live co-editing (ADR 0006), keystroke latency doesn't matter. What matters is that sync is continuous, that nothing is ever lost, and that a self-hosted server is easy to run.

## Decision
**Transport.** HTTP with JSON bodies and bearer device tokens. Endpoints:
- `GET /v1/families`: families and threads the device can see.
- `POST /v1/families/{id}/deltas`: push a batch of deltas. The server dedupes by `(actor, lamport)`.
- `GET /v1/families/{id}/deltas?after=<seq>&wait=<s>`: pull, as a long-poll.
- `PUT|GET /v1/blobs/{hash}`.
- `POST /v1/threads/{id}/lease`.

Every request carries `Silt-Protocol: 1`. A server that doesn't speak that version answers `426` with the versions it supports, and the client stops and says so.

**State machines.** `silt-sync` holds Sans-I/O state machines: `ClientSync` per family (push cursor, pull cursor, one request in flight) and `Room` per family on the server. Both are tested with a deterministic network simulation that drops and reorders messages and rolls the server back. The daemon and the server are thin shells around them.

**Ordering.** The server gives each delta a per-family `seq` in arrival order. It is only a pull cursor; causality always comes from `parents`.

**Recovery.**
- *Server restart:* clients retry with backoff.
- *Server generation:* every response carries a generation the server picks at random each time it starts. A client that sees it change resets both cursors, pushes everything again and pulls from the start; the server dedupes. Restoring the server from an older backup always means a restart, so this alone guarantees nothing is lost. The simulation showed that cursor checks without a generation miss cases where a rolled-back server grows the same ids at the same positions again.
- *Cursor fingerprint:* a pull also names the delta the client last saw (`after`, `last`). The server answers `409 unknown-cursor` if that delta isn't at that position, and the client resets.
- *Stale answers:* a push or pull sent before a reset only moves a cursor if it started at the current cursor. A stale pull's deltas are dropped, since they may build on history this client doesn't have.
- *Same `(actor, lamport)`, different bytes:* this happens when a client's data was restored from a backup and reused its actors. The server answers `409 conflict` and its history wins. The client makes a **recovery fork**:
  1. It notes the conversation each affected thread has here.
  2. It drops its copy of the family and reads it again from the server.
  3. For each thread checked out or recording here, it forks a recovery thread from the server's version.
  4. It moves the checkouts and sessions there, with fresh actors, and records what is on disk as the recovery thread's changes. Conversation lines the server never got are added there again.
  5. `silt auth status` and the agent's next prompt say what happened.

**Git mirror and bundles.** When a thread is created, and when its checkout's HEAD moves to a commit origin doesn't have, the daemon does two things:
- It pushes the commit to the server's git mirror as `refs/silt/threads/<thread>`. The mirror is one bare repository per family, served over git's smart HTTP protocol by `git http-backend`. It sits behind the same device tokens and visibility checks as the rest of the API, with the token passed to git through the environment.
- It writes a git bundle of the commits origin lacks, stored as a blob and referenced by `GitOp::Bundle { commits, blob }`.

Another machine missing a commit tries origin first, then the mirror, then the bundles.

**Leases.** The server holds one lease per thread (`device`, `epoch`). `silt resume` takes the lease and records `Op::Lease { device, epoch }` in the family log, so every device learns of it through sync.
- A device without the lease never writes to the thread again. Disk changes in its checkout go to a fork of the thread instead (M4 builds on this), and `silt log` shows the thread as moved.
- Deltas made offline before the move are still accepted. They merge like any concurrent edit.
- The old device's fork starts from the thread's head as it was there before the lease change arrived. Work that came in the same batch as the lease therefore stays in the moved thread.

**Resume.** `silt resume <thread> [--here]` in a clone of the same repository, matched by normalized origin URL, does the following:
1. Pulls the family.
2. Takes the lease.
3. Makes sure the base commit is present, from origin or from the bundles.
4. Creates a managed checkout at the base, or uses the current clone with `--here`, which needs a clean tree. Writes the thread's current files and removes the ones the thread deleted.
5. Restores the agent session from the thread's native lines, with paths rewritten from the original cwd to the new one (ADR 0007).
6. Starts tailing the session after the restored lines and launches `claude --resume`.

**Redaction.** Before pushing, the daemon redacts secrets:
- values of environment variables whose names look secret, at least 8 characters long;
- known token formats.

Conversation lines get `[REDACTED]`. Text edits keep their length (`*` per character), so CRDT positions stay valid. Redaction is deterministic, so pushing again produces identical bytes.

## Consequences
- A pull's latency is the long-poll round trip. That's fine for async handoff, and a WebSocket can replace the long-poll later without touching the state machines.
- Bundles duplicate some git data in the blob store. They are small, since they only carry commits origin doesn't have.
- Redaction is basic until M8. Blobs (binary and large files) are not redacted. ADR 0015 replaces the anonymous placeholders with ones that name the secret, and adds a local overlay that puts known secrets back.
