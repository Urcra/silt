# 0031: Permalinks

- Status: accepted
- Date: 2026-10-21
- Amended by: [ADR 0075](0075-every-artifact-in-one-place.md): permalinks to an artifact and to one of its versions

## Context
People paste links to work into chats, reviews and issues. The plan for the web UI asks for stable URLs to a thread, a turn, a comment, a file's lines at a version, and a merge request's file, working in both serving modes (ADR 0026). Two things make that harder than it sounds:
- a file's content depends on the version, and "now" moves on;
- the server has every delta, but not the git texts most files start from. A thread's base commit usually comes from origin, so daemons never bundle or mirror it (ADR 0009).

## Decision
**URLs.** All of them are paths in the app, so they mean the same on a daemon and on the server's `/app/`:
- a thread: `/t/<thread>`;
- a turn: `/t/<thread>#turn-N`, numbered as `silt diff --turn` numbers turns;
- a comment: `/t/<thread>#comment-<id>`;
- a file's lines: `/t/<thread>/files/<path>?at=<version>#L4-L9`;
- a merge request's file: `/mr/<id>#file-<path>`.

The thread page and the merge request page follow the hash once what it names has loaded, scroll there, and highlight it.

**Versions.** `at` is a frontier: its delta ids as `actor.lamport`, comma-separated. A link without one shows the thread's head. "Copy link" on a file always pins the version it showed, so the link keeps showing those lines. A version only counts within the thread's own history. A link to one thread can't show a file as another thread of the family had it, such as a private fork's.

**Which server.** "Copy link" on a machine that's signed in to a server copies the server's `/app/` URL, since nobody else can open this machine's. Otherwise it copies the page's own URL.

**Files in hosted mode.** The server doesn't keep a replica, and it lacks the git texts edits are based on. So it answers a file link like a merge request's diff (ADR 0026): it asks one of the person's online devices that holds the family, with `RemoteCommand::FileAt`. That needs the device's `review` capability. With no device online, the link says so. Turns, comments and merge requests come from the server's own copy as before.

## Consequences
- A file link opened in hosted mode needs one of the reader's machines online. The read-only link page (ADR 0027) has no file view for the same reason.
- `Frontier` has a text form, `Display` and `FromStr`, in `silt-types`.
- Versions in links are long. They stay readable for a single head, which is the usual case.
- Hash links inside the app go through the router: the page's `<base>` would otherwise resolve a bare `#…` against the app's root.
