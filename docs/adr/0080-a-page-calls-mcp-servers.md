# 0080: A page calls MCP servers the viewer lets it use

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0072](0072-what-an-artifacts-page-can-do.md) (the capabilities), [ADR 0076](0076-artifact-types-slides-canvases-and-data-pages.md) (data pages' sources)
- Amended by: [ADR 0084](0084-data-pages-live-from-a-connector.md): A data page's numbers live from a connector
- Amended by: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md): Silt sandboxes the agents it starts, and production is reached through plugins

## Context
A page's numbers are frozen when the agent writes them. A status board, an issue tracker view or a dashboard of a service's metrics goes stale at once, and can't act: it can't close an issue or re-run a query. A page needs to call the viewer's connectors with the viewer's credentials, to show live data and take actions the viewer allows.

Silt has no connectors of its own. Its agents bring their own MCP configuration, which often holds tokens, and the only MCP server Silt hands them is `silt mcp`. A page here comes from a thread that may be another person's (ADR 0016), so anything it can reach must be the viewer's choice. It must also be visible to the viewer, and it must run on the viewer's machine.

## Decision
**Page connectors: MCP servers a person sets up for pages.**
- **Where they're kept:** `$SILT_HOME/page-connectors.json`, one list per machine, edited in Settings › Artifacts › Connectors. They're the person's, and never synced.
- **What each has:**
  - a name;
  - a transport: stdio (a command and its arguments) or Streamable HTTP (a URL, with headers taken from the system's keyring, never stored in the file);
  - the tools it offers to pages, all off until ticked.
- **Importing:** Settings can import servers from Claude Code's configuration, with every tool off. It copies the command and URL; secrets stay where they are, referenced by name.
- **Who may refuse them:** a workspace's rules (ADR 0065) and an org's policy can refuse page connectors, or allow only named ones.

**The daemon is the MCP client.**
- **How:** it starts and talks to each server with the official Rust SDK, `rmcp`, on demand. A stdio server ends after five minutes unused.
- **Where calls run:** only on the viewer's own daemon. In the hosted app, a call goes through the control channel to one of the viewer's own online devices (ADR 0026), and is `null` without one. Read-only links get `null`.

**What a page declares and what the viewer allows.**
- **Declaring:** `capabilities: {mcp: {servers: [{server, tools: [...]}]}}`. `server` is a page connector's name. Every entry names its tools, and an empty list is refused.
- **Consent:**
  - The first call to a server asks the viewer, showing the tools the page wants. It's granted per viewer per artifact (ADR 0072).
  - A tool missing from the viewer's own page connectors, or not ticked for pages, is refused as `not_granted`. The page then shows the feature as unavailable.
- **Reading and acting:**
  - **Reads:** a tool the server marks `readOnlyHint: true` may be called freely within the grant: `watchTool(server, tool, input, handler, {refetchInterval})` caches and refreshes it, polling no faster than every 30 s.
  - **Actions:** any other tool is an action. `callTool` shows the viewer the tool and its input, and runs it only when they confirm, every time. A page can't act on the viewer's behalf unseen.
- **Limits:** a result is at most 1 MiB; a page may make 30 calls a minute.
- **Errors:** failures reject with the server's error, marked `retryable` or not.

**Data pages can refresh from a connector.** A source of an ADR 0076 data page may be `{kind: "mcp", server, tool, input}`. The viewer's daemon re-runs it on open and on Refresh, within the same consent, and the page shows when each number was fetched and from where.

**What's untrusted.** A tool's results are as untrusted as the page. Pages insert them with `textContent`, and nothing a tool returns becomes code.

## Consequences
- **Pages can show live data and take actions,** with the viewer's own credentials, on their own machine, and only through servers and tools the viewer ticked.
- **A hostile page can read what the viewer let it read,** and could write it into its own data, which syncs. The consent prompt says so, and actions always need a click.
- **Silt now runs MCP servers the person lists,** and `rmcp` becomes a dependency of the daemon.
- **Page connectors don't follow the person between machines.** Each machine has its own list, since commands and credentials are per machine.
