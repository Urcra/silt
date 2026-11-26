# 0127: Switching threads waits for nothing else

- Status: accepted
- Date: 2026-11-25
- Amends: [ADR 0117](0117-conversations-open-at-their-end.md) (where a history is read whole, and how many are kept), [ADR 0021](0021-browser-event-bus.md) (what the web UI stays subscribed to)

## Context
Switching to a thread sometimes showed its loading skeleton for seconds, and switching back to one sometimes showed it as it was when left, a prompt sent since still greyed out, until a read caught up.

The daemon's core handles one event at a time: hooks, watchers' changes, sync and every web request. A thread whose history it didn't keep was read whole there. On a 4-core Linux machine, in the perf build, with the 100 MB session `silt-bench` records (38,811 entries), the first page of its thread took 0.8 to 1 s:

- 230 ms expanding its history, which read the family's log back from the store when it had been let go of;
- 660 ms reading every transcript line into entries.

Every other request waited as long: the thread listing, asked every 10 ms meanwhile, waited 754 to 946 ms. So did the reads of the thread being switched from, and capture. A Windows machine, or a longer session, waits longer.

Three things made it frequent:

- **The daemon kept 16 histories and cleared all of them** when a 17th was read, so after looking through 17 threads every switch read one whole again.
- **The web UI let go of a thread's history 5 minutes after leaving it,** React Query's default, so going back showed the skeleton however quickly the daemon answered.
- **Going back to a thread showed what the page held** while its read caught up. Nothing told the page what had changed while it was away, and the read waited behind whatever the core was doing.

## Decision
**A history is never read whole on the core for a request.** A request for a thread's history or turns that the daemon doesn't keep waits, as one waits for a family to load (ADR 0116), while it's read elsewhere, and the core goes on:

- for a family not held, the loader reads the histories asked for in the family it loads, before the core holds it;
- for a family held whose log was let go of, the loader reads it in a copy it loads. A log is let go of only while the store holds just its deltas, so the copy's is the same, and the read is kept only if the log hasn't grown since;
- otherwise a blocking thread reads it from deltas cloned off the family, 113 ms of the core for that session instead of 900.

What the log added meanwhile is read on from it when the request goes on. Other requests that read a history whole, such as search within a thread, still read it on the core.

**The 16 histories kept go least recently read first,** one at a time.

**The web UI keeps the 8 threads left most recently warm.** Their topics stay subscribed, and their histories are read on as they change, though not shown. A thread let go of has its history dropped, so the page never shows one nobody kept current. A hosted page has no event socket, and the server reads a history again whole, so it keeps none.

**An event that holds the core for 250 ms or more is logged** with its kind, never its fields, so what else holds it on a person's machine can be found.

## Consequences
- With the same session, the listing waits at most about 110 ms while a thread whose history isn't kept opens, where it waited a second.
- Opening that thread still takes as long as reading its history whole, about a second for that session, but only it waits.
- Going back to a thread left lately shows it as it is at once.
- Up to 8 threads not shown are read on as they change: those an agent works in, whose families are held anyway.
- A hook's capture and watchers' changes still run on the core; one that holds it long is now in the log.
