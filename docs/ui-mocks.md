# What the web UI mocks

The design boards draw some things the daemon and server can't tell yet. [ADR 0039](adr/0039-the-web-ui-as-designed.md) builds those screens in full anyway. Wherever data is missing, the UI shows a mock in its place. This page lists every mock, and what it takes to remove it and show real data.

- **Where mocks live:** `web/src/mocks.ts` registers each one. In the page, a mocked element carries `data-mock="<id>"`, so `document.querySelectorAll('[data-mock]')` finds them all.
- **Who sees them:** the dev server (`pnpm -C web dev`) and a build with `VITE_SILT_MOCKS=on`, as the gate's is for its screen references. Every other build, a release's or `cargo install`'s, leaves mocked parts out.
- **Labels go with them:** a mock wraps its value's label too (`<Mock>` around the whole row, `mockRows` for key-value rows), so a build without mocks has no label left without a value.
- **Kept honest:** `web/src/mocks.test.ts` fails when the code registers a mock this table doesn't list, or this table lists one the code doesn't have.

| Mock | Screen | What it shows | What replacing it needs |
|---|---|---|---|
| `new.device` | New thread, the "On" chip | This machine's name, as if a device could be chosen | A thread starts only in a clone on this machine (`startAgent` takes a clone path). Starting on another device means asking it through the server's control channel (ADR 0026), as "Continue there" does for an existing thread, with its clones listed per device |
| `reconcile.checkout` | Reconcile, this machine's side | "the clone on laptop now follows 2c8e6f1" | `recover_diverged` moves each checkout with `move_work` but records only a note. Add the moved checkouts' paths to `RecoveryView` |
| `reconcile.agent` | Reconcile, this machine's side | "claude session moved with it" | The same: record whether a session moved with the checkout (`move_work` moves `sessions` too), and its agent, in `RecoveryView` |
| `search.saved` | Search, the Saved list | Two sample searches that set their filters and run | Nothing stores searches. Keep them as per-person state on the server (ADR 0024), next to pinned and snoozed threads, with a "Save this search" action on the page |
| `share.link-opens` | Share a thread, read-only links | "opened 3 times" beside each live link | The server serves `/r/<token>` without counting. Count each page opened per link in its `thread_links` row, and return the count in `ThreadLink` |
| `thread.handoff-before` | Thread, inspector, Handoff | "Before: laptop · 2h ago": the device the thread was on before it moved here, and when | The server holds one lease per thread (ADR 0009) but keeps no record of earlier holders. Record each lease taken (device, time) in the family log or the server's lease table, and add the latest previous holder to `ThreadSummary` |
| `add.discovered` | Add a project, clones on this machine | A sample clone ("~/code/scratch", no remote) with a disabled "Add here only" | Nothing looks for clones. Have the daemon scan the folders around known clones (and a configured list) for git repositories, with each one's remote, and serve them beside the projects |
| `add.visibility` | Add a project, defaults | "Visible to: everyone in the org (org default)", disabled | Visibility is set per thread and per org (`default_visibility`), not per project. Add a project-level default to `OrgChange::project_settings` and `Project` |
| `add.prepare` | Add a project, defaults | "run `.agents/prepare`, found in the repository" | New checkouts don't run anything yet. Decide where a prepare script lives (`.agents/prepare` in the repository), run it in each new checkout, and report whether a project has one in `Project` |
| `devices.checkouts` | Devices, each machine | "Checkouts: 3" | `DeviceView` has the agents a device reports but not its checkouts. Have daemons report their checkout count with their agents (the report the server already takes for running agents, ADR 0026) |
| `devices.version` | Devices, each machine | "Daemon: 0.0.1 · synced" | The same report can carry the daemon's version (now in `DaemonStatus.version`) and whether its sync is caught up; the server keeps the last one per device |
| `devices.pair-capabilities` | Devices, Pair a phone | What the phone may do, shown before pairing but fixed | `pairPhone()` takes no arguments, and a paired phone starts with answer and review. Let the pairing carry its capabilities, so they can be chosen before the code is shown |
| `graph.device` | History graph, the selected row | "On: workstation · 1h ago" | Deltas carry an author (the actor) but the graph doesn't map actors to devices. Record each checkout actor's device when it registers, and add the device to `GraphRow` |
| `live.agent-editing` | Live session, editor toolbar | "claude is editing" while the agent works, on whichever file is open | `AgentActivity` carries no file or line. Report the files the running tool touches (the daemon has them as `InFlight.files`) and the line of its last edit in the agent's status, so the editor can mark the file in the list and the line in the editor |
| `live.sync-devices` | Live session, status bar | "in sync with 3 devices", counting the people here | The replica knows only its own state. Have presence say which delta each device has applied, so the bar can count the devices caught up and name one that lags |
| `thread.synced` | Thread, inspector, Handoff | "Synced: all 214 entries" | `ClientSync` knows what was pushed and pulled. Expose its pushed and total counts per thread through `ThreadSummary` (or `LiveState`), so the page can say "all synced" or "12 waiting to push" |

## The site's placeholders

The site's landing page and docs ([ADR 0033](adr/0033-landing-page-and-site.md)) follow their boards, but some of their words are placeholders until the docs for people and agents are written. Each placeholder is marked `<!-- placeholder -->` in the page or its template.

- **Pages the Docs board lists:** its Install, Quickstart, Fork and merge, Review and search and Phone pages are the guide's index (install and first steps), [forks and merge requests](guide/forks-and-merge-requests.md), [history and search](guide/history-and-search.md) and [the hosted app and phones](guide/hosted-app-and-phones.md), which the sidebar's groups in `crates/silt-site/src/lib.rs` list.
- **The landing page's own sections** in `crates/silt-site/src/lib.rs` (`landing_document`): How it works (its four steps), Works with (the agents' names), Agents from the browser (its list and the sample card), Self-host (its list and terminal), Where it is (what works today, next and later, which should follow `ROADMAP.md`), and the closing call to action. Moving their words into `docs/landing.md`, beside the features, removes them from the template.
- **The install command** in the landing page's hero, which waits for a released install path.
