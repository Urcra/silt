# 0076: Artifact types: slides, design canvases and data pages, drawn by Silt

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0068](0068-artifacts.md) (the kinds), [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md) (the guide)
- Amended by: [ADR 0077](0077-docs-and-sheets-edited-together.md): `doc` and `sheet`
- Amended by: [ADR 0080](0080-a-page-calls-mcp-servers.md): data pages' sources can be MCP tools
- Amended by: [ADR 0081](0081-live-in-a-page-rooms-and-whiteboards.md): `whiteboard`
- Amended by: [ADR 0084](0084-data-pages-live-from-a-connector.md): A data page's numbers live from a connector

## Context
Every artifact in Silt is one page or one picture. Some requests are better served by something with its own structure and controls:
- **A deck** to present: fixed-size slides, paging, full screen, speaker notes, a PDF.
- **A design canvas** of artboards at their real sizes, side by side.
- **A data page** whose numbers say where they came from.

An agent can fake each as an HTML page, but then the deck has no presenter view, the canvas no pan and zoom, and a number on a data page is just text.

Each needs a viewer that takes the agent's content as data and draws the controls itself. Silt can't run viewers published by others; a type's code would be peer content (ADR 0016). It can ship its own.

## Decision
**A type is a kind whose content is JSON, drawn by a viewer built into Silt.**
- **The first types:** `slides`, `canvas` and `data`. `doc` and `sheet` follow in ADR 0077, and `whiteboard` in ADR 0081.
- **Checking:** each type has a JSON Schema in `silt-proto` (`silt_proto::artifact_types`), checked by one pure function before a version is kept, in the daemon and the server alike. A version that fails is refused with the schema's error.
- **Guidance:** `silt_artifact_guide { kind }` returns the type's schema, its guidance and an example. A built-in skill per type (`artifact-slides`, `artifact-canvas`, `artifact-data`) carries the craft.
- **Revising:** `silt_artifact` takes `patch`, an RFC 6902 JSON Patch, for a type's content, as `edits` (ADR 0073) are for text. Adding slide 4 or moving a board is one small call.
- **What's shared with every artifact:** versions, comments, the design system and its theme, read-only links, the gallery (ADR 0075) and the offline copy.
- **Where the viewers run:** in the web UI's own bundle, so the daemon, the hosted app and links all have them. A type's own code is Silt's, reviewed and shipped like the rest.

**Where agent-written HTML still goes.**
- Slides and boards hold HTML. Each slide or board renders in a sealed frame (ADR 0068, ADR 0074), never in the viewer's page.
- Data pages hold no HTML: Silt draws them from the data alone.

**`slides`: a deck.**
- **Content:** `{aspect: "16:9" | "4:3", style?, slides: [{title, html, notes?}]}`. `style` is CSS shared by every slide.
- **Drawing it:** each slide is composed into a fixed stage, 1280×720 or 1024×768, scaled to fit. The frame and its policy are a page's, and all the slides share one frame, so a deck's script and fonts load once.
- **The viewer's controls (Silt's, outside the frame):**
  - previous and next, by click, arrow keys and swipe;
  - a strip of slide thumbnails;
  - `Present`, which goes full screen, with a presenter view showing the notes, the next slide and a timer;
  - a link to slide n (`#n`).
- **Comments** are on a slide, quoting the text selected in it.
- **Exporting:**
  - **PDF:** one page per slide, rendered by the daemon's headless browser where one is installed, otherwise not offered;
  - **the offline copy:** one HTML file with the viewer's controls.
  - Silt doesn't make PowerPoint files.

**`canvas`: artboards for designs and mockups.**
- **Content:** `{style?, boards: [{id, name, caption?, width, height, x?, y?, html}]}`. Boards without a position are laid out in a row, in order, 80 px apart.
- **Drawing it:**
  - a canvas that pans and zooms (wheel, pinch, space-drag, "fit all", "zoom to board");
  - each board in its own sealed frame at its real size, under its name and caption;
  - frames off screen are unloaded, and a placeholder of the board's size shows instead.
- **Opening a board:** at 100% in the full view, or at a phone's width on a phone. Two selected boards open side by side to compare.
- **Comments** are on a board.
- **Guidance:** `artifact-design`'s "Mockups" section moves to `artifact-canvas`, and `artifact-design` points there for any mockup with more than one state.

**`data`: numbers with their sources.**
- **Content:**
  - `{title, summary?, sources, blocks}`;
  - each source is `{id, name, kind: "query" | "file" | "command" | "agent" | "manual", detail, at}`: what was run or read, and when;
  - each block is a `stat`, `chart`, `table` or `text`, and every number-bearing block names its `source`.
- **Charts:** `line`, `area`, `bar` (grouped or stacked), `dot`, `scatter`, `heatmap` or `segmented`, with series, x and y.
- **Drawing it:**
  - Silt's own components draw every block, following `artifact-dataviz`: one scale, thin marks, the design system's chart tokens, a hover readout, and a table view for each chart;
  - each number links to its source, showing what produced it and when;
  - the agent gives values, not colours or layout code, so every data page reads as one system.
- **Freshness:** a data page doesn't refresh itself. The agent makes a new version when asked, and the page shows each source's time. ADR 0080 lets a source be an MCP tool that the viewer's daemon re-runs.
- **Exporting:** a chart's table as CSV, through `downloads` (ADR 0072).

## Consequences
- **Decks, design canvases and data pages are first-class,** with controls Silt maintains and agents don't rewrite each time.
- **Data pages are safe by construction:** no agent code runs in them. Slides and boards keep the sealed frame.
- **Each type is code Silt ships and maintains:** a viewer, a schema, a skill and tests. Types are added one at a time, in the order above.
- **PDF export needs a headless browser,** as `silt_artifact_check` does.
- **A kind is a string on the record,** so new kinds need no wire change. But an older web UI draws an unknown kind as an image, which fails, so a type shows properly only where this release serves the UI. From now on the UI shows an unknown kind as its source.
