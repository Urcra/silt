# 0133: December decorations in the desktop app

- Status: accepted
- Date: 2026-11-26

## Context
People asked for something seasonal in December that leaves their theme alone. Silt's look comes from themes (ADR 0098), which a person picks and may have written. A Christmas theme alone would replace that pick. Decorations drawn in fixed colours would clash with every theme but one, and red and green read as state in Silt: a thread's marks, diffs and "Needs you" mean something.

## Decision
**Decorations sit over the theme, in its own colours.** In December the desktop app hangs a string of lights under each page's header, puts a hat on the workspace's tile, lets snow rest on the composer's top edge and above the machine footer, and on the empty New thread screen draws a tree with snow falling behind it. The tree is a branch graph: branches fork out of `main` and merge back, with commits as baubles. Each bulb and bauble takes one of the theme's `--danger`, `--ok`, `--warn`, `--info` and `--accent`; branches are `--ok`, the wire `--border-strong`. Snow is white on every theme, with a faint edge on a light ground. No token changes, so any theme, a person's own included, gets decorations in its palette.

**They never get in the way.** Every decoration is drawn on top of the layout, which doesn't move or resize, takes no pointer events and is hidden from screen readers (`aria-hidden`). Status marks, the "Needs you" box, unread dots and diffs are never decorated. Snow falls only where there's nothing to read yet; a thread, a diff, a doc or a merge request stays still. With less motion, from the system or from Settings › Text and layout (ADR 0107), the lights stay lit and the snow lies still. The snow stops while the window is hidden.

**Desktop app only, and on by default.** Settings › Appearance › Seasonal touches offers Auto, Always and Off, kept as `seasonal` in `appearance.json` beside the theme pick, with a Falling snow switch. Auto, the default, shows them from 1 December to 6 January by the machine's calendar. The browser and the hosted app show neither the decorations nor the setting: a browser tab is often a shared screen or someone else's machine, and the hosted app's pick lives in the browser anyway. A value Silt can't read falls back to the defaults without losing the theme pick.

**The desktop app's starting page decorates too.** The `silt://theme/` link the page sends the window (ADR 0099) now also carries the five tones and the Seasonal touches choice, and the app keeps them with the last theme. While the starting page connects, the tree's baubles light one after another; if it can't connect, the tree goes dark and the error is the only colour on the page. An older page's link without them is still read, and shows no tree.

**A Christmas theme ships beside them, off unless picked.** Yule, light and dark, is a bundled theme like Nous and Fjord: snow on pine, and a forest at night, with cranberry, gold and holly beside a pine accent. It passes `silt theme check` as every bundled theme must.

## Consequences
- In December every desktop user sees the decorations unless they turn them off; that's one click in Settings › Appearance.
- A theme with weak `--danger` or `--ok` colours gets dim lights; nothing checks a theme's tones for decoration.
- Falling snow costs a canvas redrawn each frame on the New thread screen, paused when the window is hidden or motion is reduced.
- More seasons would fit the same setting and rules, but each needs its own decision.
