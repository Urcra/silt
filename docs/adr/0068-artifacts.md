# 0068: Artifacts: what an agent shows the person, beside the thread

- Status: accepted
- Date: 2026-11-14
- Amends: [ADR 0011](0011-history-tools-search-and-web-ui.md) (Silt's MCP tools), [ADR 0027](0027-read-only-links.md) (read-only links), [ADR 0055](0055-reviewing-a-threads-changes.md) (comments sent to the agent)
- Amended by: [ADR 0070](0070-an-artifacts-controls-in-its-header.md): the version and the view move from pills floating over the page into its header
- Amended by: [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md): the frame loads libraries and fonts from a CDN allowlist, takes a design system's tokens and the viewer's theme, and artifacts are no longer redacted
- Amended by: [ADR 0072](0072-what-an-artifacts-page-can-do.md): a page can use capabilities through the holding page
- Amended by: [ADR 0073](0073-revising-artifacts-mermaid-in-pages-and-checked-palettes.md): `edits`, notes and descriptions on versions, Mermaid in pages
- Amended by: [ADR 0074](0074-an-artifacts-files-and-assets.md): frames are served from a frame address, with a version's files and the artifact's assets
- Amended by: [ADR 0075](0075-every-artifact-in-one-place.md): every artifact is listed in one place
- Amended by: [ADR 0076](0076-artifact-types-slides-canvases-and-data-pages.md): artifact types drawn by Silt: slides, canvases and data pages
- Amended by: [ADR 0082](0082-comments-inside-docs-and-pages.md): Comments inside docs and pages
- Amended by: [ADR 0121](0121-videos-and-images-without-a-size-limit.md): Videos, and images without a size limit

## Context
An agent the web UI runs can only answer in text. Asked for a chart, a diagram or a screenshot, it writes a file the person never sees. A picture shows only if the agent reads it with a tool, and then only folded inside that tool's step.

A thread hands off between machines, forks and merges back, and syncs to a server the person runs. What an agent shows has to go with the thread, with its versions and comments, and without a host of anyone else's.

## Decision
**Two MCP tools, only for agents the web UI runs.**
- `silt_show { path, caption? }` shows an image in the conversation.
- `silt_artifact { title, kind, content | path, id? }` makes an artifact, or with `id` adds a version. `kind` is `html`, `svg`, `markdown`, `mermaid` or `image`.

Only the daemon starts `silt mcp` with `--ui`, for the agents it runs for the web UI, the hosted app and phones. Without `--ui` the tools aren't listed. `silt claude` and other terminal runs never see them, so an agent there never shows something nobody is looking at.

**The daemon records; the MCP process only asks.**
- The tool sends `PutArtifact` with the run's token.
- The daemon takes the thread from the token, not from the call, and refuses unless that thread has an agent the daemon runs.
- **What a path may name:**
  - It resolves against the run's checkout.
  - It must be a regular file of at most 10 MiB.
  - It must not be in Silt's data dir or a folder the workspace denies (ADR 0065).
- Text is redacted before it's kept, as prompt attachments are.

**Each version is a blob named in the thread's history.**
- The daemon keeps the content as a blob, attached beside the run's session (`ConvOp::Attachment`, named `artifact/<id>/v<n>/<file>`), which carries it through sync.
- An artifact record line in the session names it: `{"t":"artifact", id, version, title, kind, media_type, blob, size, inline}`.
- So an artifact hands off, forks and is searched (by its title) with the thread, and goes to the server like the rest of the history.
- It never shows in the thread's changes and never lands in a commit or a merge.
- Versions count up per id along the thread's lineage, so a fork continues from what it inherited.
- An id is 12 hex characters, checked before it becomes part of a name (ADR 0016).

Neither the content nor the record needs a new delta.

**Rendering treats the content as hostile.** It comes from an agent, possibly on another person's machine.
- **HTML** renders in an iframe from `srcdoc`, with `sandbox="allow-scripts"` and never `allow-same-origin`. Its origin is opaque, so it can't reach the daemon's `/api` or the server's cookies. A CSP meta allows no network.
- **SVG, Mermaid and images** render as `<img>` from a data URL, where no script runs. Mermaid runs in the page with `securityLevel: 'strict'` and its SVG is shown the same way.
- **Markdown** renders with the web UI's own renderer, which allows no raw HTML.
- The content comes as JSON from `/api/threads/{id}/artifacts/{blob}`, or `/app/api/…` on the server. Each serves only a blob that an artifact of that thread names.

**The web UI.** It follows the mocks:
- **In the conversation:** images shown inline; artifacts as cards.
- **Artifacts tab:** a new tab in the thread's panel. An open artifact's header replaces the tab row, so there is one bar.
- **Floating pills over the page:** the version and the view. ADR 0070 moved them into the header.
- **Full view:** the panel takes the conversation's space too.
- **On a desktop:** a new artifact or version arriving live opens itself.
- **On a phone:** a full-screen sheet.

**Comments on an artifact go to the agent.**
- A comment on an artifact is a comment on the history entry of the version it was made on, so the daemon and the server take it as they take any comment on a message. The text selected in the artifact goes into its words, quoted.
- Send to the agent turns the artifact's waiting comments into one prompt asking for a new version, and marks them sent, as a review does (ADR 0055). The server records them sent too, for the hosted app.
- Nothing about this changes the wire format, so the protocol stays at 10.

**Sharing.**
- **A read-only link to one artifact.** It's a thread link (ADR 0027) that names the artifact and either a version or "the latest", with the same expiry, revocation and org rules.
  - `/r/<token>` shows the artifact.
  - Its content is served from `/r/<token>/content` with `Content-Security-Policy: sandbox allow-scripts` and no network.
  - Like every read-only link, it needs a server.
- **An offline copy.** Download as HTML writes one self-contained file, which works anywhere without Silt.

## Consequences
- An agent can show a person what it made, on any machine the thread reaches, without a third-party host.
- `silt mcp` takes `--ui`, and the daemon takes `PutArtifact`. The scoped run token (ADR 0065) now allows one write, and only for the token's own thread.
- Agents in a terminal can't make artifacts. A thread that has artifacts from an earlier web UI turn still shows them.
- Large artifacts sync like other attachments, so a 10 MiB page is 10 MiB on the server. Older versions stay, as the history does.
- A server older than this one lists artifacts' comments as comments on messages; it can't serve artifact links.
- Artifact links need a server; on one machine alone, the offline copy is the way to share.
- An agent can't read an artifact back through Silt yet, so after a handoff it edits the file it has, or writes the next version afresh.
