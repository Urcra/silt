# 0039: The web UI as designed

- Status: accepted
- Date: 2026-10-22
- Amends: [ADR 0020](0020-next-generation-web-ui.md) (which boards fold into the workspace)
- Amended by: [ADR 0054](0054-the-desktop-ui-review.md) (where the desktop screens now differ from their boards)
- Amended by: [ADR 0044](0044-the-standard-agent-workspace.md): The standard agent workspace: open on the composer, one shell for every screen
- Amended by: [ADR 0098](0098-themes-for-silts-ui.md): Themes for Silt's UI

## Context
M11 to M17 built what the "Silt Web Interface" canvas asks for, but not how it draws it. Every view sits in one narrow column under a top bar. Messages are plain text, tool calls are rows of their own, diffs are unified only, and some boards have no screen at all (Reconcile). The canvas has a board for each screen on the Nous design system.

Some of what the boards draw has no data behind it yet. Examples are a structured record of a recovered thread, where a thread was before its last handoff, the line an agent is editing, and how often a link was opened.

## Decision
**Boards map to routes in one shell.** The shell is drawn by the Workspace board:
- a top bar with the crumb, the palette field, the device pill and the avatar;
- the project sidebar;
- the view;
- an inspector on the right where the view has one;
- a status bar.

On a phone the sidebar is a drawer, and a tab bar takes the status bar's place.

| Board | Route |
|---|---|
| Threads (Main) | `/`, the workspace home: Needs-you cards and threads by project. The board's own sidebar items go to the shell's sidebar |
| Agent workspace, Thread | `/t/$id`: one page. The transcript and composer are in the middle, and the inspector on the right holds changes, files, lineage, commits, comments and handoff. With no agent running it is the Thread board; with one running, the Workspace board |
| Live session | `/t/$id/live` |
| Merge request | `/mr/$id` |
| Share a thread | a dialog over `/t/$id` |
| Reconcile a diverged thread | `/t/$id/reconcile`, for a recovery thread |
| Search | `/search` |
| Inbox | `/inbox` |
| History graph | `/graph` |
| New thread | `/new` |
| Add a project | `/projects/add` |
| Devices and phone pairing | `/devices` |
| Switch workspace | a popover from the sidebar's workspace header |
| Command palette | ⌘K over any route |
| Sign in | the server's `/login` page, which stays server-rendered |
| Phone — needs you | `/needs-you` at phone width |
| Phone — review a diff | `/mr/$id` at phone width: one file at a time |
| Landing page, Docs | `silt-site`'s `index.html` and guide pages |

**What is not built from the boards:**
- **Sandbox wording.** This covers "sandboxed", "Allow with network" and "Sandboxed autopilot". ADR 0017 and ADR 0020 removed Silt's sandbox, so permission cards and presets offer what the agent offers.
- **"Was this page useful?"** on the docs. It would need a server to count.
- **Landing page v2 (teams).** ADR 0033 chose one landing page.

**Drawn differently, on purpose:**
- **Ids are 12 characters,** as `silt log` prints them, not the boards' 7.
- **Shortcuts the browser keeps** move elsewhere. ⌘1–⌘4 become Alt+1–4, and ⌘N becomes N.
- **A thread's diff opens in the inspector** (Changes, or D) rather than always sitting beside the conversation. The Thread board's Conversation/Changes/Comments tabs are that inspector.
- **Permission choices** are the agent's own words ("Allow", "Reject").
- **On a phone,** a thread's terminal commands and its Live session aren't offered.
- **The site is plain files**, so a few parts of the Docs and Landing boards wait:
  - the docs' search field, which needs an index built with the site;
  - "Open Silt", which has no one address to open, so the docs' top bar offers Install instead;
  - each page's "Updated" date;
  - the landing page's product picture and its drawing of a thread's lineage.
- **The site takes IBM Plex from Google Fonts,** because it ships no binaries. The app and the server's pages don't.

**Nous in the app:**
- **One token source,** `web/src/nous.css`, with dark values written once.
  - The web app imports it.
  - silt-server's pages and silt-site include it with `include_str!`.
- **Two contrast fixes are kept.** `--fg-3` is darker than the design's `paper-500`, and text on amber is dark, because the design's values fail 4.5:1.
- **IBM Plex Mono stands in for Lilex.** The fonts are shipped with the app, not loaded from Google.
- **Icons:** the app keeps its own 16 px stroke icons, drawn like the boards' inline SVG.
- **Board classes become React primitives:** TopBar, SidebarSection, StatusBar, Pill, Dot, Tabs, DiffBlock, and the button variants.

**Data the boards draw that doesn't exist yet is mocked, visibly to developers and in one place:**
- `mocked(id, value)` in `web/src/mocks.ts` gives a mock's value.
- Each mocked element carries `data-mock="<id>"`.
- Only the dev server and a build with `VITE_SILT_MOCKS=on`, as the gate's is, show mocks. Every other build, a release's or `cargo install`'s, leaves each mocked element out, so nobody who installs Silt sees made-up data.
- [docs/ui-mocks.md](../ui-mocks.md) lists every mock, what it shows and what replacing it with real data needs. A test fails when the code has a mock the doc doesn't list.
- Small additions to existing views are made rather than mocked. Examples are counts and ids the daemon already knows.

**Screens are checked against their boards in two ways:**
- **References:** Playwright takes a screenshot of every screen on a fixed fixture, light and dark, at desktop (1440×900) and phone (390×844) width. The screenshots are kept in the repository, and the gate fails when a screen drifts from them. A deliberate change updates them (`pnpm -C web e2e:update`), and review compares the new reference against its board.
- **Critique:** while a screen is built, a separate reviewer compares it against its board and lists the differences. Each difference is fixed or recorded here or in the mocks doc.

## Consequences
- The thread page and the workspace become one page. The old Thread board's inspector holds what the old page stacked above the history.
- Every mock is debt with a written way out. M20 is done with mocks in place; replacing them is later work, each mock its own task.
- The screen references make the gate fail on any visual change, including an intended one. Updating them is part of the change.
