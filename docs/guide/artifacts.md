# Artifacts

An agent the web UI runs can make you an **artifact**: a page, a chart, a deck, a document or a sheet that opens beside the conversation and is kept in the thread. Because it lives in the thread's history, it hands off, forks and syncs with the thread: open it on another machine or in the hosted app, fork the thread and change it there, or rewind the thread and it rewinds too. The [web UI](web-ui.md) covers opening, commenting on and sharing one; this page covers what artifacts can be and do.

## Kinds

| Kind | What it is | How it's drawn |
|---|---|---|
| `html` | A page the agent writes | In a sandboxed frame that reaches nothing of Silt's |
| `svg`, `mermaid`, `markdown`, `image` | A picture, a diagram, a document | By Silt, with nothing in them running; pictures and diagrams zoom and pan |
| `video` | A screen recording, a demo | By Silt's player, played as it arrives; no size limit |
| `slides` | A deck: slides of HTML on a 16:9 or 4:3 stage, with notes | Silt's viewer: keys, a strip of slides, **Present** with the notes, the next slide and a timer, `#n` links to a slide, a comment on the slide shown, and a PDF on a computer with Node and Playwright |
| `canvas` | Boards of mockups at their real sizes | A surface you pan, zoom and pinch; **Fit all**, a board's name to zoom to it, or **Compare** on two boards to see them side by side |
| `data` | Stats, charts and tables, each naming the source its numbers came from | Silt's charts, in the design system's colours, each with a table view and a CSV download; every number links to its source |
| `doc` | Markdown you and the agent edit at the same time | **Read**, **Rich text** and **Edit**; everyone's edits merge |
| `sheet` | Grids with formulas you and the agent edit at the same time, up to 16 sheets | Double-click a cell to type; **Row above**, **Column left**, **Add a sheet** and a tab for each, formulas that read another sheet (`=SUM(Sheet2!B2:B9)`), and `.xlsx` with every sheet and its formulas |
| `whiteboard` | Notes, arrows, sketches, sections and images you arrange together | Drag to move, resize by a corner, double-click to rename; **Add a note**, **Add an image**, **Draw**, **Connect**, **Delete**, **Download as SVG** and **Download as PNG**; everyone viewing sees each other's cursor |

Decks, canvases and data pages are JSON that Silt checks against a schema before keeping a version, and the agent revises one with a small patch rather than resending it ([ADR 0076](../adr/0076-artifact-types-slides-canvases-and-data-pages.md)). Docs, sheets and whiteboards are edited in place: what you type and what the agent writes are both kept, and they're part of the thread like everything else. A doc's **Save version** keeps it as it stands, with a note; **Compare** shows any version against it now; and its download menu gives Markdown, HTML or a PDF, printed on a computer with Node and Playwright, or through the browser's print dialog in the hosted app. **Rich text** edits the prose as it reads; leaving a paragraph, heading or list writes it back, and code, diagrams, charts and tables stay as written. To comment on a doc's text, select it in **Edit** and press **Comment**: the comment stays on that text as people edit around it. Search finds a doc's or a sheet's words as they stand now, and a hit opens it ([ADR 0077](../adr/0077-docs-and-sheets-edited-together.md), [ADR 0081](../adr/0081-live-in-a-page-rooms-and-whiteboards.md)).

## Versions

Each time the agent changes an artifact it makes a new version, which can say what changed; the version menu lists them. A page that saves itself (below) makes versions too, marked with whose they were. A doc's or a sheet's later versions are snapshots of it as it stood.

An HTML page can have files beside it, such as `data.json` or `css/site.css`, which it loads relative to itself, and later versions keep them unless the agent changes one ([ADR 0074](../adr/0074-an-artifacts-files-and-assets.md)). **Share › Download as one HTML file** carries them inside the file.

## What a page can do

An HTML page can ask for more than showing itself. It declares what it uses, and each capability works only where it's declared, served and, where it acts for you, allowed by you ([ADR 0072](../adr/0072-what-an-artifacts-page-can-do.md)):

- **`db`:** data people share, such as a poll's votes or a checklist's tasks, kept in the thread under rules the page declares (who reads and who writes each path). The server checks every write by who made it.
- **`artifact`:** the page saves itself as the next version, for the person viewing it, such as a board that remembers its cards.
- **`user`:** who's viewing, as an id that's the same everywhere.
- **`downloads`:** a file the page made, saved once you confirm.
- **`comments`:** the page opens the artifact's comment composer, or, once you allow it, comments, replies and resolves as you ([ADR 0082](../adr/0082-comments-inside-docs-and-pages.md)).
- **`sample`:** the page asks Claude Code a question, with no tools and no session, once you allow it, and can show it images and offer it tools of the page's own, which run in the page. In the hosted app one of your computers answers; a device needs the **Ask an agent a page's question** permission for that.
- **`assets`:** files people add to the page, such as photos.
- **`files`:** the thread's own files and the turns that changed them, once you allow it, within the paths the page names and outside folders your workspace denies ([ADR 0078](../adr/0078-a-page-reads-the-threads-files.md)).
- **`dictation`** and **`tts`:** the page hears you through this machine's speech helper, and reads text aloud in your browser's voices ([ADR 0079](../adr/0079-voice-in-a-page.md)).
- **`mcp`:** the page calls tools of MCP servers you set up for pages (below) ([ADR 0080](../adr/0080-a-page-calls-mcp-servers.md)).
- **`room`:** moments and presence, such as cursors, between the people viewing the page now ([ADR 0081](../adr/0081-live-in-a-page-rooms-and-whiteboards.md)).

`localStorage` works in a page, kept for you and that artifact in your own browser, and a page keeps its state when a new version arrives while it's open.

When a page first asks for something that acts for you or reads what's yours, such as `sample`, `files`, `dictation` or `mcp`, Silt asks in a bar above the page, and remembers your answer for what that version declares. A later version that declares more, such as another connector's tools, asks again, and so does a new page that asks an agent, calls connectors or reads files.

A page can't fetch from other sites, but it can still send what it reads to one by opening it, so allow only pages you trust with what you let them read.

## Connectors for pages

**Settings › Appearance › Connectors for pages** lists the MCP servers pages may call on this computer. **Import from Claude Code** copies the servers Claude Code knows, with every tool off; open a server's **Tools** to tick the ones pages may use. A tool the server marks as only reading may refresh on its own; any other tool asks you each time a page runs it. An HTTP server's token goes in this computer's keyring, the Keychain, Windows' Credential Manager or a Secret Service: under the server, **Add a secret header**, give its header, usually `Authorization`, and its value. Its headers can also come from environment variables you name. Either way no secret is kept in Silt's files. A computer with no keyring, as a server without a desktop, says so when you try.

## The Artifacts page

**Artifacts** in the sidebar lists every artifact in the threads you can read, newest first, one row each. Filter by kind, project, your own threads or words in the title and description. **Pin** keeps one first, for you alone. The thread's owner can **Archive** one from its **Share** menu, which hides it from the thread's Artifacts tab and this page for everyone, until unarchived; it stays in the history.

Each artifact has a link of its own, `/t/<thread>/artifacts/<id>`, and `/v/<n>` for a version. **Share › Copy link** copies it.

## Checking a palette

`silt design palette` checks a chart's colours: lightness, chroma, and how far apart they stay under normal vision and colour-vision deficiencies; `silt design contrast` gives a text colour's contrast against its background. The agent's charting guidance uses them, and its default palette for a page with no design system passes them.
