# 0099: Themes sync, ship fonts, and reach the desktop window

- Status: accepted
- Date: 2026-11-18
- Amends: [ADR 0098](0098-themes-for-silts-ui.md)

## Context
ADR 0098 left four gaps:
- **Your themes stayed on one machine.**
- **A theme could only name fonts.**
- **The desktop window ignored the theme:** its frame and its loading page followed the system.
- **Warnings were hard to read.** Nous's `warn` (honey) was used for words in a few places, though its own design system says it reads at only 2.3:1 on light paper.

## Decision
**Words in a warning take `warn-fg`,** as links take `accent-fg`. `warn` stays for icons, dots and badges, at 3:1.
- A theme that leaves `warn-fg` out has it derived from `warn`, solved to 4.5:1.
- Nous's is a darker honey in light, `#936400`, and honey itself in dark.

**Your themes sync through your servers.** Only the files sync; each machine keeps its own pick, so a laptop and a desktop can differ.
- **The server** keeps them in a `themes` table, one row per person and file name, as `ThemeCopy` JSON. A copy replaces the stored one only if it's newer, inside one statement, so two machines saving at once can't race. They're in the server's backups.
- **The daemon** syncs them as it syncs design systems, every 10 s:
  - it keeps an index of what it last sent and took;
  - a removed file travels as a tombstone;
  - a copy counts as sent only once every server has taken it.
- **The hosted app** lists your synced themes over Silt's own.
- **This is protocol 20.**

**A theme can ship its fonts.** Such a theme is a folder, `themes/<name>/theme.json` beside a `fonts/` folder, and its `fonts.faces` name files there. A plain `themes/<name>.json` still works.
- **Checks:** a font is woff2, woff, ttf or otf by its first bytes, not only its name; at most 1 MiB a face and 4 MiB a theme.
- **Serving:** the CSS loads a face from `theme-fonts/<theme>/<file>`, never inline, so the CSS put into every page stays small.
  - The daemon serves that path to its own machine only, as `/vendor`, and only for a current theme's face.
  - The hosted app serves it behind the session, from the person's synced copy.

**The desktop window follows the shown theme.**
- The page tells the app its appearance and background with a `silt://theme/` link, which the window catches and refuses, as it does `silt://open/`.
- The app sets the window's theme and background, and keeps them for the next start.
- The loading page and the window open in them, so a dark theme never flashes light.

## Consequences
- **Protocol 20:** every machine and the server update together.
- **Last edit wins, whole file:** two machines editing the same theme keep the later edit, by the machines' clocks, without a merge, as design systems do.
- **Window frames differ by platform:** on Linux and macOS a window's theme is the whole app's, and how far the frame follows depends on the platform.
