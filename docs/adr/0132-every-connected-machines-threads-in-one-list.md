# 0132: Every connected machine's threads in one list

- Status: accepted
- Date: 2026-11-26
- Amends: [ADR 0108](0108-machines-over-ssh.md) (what the app gives a machine's page, and what `silt://open/` picks), [ADR 0115](0115-the-sidebar-names-the-workspace-and-the-machine.md) (the machine menu)
- Builds on: [ADR 0026](0026-hosted-mode-and-remote-control.md) (agents running on the person's other devices), [ADR 0090](0090-agents-aware-of-each-other.md) (a terminal session's turn)

## Context
The desktop app's window shows one machine at a time: This computer, a WSL distribution (ADR 0045) or a machine over SSH (ADR 0108). Each serves its own page from its own daemon, and nothing passes between them without a server. Someone working on This computer, a NUC over SSH and a WSL distribution switches machines in the menu, and back, only to see whether anything there moved. The menu doesn't say whether a machine has anything running or waiting either. ADR 0115 left that open.

With a server, a thread from another device is already listed (`On homelab · 2d`), and an agent started from the web UI there reports how it stands (ADR 0026). An agent started in a terminal with `silt claude` doesn't. A daemon reports only the agents it runs itself (`agent_reports` reads only its ACP runs), so that thread reads as idle everywhere, this machine's own list included, until its messages sync.

## Decision
**The machine menu gets All machines,** above the machines, as the workspace switcher has Everything. Picking a machine shows only that machine, as today. All machines lists the threads of every connected machine in the one sidebar list, sorted as it is now. The window still shows one machine's page. The foot says which (`3 connected · showing nuc`).
- Only connected machines are listed. All machines connects nothing, so an SSH machine still connects only when it's picked or tested (ADR 0108).
- A thread joins its project by the project's origin (`repo`), so a project worked on from two machines is one group. Needs you gathers from every machine.
- A thread that is on two machines through a server is listed once, from the machine that holds it (`location: here`).
- Opening a thread on another machine switches the window to it and opens the thread there. The row is chosen at once, and the foot says `Opening it on nuc…`.
- Its other actions, pin, snooze and settle, and the read mark, pass through to its machine without switching. The page asks the app with a `silt://threads/state` link carrying the app's key, the backend, the thread and the change. The app reads the change as a `StateChange`, drops anything else, and sends it to that daemon's own endpoint. The list shows the change when that daemon's listing does.
- **New thread gets a machine picker** in All machines: "What should we work on in [silt] on [nuc]?". It lists the connected machines, with whether each has a clone of the project and how it stands. Picking one switches the window to that machine's New thread, with the draft and the project's origin, so its clone is picked there, or its folder picker opens when it has none. One machine at a time has no picker: New thread starts on the machine shown, as today. Pick up in shows the project's threads from every machine.
- Each machine's row in the menu says how it stands: `over SSH · 1 needs you · 3 running`.
- How the machines stand is said in the sidebar's foot, not in the list: `Getting nuc's threads…` above the machine while they load, as `Opening it on nuc…` is, and `2 connected · lost nuc` when one drops. A dropped machine's threads leave the list, and the app reconnects as it does now. With only This computer connected, the list is its own and the foot says `1 connected`.

**The desktop app gathers the listings.** It holds each connected backend's address and token already. While All machines is picked, it asks each one it doesn't show for `/api/threads` every 3 seconds, over the tunnel, and sooner after a change it passed on. It hands every page the result:
- `window.__siltThreads`, with a `silt-threads` event when it changes: per backend, its id, `loading`, `ready` or `lost`, and its `ThreadSummary` list as that daemon gave it.
- Another machine may run another build. The page reads a summary's fields it knows and ignores the rest, and a missing field reads as absent.
- Every machine's page gets them, This computer's, a distribution's and an SSH machine's alike: they are all the person's. A page still can't make the app connect anywhere new.

**`silt://open/` can name a thread,** `?backend=<id>&thread=<id>`, on a backend the app knows. The id is a thread id's hex and nothing else, so it can't become a path, and the app opens `/t/<id>` on that backend's page. This is allowed for SSH machines too: it names a thread, not a directory.

**How a thread's machine is marked is a setting,** in Settings › Machines:
- a machine chip (the default);
- the chip in the machine's colour;
- in the line under the title, `Working · on nuc`, as a synced thread is already named;
- a colour stripe at the row's edge;
- the machine's initial on its colour;
- nothing.

Only another machine's threads are marked, unless **Mark the shown machine's threads too** is on. Each machine has a colour, from six that read on light and dark and stay apart from amber, rust and olive, which mark a thread's state. They are assigned in the order machines are added, and can be changed on the machine's row. The app keeps the colours, the list choice, the marking and the toggle beside it in `view.json` in its data dir, so they hold on every machine's page. Each page gets them in `window.__siltBackends` and `window.__siltDesktop.view`, and changes them with a `silt://view/` link carrying the app's key, as it edits machines.

**An agent run from a terminal reads as working.** Capture already records a terminal session's turn (`idle`, from `UserPromptSubmit` and `Stop`), which ADR 0090's peers read.
- A thread whose terminal session is mid-turn reads as working in its own machine's listing, so All machines shows it too.
- Its daemon reports it to its server beside its ACP agents, so the person's other devices and the hosted app show it.
- A command for it, a prompt or a stop, is refused with "answer it in the terminal on nuc". Protocol 25.

## Consequences
- Machines over SSH and in WSL work as one place to watch, with no server. Moving a thread between them still takes one ([handoff](../guide/handoff.md)).
- A machine's page now holds the titles and states of other machines' threads, though not their conversations. The person owns all of them; ADR 0108 kept an SSH machine's page from steering the app, and it still can't.
- An older build on another machine still answers `/api/threads`. Its page, though, shows only its own threads until it runs a build with this.
- Search and All threads still cover only the machine shown.
- The tray and notifications across backends, still to come, can read the same listings.
- A terminal agent's permission prompts are answered in its terminal, so they show as `Waiting on you` only once a hook reports them.

## Alternatives considered
- **Following each daemon's `/api/events`.** It would tell the page sooner, but needs a WebSocket client in the app, which only speaks plain HTTP to daemons. A listing every 3 seconds is small next to what the page itself asks.
- **The page asks the other daemons itself.** It would need each daemon's token, and each daemon would have to accept another origin. The app already holds both and stays the only thing that reaches other machines.
- **The daemon on This computer gathers them.** It knows no other machine; the app owns the tunnels. And it would make This computer's page special, when any machine's should do.
- **A section per machine.** It splits a project worked on from two machines, and the list stops being one order of what moved last.
- **Connecting every machine for All machines.** An offline VPS would slow the list, as it would the launch, for a machine the person didn't ask to see (ADR 0108).
- **Only a count per machine in the menu.** It says that something moved, but not what, and the person still switches to find out.
