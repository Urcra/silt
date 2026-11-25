# 0032: Several servers per daemon, and projects kept on this machine

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0009](0009-sync-and-handoff.md), where a daemon syncs with the one server it signed in to, and [ADR 0013](0013-accounts-orgs-and-self-hosting.md).
- Amended by: [ADR 0043](0043-history-from-day-one.md): Your history from day one: release binaries, importing old sessions, sending them
- Amended by: [ADR 0065](0065-workspaces.md): Workspaces: a server, defaults, rules and history apart for each kind of work
- Amended by: [ADR 0115](0115-the-sidebar-names-the-workspace-and-the-machine.md): The sidebar names the workspace at its top and the machine at its bottom

## Context
Today a daemon signs in to at most one server, in `server.json`, and every family it holds syncs there. People work for more than one group, though: a company's server and an open-source project's, or work and their own. With one sign-in they need a second `SILT_HOME`, with a second daemon and a second web UI. Some work shouldn't reach any server at all: a client's code, or an experiment. Today the only way to keep it off is to sign out of the server. The plan for the web UI asks for both, with a WorkspaceSwitcher in the web UI. It also leaves open whether several servers are worth it.

## Decision
**Several servers are worth it.** The cost is contained by binding each family to one server, rather than letting a family span servers. Sync, the control channel, ownership, devices and the server's view of a family stay as they are, once per server.

**Sign-ins.**
- The daemon keeps one sign-in per server in `servers.json`, a list of today's `RemoteConfig` (URL, token, device, device name, user). The first is the default.
- A `server.json` from before is read as the only sign-in, and replaced by `servers.json` when sign-ins next change.
- `silt auth login --server <url>` adds a sign-in, or renews the one for that URL. `silt auth status` lists them. `silt auth logout --server <url>` removes one.
- The device id and the user name are per server: the same machine is a device of each server, and the person may have different names on them.

**A family's home.** Every family has a *home*: one server, by its URL, or this machine only.
- Kept per family in the daemon's store (`family_homes`), since sync works per family and all of a family's threads share one causal graph.
- A family without a row has the default server as its home, so families from before keep syncing where they did. With no sign-in at all, nothing syncs, as today.
- A new thread's family gets its project's home: the project's choice if the person made one (`project_homes`, by the project's normalized origin), else the default server.
- A family pulled or resumed from a server has that server as its home. A thread listed by a server belongs to it.
- Forks stay in their family, so a fork is always where its source is.

**Per server, as before.** Each server with a sign-in gets its own listing, control channel and per-family push and pull loops. These cover only the families whose home it is. Everything that asks "the server" asks the thread's home:
- registering threads, visibility and sharing;
- the inbox, presence, search and drafts;
- merge request review, git mirror pushes and blob fetches;
- the lease and the device that holds it;
- who owns a thread.

What belongs to no thread (the inbox page, Devices, Orgs, Projects) is asked of each server, and the web UI says which server each answer came from.

**This machine only.** A family whose home is this machine never syncs. Its deltas wait in the outbox as any offline family's do, and nothing drains them. The thread page, sidebar and `silt status` say "this machine only". Sharing, links, visibility, merge requests with other people, and continuing on another device are refused with that reason.

**Moving a project to a server.**
- `silt project home <url>`, or the project's "New threads sync with" on the Projects page, sets the project's home. `silt project move` already moves a project between orgs. `silt project home local` keeps its new threads on this machine.
- It also sets the home of each family of the project that was on this machine only.
- The new home's sync loop then registers the threads and pushes the waiting history, as for a family made while offline.
- Moving a family off a server, or between servers, isn't offered: other devices and people may hold it there. Forking it into a new family elsewhere is a separate feature.

**Workspaces in the web UI.**
- A workspace is an org on a server, or "This machine".
- The WorkspaceSwitcher lists the workspaces grouped by server, with how many threads need the person in each. It also offers connecting another server, which shows the `silt auth login --server` command to run.
- Choosing a workspace narrows the sidebar, Needs you and the graph to its threads.
- Pages for one thread work wherever its home is.
- In hosted mode there's only the server's own workspaces.

## Consequences
- **Status:** `DaemonStatus` lists every sign-in, with its own sync error. `ThreadSummary` says the thread's home. A permalink (ADR 0031) points at the thread's home server, or at this machine for local threads.
- **Code:** the daemon's single `remote` becomes a set of sign-ins with a lookup by family. About thirty call sites change, mostly mechanically.
- **Hosted mode** is unchanged: a server only ever sees its own families.
- **Hand-off and safety:** a thread can't be handed off to a device that isn't signed in to its home server. The UI names the server when it offers devices. Nothing crosses servers implicitly: a family's deltas, blobs and git objects only ever go to its home.
