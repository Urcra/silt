# Interaction

Treat hover as part of every HTML chart, and build it with the care you give the still image. A bare stat tile, with no plot, is the only chart that skips it.

## Tooltips and hover

A tooltip is a convenience, never a lock. Anything it shows can also be read from labels on the marks or from the table view, and focusing a mark with the keyboard shows the same as hovering it.

- **Lines and areas get a crosshair.** A thin vertical rule in `chart-grid` or `border` tracks the pointer and jumps to the closest x in the data. People point at a moment in time, not at a thin stroke. The points at that x light up with an end dot on each line.
- **Bars, dots and cells are their own targets.** No crosshair here. Each one opens its tooltip on `pointermove` and on `focus`, showing its category and value, and brightens or gains an outline while under the pointer so the reader sees it react.
- **One tooltip covers every series** at that x, listed in series order, so nobody has to chase a line or area to read it.
- **Treat names as untrusted.** Series and category names arrive from CSV headers, tool output, page data and people. Fill tooltips, legends and tables with `textContent` or `createTextNode`, and never build them by gluing strings into `innerHTML`.
- **The number comes first.** In the tooltip, set the value in `fg-1` at semibold and the series name in `fg-2`. This flips the legend's emphasis, because someone hovering already knows which series they want and is after its value.
- **Keys are short strokes.** Mark each tooltip row with a short line in the series colour; a filled square is too heavy at that size. Legends match their marks: rectangles for bars and areas, lines for lines, dots for scatter.
- **Targets are larger than what's drawn:**
  - the hit area covers the mark's gap and halo, not only its painted pixels;
  - every scatter point gets an invisible target of 24px or more, or, when points are packed, a nearest-point layer (d3's `Delaunay.find` or a `Voronoi`), so the pointer only needs to be nearer to it than to any other.
- **A value with no room on its mark** (`marks.md`) appears in the tooltip, and stays in the table view.
- **Placement:** close to the pointer, flipped to stay inside the chart and the page, and never under a fingertip on touch screens. Style it with `bg-elevated` (or `surface-1`), a `border` hairline and the text tokens, and give it `pointer-events: none` so it can't steal the hover.
- **Keyboard:** either every mark has `tabindex="0"`, or the plot is one focusable control whose arrow keys step through the data. Focus opens the tooltip and shows a visible ring; `Escape` closes it.
- **Touch:** tapping a mark opens its tooltip, tapping elsewhere closes it, and dragging along a line moves the crosshair.
- **A still SVG** (`kind: "svg"`) can't react, so it carries more labels and a table next to it.

## The table view

Each chart has a twin table holding the same numbers. Screen readers, people checking a figure and anyone copying data all rely on it.
- A real `<button>` with `aria-pressed`, labelled "Table" or "Chart", swaps the drawing for a `<table>` in the same space.
- The table has a row per data position and a column per series, headed with names and units, figures right-aligned in `tabular-nums`.
- The `<svg>` carries `role="img"` and an `aria-label` stating what the chart shows; details live in the table.
- A `data` artifact provides this toggle and a CSV download on its own.

## Filters and date ranges

Filters are the page's usual form controls (`<select>`, `<input>`, buttons), styled like the chart's chrome. Charting only adds rules about where they sit and what they affect:
- **Put them in a single row at the top,** aligned left, ahead of everything they affect. Not in a chart's card and not repeated per chart; a chart that needs its own range belongs on a different page.
- **Lead with the date range,** since readers want it most. Offer presets first (today, last 7, last 30, last 90 days, this month) and a custom range after them.
- **One filter row drives the whole page,** so every chart, tile and table shows the same slice and their figures match.
- **Colours stay put.** When a filter hides some series, the others keep their own colours (`colour.md`).
- **Redraws don't blank the chart.** While new data is computed, keep the old drawing at lower opacity. No placeholder skeleton, no shift in layout, no flicker.
- **Date pickers:** show the presets as a list, because choosing "last 30 days" from a calendar is tedious. Tick the chosen one, make hover a pale wash that never looks like a selection, and put the custom range under a thin rule at the bottom.
- **Remembering a filter** for one viewer can use `localStorage` inside a `try`. A filter everyone should see belongs in page data (`artifact-capabilities`).

## Motion

- When a chart's data changes, move marks from old to new values over 150 to 250ms so the eye can follow. A chart on a page that's only read doesn't animate on arrival.
- Under `prefers-reduced-motion: reduce`, nothing moves.
- Everything is visible once the page has loaded; no chart sits at zero opacity waiting for a scroll.

## Data in the page

The frame can't reach the network, so a chart's data is written into the page or kept in a file of the same version that the page fetches by a relative path (`fetch('data.json')`). Keep one copy of the data and build every chart, tile and table from it so they agree. Data that people enter, or that the agent seeds later, lives in page data (`silt.use("db")`), redrawn from `onSnapshot`, with an empty state while it loads.
