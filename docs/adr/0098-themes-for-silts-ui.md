# 0098: Themes for Silt's UI

- Status: accepted
- Date: 2026-11-17
- Amends: [ADR 0039](0039-the-web-ui-as-designed.md) (Nous stays the default, no longer the only look)
- Amended by: [ADR 0099](0099-themes-sync-ship-fonts-and-reach-the-window.md) (sync, fonts, the desktop window, `warn-fg`)
- Amended by: [ADR 0107](0107-text-and-layout.md): Text and layout

## Context
Silt's UI has one look, Nous, light and dark (`web/src/nous.css`). The light, dark or system pick is kept per browser. People want to pick a look from a gallery, write their own, and bring along a theme they already use in their editor.

The UI is already shaped for it. Components use about 27 semantic CSS variables through Tailwind's names, and code is coloured through Lezer's `tok-*` classes. A theme is new values for those variables.

Mockups were drawn on the app's own screens.

## Decision
**A theme is a JSON file in Silt's own format.** A file holds a family of themes. Each theme has one appearance, light or dark. A theme sets:
- `colors`: the semantic tokens by their CSS names (`bg`, `fg-1`, `accent`, …);
- `syntax`: a colour, style and weight per highlight tag;
- `fonts`: the UI's and code's families, as lists of names;
- `radii`: `sm`, `md` and `lg`, in pixels.

Only `bg`, `fg-1` and `accent` are required. Silt derives the rest in OKLab, and solves derived text colours to 4.5:1. A generated JSON Schema (`docs/reference/theme.schema.json`) gives editors completion.

**`silt-theme`, a pure crate,** parses, derives, compiles and checks themes, and imports them from VS Code and Zed. It uses silt-design's `palette` for colour maths and contrast. It is its own crate because design systems style artifacts, with another trust model and their own `Theme`.

**A theme's file is untrusted, so the CSS is built, never passed through:**
- colours are hex, parsed and printed back by Silt;
- token and tag names are closed enums;
- font names match `[A-Za-z0-9 _-]{1,64}` or a generic family, and are quoted by Silt;
- radii are clamped to 0–24 px;
- there is no `url()`, `@font-face` or `@import`, so a theme names installed fonts and can't load one;
- unknown keys and bad values are warnings with their JSON pointer, and the rest of the file still loads.

**You pick a light theme and a dark theme;** System, Light and Dark choose which shows, as before. The compiled CSS uses the selectors silt-design's frames do:
- `:root, :root[data-theme=light]` for the light theme;
- `@media (prefers-color-scheme: dark) { :root:not([data-theme=light]) }` and `:root[data-theme=dark]` for the dark one.

Fonts, radii and syntax styles can then differ per appearance, which `light-dark()` can't do.

**Nous is a theme like the others,** `crates/silt-theme/themes/nous.json`. `nous.css` stays hand-written, as the server's pages and the site include it, and a test checks the two match. Syntax colours become `--syntax-*` tokens in `nous.css`, so `index.css` reads only tokens.

**Silt ships Nous, a few originals, and ports of MIT-licensed themes.** Each port names its author, licence and source, and NOTICE credits it. A gate test checks that every bundled theme parses without a warning and passes the contrast checks.

**The daemon keeps themes and the pick per machine,** so the browser and the desktop app agree:
- your themes are in `$SILT_HOME/themes/*.json`, and one replaces a bundled theme with the same id;
- the pick is in `appearance.json`;
- a missing theme falls back to Nous and keeps the pick.

The daemon inlines the compiled CSS into `index.html`, so a page never flashes Nous first. It watches `themes/` and tells open pages over `/api/events`, so saving a file restyles the UI: an author's dev loop.

**`silt theme`** lists, checks, creates (`new --from`) and imports themes. `import` reads a VS Code theme (`colors`, `tokenColors` matched by TextMate scope) or a Zed family (`style`, `syntax`) once, into Silt's format. It reports what it mapped, derived and ignored, and nudges failing colours with `--fix-contrast`.

**The hosted app** offers the bundled themes, and keeps its pick in the browser.

## Consequences
- **Two places hold Nous's values,** `nous.css` and `nous.json`, kept equal by a test.
- **Some colours stay outside themes:**
  - colours that belong to content: whiteboard notes, the chart palette, the deck viewer's chrome;
  - identity colours: the QR code and cursor labels;
  - artifact frames, which keep their design system.

  Mermaid follows light or dark as before.
- **No protocol change:** themes are UI routes, not sync. Your themes don't sync across machines yet.
- **A theme can't ship a font,** only name one; bundling font files is later work.
- **The desktop app's window chrome** still follows the system rather than the theme.
