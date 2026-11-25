# 0107: Text and layout

- Status: accepted
- Date: 2026-11-19
- Amends: [ADR 0098](0098-themes-for-silts-ui.md) (the appearance gains text and layout), [ADR 0049](0049-the-agents-work-grouped-by-turn-or-by-run.md) (detail and grouping are kept per machine)

## Context
Themes (ADR 0098) set Silt's colours and fonts, but nothing set how big anything is. Every size was in pixels:
- the interface at 13 px, replies at 13, code at 12 and metadata at 11;
- about 30 class names with sizes of their own, such as `text-[15px]` for every heading;
- a 720 px thread column and 30 px sidebar rows.

In a browser, its own zoom was the only way to bigger text. The desktop app had no zoom at all: `Ctrl+=` did nothing. A few display choices did exist, but each sat on one page's toolbar and was kept per browser: a transcript's detail and grouping, and a diff's layout.

People want to change four things, each on its own:
- whole-window zoom on `Ctrl+=`, `Ctrl+-` and `Ctrl+0`;
- text sizes for the interface, the conversation and code, so the conversation can be bigger without everything else;
- density apart from text size;
- fonts apart from the colour theme.

## Decision
**Text and layout is part of the appearance.** `AppearanceSettings` gains `text`, a `silt_theme::TextSettings`. The daemon keeps it per machine in `appearance.json` beside the theme pick, since a laptop and a large monitor want different sizes. The hosted app keeps it in the browser. Settings has a section of its own, **Text and layout**, beside Appearance:
- **Zoom**, 50–200% in steps of 10, in the desktop app only, where nothing else zooms. A browser keeps its own.
- **Interface, conversation and code text**: 11–18, 11–22 and 10–20 px, by default 13, 13 and 12.
- **Interface font and code font**: the system's, or a name. Either wins over the theme's, since fonts are chosen apart from colours. **Ligatures** in code, on or off.
- **Density** (compact, regular, roomy), **conversation line height** (1.45, 1.6, 1.8) and **conversation width** (640, 720, 880 px, full).
- **Diff layout**: auto, unified or split. Auto is today's choice for each page. Each diff can still switch. **Wrap long lines** in a unified diff, and **tab width**, by default 4.
- **Detail** and **grouping** of a thread's work. They move from the browser to the machine, and a thread's own menu changes the same setting. Each browser's old choice moves over once.
- **Reduce motion**: as the system asks, always, or never.

**Each setting left out is Silt's default,** so a default can change later. The UI writes a value equal to the default as left out, and Reset leaves it out.

**Read leniently, compiled, never passed through.** As with themes, the file is the person's to edit by hand:
- sizes are newtypes clamped to their range, and zoom is kept to its steps;
- a font is `system` or a name checked as a theme's are;
- every other value is a closed enum;
- a value that can't be read is left out, and a `text` that can't be read at all is empty, so a bad edit never loses the theme pick.

`silt_theme::text::css` builds one block of variables from the checked values:
- `--ui-size`, `--chat-size` and `--code-size`;
- `--font-sans` and `--font-mono`;
- `--code-ligatures`, `--row`, `--density`, `--chat-leading`, `--read-width` and `--tab-size`.

Its selector is `:root:root:root`, which outranks the theme's `:root:not([data-theme=…])` wherever it lands, so a chosen font beats the theme's. It follows the theme's CSS in the same `<style>`, which the daemon puts into `index.html`, so the first frame is already at the right size. Motion is `data-motion` on `<html>`. `nous.css` sets the defaults, and its reduced-motion rule yields to `data-motion`.

**The web UI's scale is computed from those variables.**
- Tailwind's `text-2xs` to `text-2xl` are shares of `--ui-size`, at the same 11–24 px by default.
- `text-chat` and `text-code` are the conversation's and code's sizes. Markdown, the person's messages, code blocks, diffs and tool output use them.
- The literal sizes became named steps of the scale (`text-heading`, `text-label`, `text-output`, …), and a unit test refuses a new `text-[Npx]`. A whiteboard's notes are content and stay as drawn.
- The thread column is `max-w-(--read-width)`, sidebar rows are `h-(--row)`, and a turn's spacing is scaled by `--density`.
- Wrapping lives in the diff's React tree, since it changes the grid as well as `white-space`. Side by side, each half always wraps.

**A page changing the settings asks for their CSS.** It shows the change at once and sends `text` with its CSS request, so the CSS never trails the save. The hosted app sends `text` to `/app/api/themes/css` the same way.

**The desktop app zooms its webview.** The page sends `silt://zoom/?percent=120`, which the window catches and refuses, as it does `silt://theme/`. The app calls `set_zoom` and keeps the level in its data dir, and applies it when the window opens, before the daemon's page loads. `Ctrl+=`, `Ctrl+-` and `Ctrl+0` are the web UI's own shortcuts there. They aren't Tauri's zoom hotkeys, which would change the level without the page knowing to keep it. The palette offers zoom and bigger or smaller conversation and code text.

## Consequences
- **No protocol change:** text and layout is a UI route, not sync, and each machine keeps its own, as it does its theme pick.
- **A size away from the default can look off** where a component assumed 13 px. The default renders as before, and the screenshot tests check it.
- **Fonts can be set in two places,** a theme and Text and layout. The latter wins and says "over the theme's".
- **Tab width now defaults to 4** where browsers drew 8.
