# 0075: Every artifact in one place

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0068](0068-artifacts.md) (artifacts reached only from their thread), [ADR 0031](0031-permalinks.md) (permalinks)

## Context
An artifact can only be reached from its own thread's Artifacts tab, or found by searching for its title. To find last week's benchmark report or a mockup, someone has to remember which thread made it. A person needs one place listing every artifact they can open, most recent first, with a title, a subtitle and a thumbnail, and a way to pin the ones they keep coming back to.

Artifacts are already in each thread's history. The daemon holds every family it syncs, and the server indexes each thread's entries. Both can list them; nothing does.

## Decision
**An Artifacts page, in both serving modes.**
- **Where:** `/artifacts` in the web UI, from the sidebar. The route is `GET /api/artifacts`, or `/app/api/artifacts` on the server, answering `silt_proto::view::ArtifactListing`.
- **One row per artifact, not per version:**
  - the title and description (ADR 0073);
  - the kind;
  - the thread and its project;
  - the latest version's number, its time and its author (an agent, or the person whose page saved it, ADR 0072);
  - a thumbnail.
- **Order:** the most recently changed first. Filters: project, kind, "made in my threads", and text, matching the title and description.
- **Who sees what:** only artifacts in threads the viewer can read, by the same visibility rules as threads (ADR 0013).
- **Read-only links** show only their one artifact, never this page.

**Where the listing comes from.**
- **The daemon** folds it from the families it holds, with the readers the thread view uses.
- **The server** keeps an index table, `artifacts`, filled when entries are indexed, with a migration.
- **Freshness:** both update as new entries arrive, so the page is live like the thread list.

**Thumbnails, where a browser is installed.**
- **When:** after a version is put, the daemon that put it renders a thumbnail in the background with the same headless browser `silt_artifact_check` uses.
- **What:** light theme, 1280×800 scaled to 640×400, as WebP of at most 100 KiB.
- **Where it's kept:** a blob attached beside the session at `artifact/<id>/v<n>/thumbnail.webp`, named on the record, so other machines and the server show it too.
- **Without a browser,** a version has no thumbnail. The page then draws the kind's icon on the design system's surface, or the image itself for `image` and `svg` versions.
- **What it renders:** only the version's own content, in the same sealed frame, so a thumbnail can't show more than the page does.

**Pins and archiving.**
- **Pins:** pinning an artifact keeps it at the top of the page for that person. Pins are per-person state (ADR 0024), kept in `artifact_pins` and synced through the person's servers like `thread_state`.
- **Archiving:** the thread's owner can archive an artifact. A record line in the session (`{"t":"artifact-archived", id}`) hides it from the Artifacts tab and from this page for everyone.
  - It stays in the history, and unarchiving it is another record line.
  - Nothing is deleted. Purging the thread removes its artifacts with it, as before.

**Permalinks.**
- `/t/<thread>/artifacts/<id>` opens the latest version, and `/t/<thread>/artifacts/<id>/v<n>` a given one (ADR 0031).
- The Artifacts page, cards and Share all use them.

## Consequences
- **A person finds an artifact without remembering its thread,** across every project on the machine, or on the server in the hosted app.
- **The server gets a new index table and a migration.** The daemon needs none.
- **Thumbnails are another attachment per version,** at most 100 KiB each, and only where a browser is installed. Machines without one show icons.
- **Pins sync as personal state does.** That's new on the wire, so it ships with protocol 13 alongside ADR 0072's `Op::Page`.
- **Archiving hides without deleting,** so a fork's inherited history stays whole.
