# 0070: An artifact's version and view in its header

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0068](0068-artifacts.md) (floating pills over the page)

## Context
ADR 0068 put an open artifact's version and view in two pills floating over the top of its page. In use they cover the top 52 px of the page. An artifact's own header, usually its title and tabs, is drawn underneath them, so every page the agent makes has to leave room it can't know about. The pills also sit apart from the header that already names the artifact. That makes two rows of controls for one thing, and the second one changes what it covers as the page scrolls.

The options were mocked up beside the thread that made this change:
- **A:** one bar.
- **B:** a slim toolbar row attached under the header.

## Decision
**One bar.** The open artifact's header holds everything:
- **The version** (`v2 ▾`) sits beside the title, as part of the name of what's shown.
- **The view** (preview and source) is a small segmented control.
- **Reload and zoom** sit before **Share**.
- **Download the file** is no longer a button of its own. It stays in Share, where it already was.

**What folds as the panel narrows.** This depends on the panel's own width (a container query), not the window's, since the panel is dragged wider or narrower:
- **576 px and wider:** everything is in the bar.
- **448–576 px,** which includes the panel's default of 480: reload and zoom (in and out) fold into **⋯**.
- **Narrower than 448 px,** as on a phone: the view and Share fold into **⋯** too. The bar keeps the title, version, comments and close, and "Artifacts" on the back button becomes just its arrow.

The page starts under the header, and nothing is drawn over it.

## Consequences
- An artifact gets its whole page. Its own header shows, and Markdown, source and diagrams no longer pad their tops to clear the pills.
- At the default width, reload and zoom take two clicks, not one. They're the controls used least.
- The header's controls are in the accessibility tree once each, by the names they had (Versions, Preview, Source, Zoom, Share), plus More for ⋯.
