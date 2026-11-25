# 0081: Live in a page: rooms, and whiteboards

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0014](0014-live-collaboration-and-silt-driven-agents.md) (presence), [ADR 0072](0072-what-an-artifacts-page-can-do.md) (the capabilities), [ADR 0076](0076-artifact-types-slides-canvases-and-data-pages.md) (the types)

## Context
A page knows nothing of the other people who have it open. A shared timer, a planning-poker round, a game, a presenter's current slide and people's cursors on a whiteboard all need moments that reach everyone at once and are gone after.

Silt has presence for threads: each viewer's daemon sends a heartbeat every 2 seconds to the server, which keeps it in memory with a 15 s lifetime (ADR 0014, `presence.rs`). Deltas travel by long polling, about a second end to end. That's right for drafts, but too slow for cursors. ADR 0006 puts live multiplayer last, so this comes after the rest of the artifact work.

## Decision
**`room`: moments between the people viewing a page now.**
- **Declaring:** `capabilities: {room: {topics?: {move: "comment"}}}`.
- **The functions:**
  - `emit(topic, data)` and `on(topic, fn)` for moments;
  - `presence(patch)` and `onPeers(fn)` for each viewer's current state, such as a cursor;
  - `join(name)` for a sub-room with the same functions, and `leave()`.
- **Who's in a room:** everyone viewing the same artifact, across versions, with an account that can read its thread. Read-only links get `null`.
  - Each peer has a user id (ADR 0072's `user`), a device, and whether they're the thread's agent.
- **Who may send:** presence is open to every peer. Topics are `collaborator` and up by default; a declaration can open one to `comment`. Nothing heard is trusted, and presence is never authority.
- **Limits:**
  - a message is at most 16 KiB and presence at most 4 KiB;
  - a peer sends at most 30 messages a second;
  - presence lasts 15 s without a refresh, as thread presence does.
- **Nothing persists.** State a late joiner needs goes in page data (ADR 0072); lasting content goes in a new version.

**A WebSocket path, beside sync.**
- **The server:**
  - it gains `GET /v1/rooms`, a WebSocket for daemons, and `/app/api/rooms` for the hosted app's browsers;
  - it keeps rooms in memory only, like the presence board, and drops a room when its last peer leaves;
  - it checks each peer's right to read the thread when they join, and each topic's level when they send.
- **The daemon:**
  - it holds one socket to each of its servers, opened when one of its pages joins a room, and multiplexes its browsers' rooms over it;
  - with no server, it relays between the views open on that machine.
- **Speed:** a moment takes one hop per side, with no long poll. That's typically under 100 ms on a nearby server.

**The agent can take part.**
- `silt_artifact_room { id, emit?, listen? }` lets the thread's agent emit a moment and read the last 100 messages and the peers present.
- The room keeps those 100 in memory for the agent only. The agent shows as a peer of kind `agent`.

**`whiteboard`: a canvas people arrange together.**
- **Content:** items in the page's data (ADR 0072), one document each under `board/items/<id>`, so concurrent edits merge per item:
  - blocks (sticky notes, text, shapes), with position, size, colour token and text;
  - arrows between two items, with a label;
  - sections that group items;
  - images (`assets`, ADR 0074);
  - marker strokes, as simplified point lists.
- **Drawing it:** Silt's own viewer draws it, with no agent code (ADR 0076): a canvas that pans and zooms, selection, moving, resizing, connecting and drawing.
- **Live:** cursors and what each person is dragging go through the room's presence. Edits are written to page data when a drag ends, so they persist and sync.
- **Without a server,** the room spans one machine's views, and edits sync as page data does.
- **The agent:**
  - it lays out a whiteboard from a first version's JSON, and adds, moves or connects items with `silt_artifact_data`;
  - `silt_artifact_guide { kind: "whiteboard" }` gives the item schema and the layout guidance.
- **Exporting:** an image (PNG or SVG) of the whole board or a section, through `downloads`.

## Consequences
- **Pages can be shared live:** a timer, a vote, a presenter's slide, a game, or a whiteboard with everyone's cursors.
- **The server keeps sockets open for rooms,** in memory, with no table and no migration. A restart drops the rooms; pages rejoin and peers send their presence again.
- **Rooms are new on the wire** (the daemon's socket), so they go with a protocol bump, the one after ADR 0072's 13.
- **Live multiplayer arrives for pages before it does for threads,** whose co-editing stays at sync speed (ADR 0014).
- **A whiteboard is page data,** so it forks and hands off with the thread. Its cursors don't; they're only for whoever is there.
