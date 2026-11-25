# 0021: The browser event bus

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0014](0014-live-collaboration-and-silt-driven-agents.md) (the per-thread live stream)
- Amended by: [ADR 0101](0101-the-ui-under-heavy-load.md): The web UI under heavy load

## Context
**How the UI hears of changes today:**
- Each thread page opens an SSE stream. The daemon builds it by asking for `Request::Live` every 250 ms and sending the answer when it differs.
- The answer is a delta count and the agent's status; the page fetches everything again when the count moves.
- Lists, the inbox and merge requests never update.
- Agent text shows only as the last snapshot of what it is saying.

**What the workspace needs** ([ADR 0020](0020-next-generation-web-ui.md)):
- a sidebar of every thread and its agent, live;
- streaming agent output;
- an inbox that updates.

ADR 0020 settled on a WebSocket.

## Decision
**One WebSocket per browser session**, at `GET /api/events`.
- **Authentication:** the token goes in the query, as the browser's WebSocket can't send headers.
- **Same checks as every route:** the `Host` check applies, and the endpoint answers only on 127.0.0.1.
- **Commands stay HTTP routes**, which are idempotent and easy to test. The socket can carry commands later, as hosted mode will need (ADR 0020).

**Subscriptions.** The browser sends JSON messages:
- `{"subscribe": [...]}` and `{"unsubscribe": [...]}`;
- topics: `threads`, `thread:<id>`, `inbox` and `mrs`;
- a new socket hears nothing until it subscribes.

**Events.** The daemon sends JSON messages tagged by `event`:

| Event | Topic | Says |
|---|---|---|
| `threads` | `threads` | The thread listing may have changed: a thread was created, renamed, moved, trashed or purged, or its agent's state changed. |
| `thread` | `thread:<id>` | The thread's family has new deltas, with `deltas`, the family's log length, as its version. |
| `agent` | `thread:<id>` | The agent's status (`AgentStatus`, or `null` once it stopped): its state, its permission requests and its error. |
| `agent_text` | `thread:<id>` | A piece of what the agent is saying (`message`), thinking (`thought`) or a tool is printing (`tool`), as it arrives. The recorded entry replaces it when the turn records it. |
| `inbox` | `inbox` | The inbox may have changed: deltas arrived from others, or notifications were read. |
| `mrs` | `mrs` | Merge requests may have changed. |
| `lagged` | any | The socket fell behind and missed events. |

- Events say what changed, not the new data, except for agent status and text, which are small and frequent.
- After a `lagged` event, the browser fetches everything again.

**Source.** The daemon emits events from where things happen, through a broadcast channel each socket reads and filters by its subscriptions:
- deltas when they are stored;
- the listing when the server's changes are taken in;
- agent events when the ACP run reports them.

The 250 ms poll of `Request::Live` and the per-thread SSE route go away.

**In the browser,** events invalidate TanStack Query keys:

| Event | Query keys |
|---|---|
| `threads` | `['threads']` |
| `thread` | `['thread', id]` and everything under it |
| `inbox` | `['inbox']` |
| `mrs` | `['mrs']` |

`agent` and `agent_text` update the thread's agent state in the cache directly. The browser's WASM replica (ADR 0014) fetches remote deltas when a `thread` event's version moves past its own.

## Consequences
- **One connection** per tab, whatever it shows. Lists and the inbox update live, and agent output streams as it arrives.
- **Presence** still goes over its own HTTP route, every two seconds. It can move onto the socket later.
- **Hosted mode** will serve the same event shapes from the server (ADR 0020).
