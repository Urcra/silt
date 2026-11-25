# 0026: Hosted mode and remote control

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0011](0011-history-tools-search-and-web-ui.md) and [ADR 0014](0014-live-collaboration-and-silt-driven-agents.md), where the browser talks only to its own daemon.
- Amended by: [ADR 0028](0028-notifications-and-phones.md): Notifications and phones
- Amended by: [ADR 0056](0056-signing-an-agent-in-from-the-web-ui.md): Signing an agent in from the web UI

## Context
The web UI is served by the daemon on the person's machine. That leaves out:
- a phone;
- a second laptop with no checkout of the thread;
- answering an agent that waits on the person while they're away from the machine running it.

ADR 0011 left a hosted UI "for later, with the same API shape". The next-generation UI ([ADR 0020](0020-next-generation-web-ui.md), plan section C) needs it now. Turns still run only on people's machines (ADR 0020, principle 1): the server never runs an agent. It relays commands to the daemon that does.

## Decision
**The server serves the web app.**
- The same bundle the daemon embeds is served at `/app/` on the server's public URL. The router and the API client take their base from the `<base href>` each server writes into the page. The daemon keeps serving at `/`.
- **Authentication:** the session cookie the server's sign-in pages already set (`silt_session`: HttpOnly, SameSite=Lax). There is no token in the URL or in storage.
- **CSRF:** every request that changes something must carry `X-Silt-Request: 1` and a matching `Origin`. Browsers don't let other sites add that header without CORS, which the server never allows.

**The browser API** lives under `/app/api/`, shaped like the daemon's `/api/` so the same client code works against either. The server answers what it holds from its own data:
- threads and their summaries;
- a thread's history, replayed from its log with the same readers the index uses;
- search, merge requests and comments;
- the inbox, orgs, projects, and personal state (ADR 0024).

Where the browser changes the log (a comment, a reply, a review verdict), the server writes the op itself, with an actor of its own for each person. The op is authored by that person, like any device's.

**Remote control.** What only a daemon can do — run an agent, read a checkout, write files — becomes a command for that daemon.
- **Control channel:** each daemon long-polls `GET /v1/control` next to its sync. It takes the commands meant for it and answers each with `POST /v1/control/{id}`. The server keeps commands in a table until they're answered or expire.
- **Commands:** start an agent, prompt, queue, steer, answer a permission, cancel, stop, set the preset or mode, and resume a thread here. Each carries the sending person and device, and is recorded like a prompt sent locally.
- **Checks on the daemon,** not only on the server (ADR 0016):
  - the sender must be the daemon's own person, as only they may run things on their machine;
  - the sending device must have the capability for the command.

  A collaborator's prompt still goes through the owner's queue on their own machine, as today.
- **Device capabilities:** per device, set by its person: answer permissions, send prompts, start and stop agents, review merge requests. A new device gets all of them except a phone's, which can answer permissions and review only. The server holds them, and sends them with each command for the daemon to check.

**What runs where.**
- Each daemon reports its running agents to the server: each thread, its state, its open permission requests, its queue length. It reports when they change, and with a heartbeat every 30 seconds.
- The server keeps the reports in memory and marks a device offline after 90 seconds without one.
- Browsers see "working on workstation" and "waiting on you" without opening the thread. A thread's live view, served by the server, shows the reported state, and its streaming text when the daemon sends it.

**Offline devices.** A command for an offline device is refused with "workstation is offline". The exception is a prompt, which the person may choose to queue: it waits on the server until the device comes back.

**Local daemon first.** The app served by a daemon keeps talking to that daemon. For a thread running on another device, the daemon passes the command on to the server, so a laptop's UI drives the workstation's agent the same way.

## Consequences
- New server tables for commands, device capabilities and last-seen, each with a migration. A server restart loses the running-agent reports until the next heartbeat, which is fine for a display.
- The server now executes writes on people's behalf for browser actions. They're limited to ops the person could make from a daemon, and checked with the same visibility and write rules.
- Hosted mode widens what a stolen session can do. Capabilities, the daemon-side checks, and a record of every relayed command (who, from which device, when) limit and show it. Signing out everywhere (ADR 0013) also ends browser sessions.
- The SPA must not assume a daemon: features without one, such as live file editing with the WASM replica, are hidden when the server serves it.
