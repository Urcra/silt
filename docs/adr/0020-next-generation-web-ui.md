# 0020: The next-generation web UI

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0011](0011-history-tools-search-and-web-ui.md) (the web UI's scope), [ADR 0014](0014-live-collaboration-and-silt-driven-agents.md) (per-thread live streams)
- Amended by: [ADR 0039](0039-the-web-ui-as-designed.md): The web UI as designed
- Amended by: [ADR 0041](0041-the-desktop-app.md): The desktop app

## Context
Today's web UI browses threads, search and merge requests, and has a live panel on the thread page. Running agents is the main thing people want to do from it, and it does that poorly:
- there's no view of every running agent;
- it sends one prompt at a time;
- it has no projects;
- it works only on the machine running the daemon.

A plan made at the time outlines its replacement: an agent workspace with projects, a prompt queue, review, hosted mode, phones and a history graph. It was written as a draft with decisions to make and open questions, before [ADR 0017](0017-acp-agents-only.md) removed the native harness and `silt sandbox`.

## Decision
**The plan is adopted.** Its phases become milestones M11 to M17 in ROADMAP.md, worked in order:

| Milestone | Phase |
|---|---|
| M11 | Foundations |
| M12 | The agent workspace |
| M13 | Projects, organization and review |
| M14 | Hosted mode and remote control |
| M15 | Phones and notifications |
| M16 | History graph, fork from a turn, revert |
| M17 | Several servers and local-only workspaces |

The site and docs are worked alongside.

**Where the plan leaned, that is the decision:**
- **The browser event bus is a WebSocket.**
  - One WebSocket per browser session carries commands and events, with subscriptions.
  - It replaces the per-thread SSE stream, which was built by polling.
  - Hosted mode uses the same event shapes.
  - Its details are recorded in their own ADR in M11.
- **The prompt queue is held by the server, with the daemon as its only consumer.** A phone can then add to it while the machine is offline. Until the control channel exists (M14), the daemon holds the queue, in its store.
- **Phones get a PWA.** Native apps are not planned.
- **The older boards fold into the workspace.**
  - The thread list becomes the workspace home.
  - The thread page becomes the workspace with the agent idle.
  - Merge requests, search, the inbox, the graph, sharing and reconcile are routes inside the same shell.

**Reconciled with ADR 0017:**
- **Silt sandboxes no agent.** The plan's "sandbox for every agent" and its "Sandboxed autopilot" preset are dropped.
- **Permission presets** answer only what the person would answer: whether to allow an edit or a command.
- **An agent's own sandbox** is chosen through its ACP modes, where it offers them, for example Codex's workspace sandbox.

**Still open, decided at the milestone that needs them, each in its own ADR:**
- hosted mode and remote control (M14);
- per-person state on the server (M13);
- projects (M13);
- read-only links (M15);
- fork from a turn and revert to a turn (M16);
- several servers per daemon (M17);
- commit ↔ turn links and trailers (M16);
- comment replies (M13).

These questions are open too:
- whether the browser co-edits through the server in hosted mode;
- who writes the change guide when no agent runs;
- where the site is hosted.

## Consequences
- The board gets M11's tasks. The plan's own phase numbers are replaced by these.
- The web app takes on what the original plan named and never installed: TanStack Router, Query and Virtual, generated API types and a design system (Nous).
- Hosted mode, the largest change, waits for its ADR (M14). Until then the browser still connects only to its own daemon.
