# 0003: WebSocket + postcard sync protocol, Sans-I/O state machines

- Status: accepted
- Date: 2026-10-20

## Context
Daemons and the server both replicate delta logs. Clients need catch-up, live broadcast, offline outboxes, presence and a version check.

## Decision
- **Transport:** one WebSocket per replica connection, carrying binary `postcard` frames for sync. The UI's JSON API is a separate WebSocket.
- **Messages:**
  - `Hello{proto}` → `Welcome | Incompatible`
  - `Subscribe{thread, have, cursor}` → chunked `CatchUp`
  - `Push{batch}` → `Ack | Nack{MissingParents | Forbidden | Diverged}`
  - Also: `Deltas` broadcast, ephemeral `Presence` and `TermStream`, `InboxNotify`.
- **Ordering:** one room actor per family serializes appends. It assigns `server_seq`, which is used only as a catch-up cursor; causality always comes from `parents`. The room persists a delta before acking or broadcasting it.
- **Idempotency and divergence:** re-pushing a duplicate delta with identical bytes is idempotent. Different bytes under the same id mean `Diverged`, which is handled by a new ActorId plus a recovery thread.
- **Structure:** the protocol logic is Sans-I/O (`Room::handle`, `ClientSession::handle` return actions), with thin tokio adapters around it.
- **Blobs** travel over HTTP `/blobs/{blake3}`.

## Consequences
- postcard is not self-describing. To keep compatibility we use a strict `PROTOCOL_VERSION`, append-only enum variants, and payloads the server treats as opaque.
- *Amended 2026-10-20:* stored deltas, on machines and on the server, carry a `format` next to their bytes (`silt_log::DELTA_FORMAT`, now 0). A change to `Delta`'s encoding bumps it and keeps a decoder for each earlier format, so existing databases stay readable. A database written by a newer Silt is refused with a clear error rather than misread.
- The state machines can be tested deterministically without a network.
