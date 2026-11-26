# Themes

Silt's web UI and desktop app come in themes. You pick a light theme and a dark theme in **Settings › Appearance**, and **System**, **Light** or **Dark** chooses which one shows. Silt ships Nous, its own look, plus a few more of its own and ports of popular themes. You can write your own, or bring one from VS Code or Zed.

## Picking a theme
- **Settings › Appearance** lists every theme with its colours, where it's from and its licence. **Use** puts a theme in the slot for its appearance: a light theme becomes your light theme, a dark one your dark theme.
- **The palette** (⌘K or Ctrl+K) has a **Theme:** entry for each theme. Moving the highlight over one shows it at once; Enter keeps it, and Esc puts back what you had.
- **A theme's page** (click its name) shows every colour, which ones Silt worked out for it, its highlight colours, fonts and corners, and how its text reads against its backgrounds.

Your daemon keeps the pick, so the browser and the desktop app on one machine agree; each machine keeps its own. The desktop app's window takes the theme too: its frame follows light or dark where the system lets it, and it opens in the theme it last showed.

**Your themes follow you.** Each machine signed in to a server sends the themes in its themes folder there, and takes those your other machines sent, fonts included, every few seconds. Removing one removes it everywhere, and when two machines change the same theme, the later change wins. The hosted app offers Silt's own themes and yours, and keeps your pick in that browser.

## Bundled themes
- **Nous**, light and dark: Silt's own warm paper and tobacco, with one amber accent.
- **Fjord**, light and dark: cool slate with a teal accent, rounder corners, and Inter and JetBrains Mono where you have them installed.
- **Yule**, light and dark: snow on pine and a forest at night, with cranberry, gold and holly beside the pine accent, and red keywords in code. It's there all year, and shows only when you pick it.
- **Ink**, light and dark: black and white at the highest contrast, with square corners.
- **Ports:** Catppuccin (Latte, Frappé, Macchiato, Mocha), Gruvbox (light and dark), Rosé Pine (Dawn, Main, Moon), Tokyo Night, Solarized (light and dark) and Nord.

Each port keeps its author's palette. Where a colour reads below 4.5:1 as text, as some comment colours do, Silt moved it lighter or darker, keeping its hue. NOTICE credits each one.

## Bringing a theme from VS Code or Zed
**Import from VS Code or Zed** in Settings › Appearance, or `silt theme import`, reads a theme once into Silt's format and saves it in your themes folder:

```bash
silt theme import ~/Downloads/tokyo-night-color-theme.json
```

- **VS Code themes** are read from their `colors` and `tokenColors`. Silt matches each highlight to the most specific TextMate scope. Comments and trailing commas are fine. A theme that doesn't say whether it's light or dark is guessed from its background; `--appearance light` chooses.
- **Zed theme families** are read from each theme's `style` and `syntax`, keeping every light and dark theme in the family.

The import says what it mapped, what Silt has no place for (a VS Code theme's minimap and terminal colours, its `semanticTokenColors`), and what Silt worked out itself. It lists any text below 4.5:1; `--fix-contrast` moves those colours until they read. A file with the same name is kept unless you pass `--force`; `--to` names the theme.

## Writing a theme
A theme file is JSON, one family a file, in your themes folder (`silt theme path`: `~/.local/share/silt/themes/` on Linux). The quickest start is a copy of a theme you like: **Duplicate to edit** on its page, or

```bash
silt theme new my-theme --from nous-dark
```

A theme needs only three colours; Silt works out the rest:

```json
{
  "$schema": "https://raw.githubusercontent.com/Urcra/silt/main/docs/reference/theme.schema.json",
  "name": "Ember",
  "themes": [
    {
      "id": "ember-dark",
      "name": "Ember Dark",
      "appearance": "dark",
      "colors": { "bg": "#1b1612", "fg-1": "#efe6d6", "accent": "#e0803a" },
      "syntax": { "comment": { "color": "#9a8f7c", "font_style": "italic" } },
      "fonts": { "ui": ["Inter", "system-ui"], "mono": ["JetBrains Mono", "monospace"] },
      "radii": { "sm": 2, "md": 6, "lg": 10 }
    }
  ]
}
```

- **`$schema`** gives an editor that reads JSON Schema (VS Code, Zed and most others) completion and a description for every key.
- **`colors`** are the UI's tokens:
  - backgrounds: `bg`, `bg-elevated`, `surface-1` to `surface-3`;
  - text: `fg-1` to `fg-3`, `fg-on-accent`;
  - lines: `border`, `border-strong`, `border-subtle`;
  - the accent: `accent`, `accent-hover`, `accent-tint`, `accent-fg`;
  - states: `ok`, `warn`, `danger`, `info`, each with a `-tint`, and `warn-fg`, for warnings set in words, as `accent-fg` is for links;
  - `backdrop`.

  Colours are hex only: `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. What you leave out is derived, and derived text is made to reach 4.5:1.
- **`syntax`** styles code by highlight tag: `keyword`, `string`, `string-special`, `number`, `constant`, `comment`, `type`, `macro`, `variable`, `definition`, `property`, `operator`, `punctuation`, `meta`, `heading`, `link`, `emphasis`, `strong`. Each is a colour, or `{ "color", "font_style", "font_weight" }`.
- **`fonts`** name fonts, first choice first, in `ui` and `mono`: installed ones, or ones the theme ships in `faces` (below). Silt adds a generic family if you give none.
- **`radii`** are the small, medium and large corners in pixels, from 0 to 24.

A family can hold several themes, as Catppuccin does. Give a theme the id of one of Silt's to replace it.

### Shipping fonts
A theme that brings its own fonts is a folder: `themes/<name>/theme.json`, with the font files in `themes/<name>/fonts/`. Each face names its family and file, and that family can then be used in `ui` or `mono`:

```json
"fonts": {
  "ui": ["Harbour Sans", "system-ui"],
  "faces": [
    { "family": "Harbour Sans", "file": "fonts/harbour-sans-400.woff2" },
    { "family": "Harbour Sans", "file": "fonts/harbour-sans-600.woff2", "weight": 600 },
    { "family": "Harbour Sans", "file": "fonts/harbour-sans-400-italic.woff2", "style": "italic" }
  ]
}
```

- **Formats:** woff2, woff, ttf or otf. Silt checks each file's first bytes, not only its name.
- **Size:** at most 1 MiB a face and 4 MiB a theme, and up to 16 faces.
- **What's left out:** a face whose file is missing, isn't a font, is too large, or sits outside the theme's `fonts/` is left out, with a warning on the theme's row. A theme that's a single `.json` file ships no fonts.

Silt serves the faces to your own machine only, from `/theme-fonts/<theme>/<file>`. `silt theme new --from` and **Duplicate to edit** copy a folder theme's fonts with it, and `silt theme check` takes the folder.

**Silt shows your edits as you save.** Pick your theme, keep Silt open beside your editor, and each save restyles the page. A mistake in the file is shown on the theme's row in Settings and in `silt theme list`; the rest of the file still loads, and a theme missing `bg`, `fg-1` or `accent` is left out.

`silt theme check` reports what Silt couldn't read and every text pair below 4.5:1, and exits non-zero while one fails, so it can run in a theme's own CI:

```bash
silt theme check ~/.local/share/silt/themes/ember.json
```

## Text and layout
A theme sets how Silt looks. **Settings › Text and layout** sets how big and how dense it is, over whichever theme shows. A preview at the top draws a reply, a code block and a diff at the values you pick, and each row you change gets a **Reset**.

- **Zoom**, in the desktop app: everything in the window, from 50% to 200%. `Ctrl+=` and `Ctrl+-` (`⌘=` and `⌘-` on a Mac) change it, `Ctrl+0` resets it, and the window opens at it next time. In a browser, use the browser's own zoom.
- **Interface text** (11–18 px, 13 by default): the sidebar, headers, buttons and lists; small print and headings follow it.
- **Conversation text** (11–22 px, 13): the agent's replies, your messages and docs.
- **Code text** (10–20 px, 12): code blocks, diffs and tool output.
- **Interface font** and **code font**: the theme's, the system's, or an installed font by name, over the theme's; and **ligatures in code**, on or off.
- **Density**: rows in the sidebar, and the space between a thread's turns.
- **Conversation line height** and **conversation width**: 640, 720, 880 px or the full width.
- **Diff layout**: Auto, which puts a diff side by side where there's room, Unified or Split; each diff can still switch. **Wrap long lines** in a unified diff, and **tab width** in code and diffs.
- **Detail** and **grouping** of a thread's work, which a thread's own menu changes too, and **reduce motion**: as the system asks, always or never.

The command palette has **Zoom in**, **Zoom out**, **Reset zoom**, and bigger or smaller conversation and code text.

Your daemon keeps these per machine in `appearance.json` in Silt's data folder, beside the theme you picked, so a laptop and a large monitor can differ; the hosted app keeps them in the browser. The file can be edited by hand: a value out of range is clamped, and one Silt can't read is left at its default. See [ADR 0107](https://github.com/Urcra/silt/blob/main/docs/adr/0107-text-and-layout.md).

```json
{
  "mode": "system",
  "light": "nous-light",
  "dark": "nous-dark",
  "text": { "chat_size": 15, "code_size": 13, "code_font": "JetBrains Mono", "density": "compact" }
}
```

## December decorations
In December the desktop app decorates itself: lights under each page's header, a hat on the workspace's tile, snow resting on the composer and above the machine footer, and on the New thread screen a tree drawn as a branch graph, with snow falling behind it. The app's starting page lights the tree while it connects.

They take their colours from the theme you use, so the theme stays as it is, and nothing under them moves or stops working. Snow falls only on screens with nothing to read yet, and with less motion the lights stay lit and the snow lies still.

**Settings › Appearance › Seasonal touches** chooses when they show: **Auto**, the default, from 1 December to 6 January; **Always**; or **Off**. **Falling snow** keeps the rest without the snow. The browser and the hosted app don't show them. See [ADR 0131](https://github.com/Urcra/silt/blob/main/docs/adr/0131-december-decorations-in-the-desktop-app.md).

The choice is kept in `appearance.json` beside the theme you picked:

```json
{
  "mode": "system",
  "light": "yule-light",
  "dark": "yule-dark",
  "seasonal": { "show": "auto", "snow": true }
}
```

## What a theme doesn't change
- **Artifacts** keep their design system's look; they follow only light or dark.
- **Content colours**, such as a whiteboard's notes and a chart's palette, stay as they are.

## Adding a theme to Silt
Bundled themes are in `crates/silt-theme/themes/`. A new one needs these:
- it must pass `silt theme check` and read without a warning;
- it must be written as Silt writes it (`silt theme check --fix-contrast` rewrites a file that needs a fix);
- it must be added to `BUNDLED` in `crates/silt-theme/src/lib.rs`;
- a port needs its `author`, `license`, `source` and `copyright`, and a line in NOTICE.

The crate's tests check every one of them. See [ADR 0098](https://github.com/Urcra/silt/blob/main/docs/adr/0098-themes-for-silts-ui.md) for how themes work.
