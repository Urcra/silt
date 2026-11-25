# 0084: A data page's numbers live from a connector

- Status: accepted
- Date: 2026-11-16
- Amends: [ADR 0080](0080-a-page-calls-mcp-servers.md) (data pages' sources), [ADR 0076](0076-artifact-types-slides-canvases-and-data-pages.md) (data pages)

## Context
ADR 0080 let a data page's source be an MCP tool, `{kind: "mcp", server, tool, input}`, that the viewer's computer runs on open and on Refresh, but not how the tool's answer becomes the page's numbers. A data page draws no code of its own, so the answer has to land in its blocks by a rule Silt applies.

## Decision
**A block points into the answer.** A block naming an `mcp` source may say `pick`, a JSON pointer (RFC 6901) into the tool's answer: a stat's value, a table's rows, a chart series' points. The answer is the tool's structured content, or its first text read as JSON, or that text. What the pointer finds replaces the block's own value only when it has the shape that block draws; otherwise the agent's value stays.

**The agent's values are the page's own.** The version keeps the numbers the agent wrote, shown until the tool answers, where it can't run, and in an offline copy. Nothing fetched is kept in the version or the thread.

**Read-only tools, agreed once.** The version declares the connector and tool under `mcp`, as an HTML page does; the viewer runs only a tool its server marks as read-only, after the person agrees once per page with **Load them**, which keeps the same `mcp` grant ADR 0072's pages take. The daemon checks the declaration, the ticked tool, the grant and the workspace's connector rules as for any page's call; the hosted app relays it to one of the person's computers.

## Consequences
- The page shows when each source was fetched, or why it wasn't.
- **Not done:** a refresh on a timer, and an answer reshaped beyond a pointer.
