# Components: the parts a chart is built from

A chart is a few parts put together in plain HTML and SVG. Build each one the same way on every chart of a page, so the page reads as one system.

## Foundations

- **Colour roles.** The design system's tokens, which Silt gives the frame as CSS custom properties for both themes: `--chart-1` … `--chart-6`, `--chart-muted`, `--chart-seq-*`, `--chart-div-*`, `--ok`, `--warn`, `--serious`, `--danger`, `--chart-grid`, `--bg`, `--fg-1` … `--fg-3`. Read them through `var()` with a fallback in CSS, or `getComputedStyle` in script. A page with no design system defines its own under the same names in its `:root`, light first, then both dark blocks, as `artifact-design` lays out, and checks them with `silt design palette`.
- **Texture fill.** One `<pattern>` per slot that needs it, a hatch at 45° or 135° (`marks.md`), used only under forced colours, print or a control.
- **Chart container.** A `<figure>` holding the title (`<h3>` or a `<figcaption>` at the top), the subtitle with measure and units, the drawing, the legend, the caption with source and date, and the table toggle. It owns the chart's width and redraws on resize. If its height is fixed, that height takes in the x-axis labels; letting it grow with its content is better.
- **Legend.** A row of keys above or beside the plot, each a swatch shaped like its mark and a name in `fg-2`, wrapping on a phone. Clicking a key may isolate its series, as a `<button aria-pressed>`; the hidden ones keep their colours for when they return.
- **Tooltip.** One absolutely placed `<div role="tooltip">` per chart, reused for every mark, built with `textContent`, the value first (`interaction.md`).
- **Axis.** Ticks from the same scale as the marks, in `fg-3` at 11 to 12px, `tabular-nums`; the baseline in `border`; gridlines in `chart-grid` on the value axis only.
- **Data label.** A `<text>` in `fg-1` or `fg-2`, or inside a fill in white or the dark ink, placed only after measuring that it fits (`marks.md`).

## The common charts

- **Bar chart.** Grouped and stacked, horizontal and vertical. Bars at most 24px, 2px gaps, a rounded data-end. Horizontal when the category names are long or there are more than about eight.
- **Line chart.** Several series at 2px, end dots and end labels where they separate, a crosshair tooltip. The area variant is a 10% wash under one series or a stack.
- **Stat tile.** A label, the value, a signed change with an arrow, an optional sparkline (`marks.md`). A row of them wraps to two columns, then one, on a phone.
- **Meter.** A track in a pale step, a fill carrying severity, the ratio printed beside it.
- **Segmented bar.** A total cut into parts, in a thin version that sits beside figures and a tall version that prints each part's value when it fits.

## Rounding out the kit

- **Area chart,** stacked, each band's top edge a line.
- **Sparkline,** 12 or so points, no axes, an end dot in `accent`, at the height of a line of text.
- **Heatmap,** cells in `chart-seq-*` with a 2px gap, labelled only where a value matters, with a scale legend.
- **Scale legend** for sequential and diverging colour: a row of the steps with the values at the ends and, for diverging, the midpoint's meaning ("no change").
- **Filter row and time range,** above the charts it scopes (`interaction.md`).
- **Empty state:** the chart's box at its usual size, with a line saying what will appear and how ("No runs yet. Results show here after the first `silt-bench run`."), never a blank box or a spinner that never ends.
- **Table view,** the twin of every chart (`interaction.md`).

## Patterns over these

- **Small multiples** are a layout over the same chart, a grid of panels on one scale (`form.md`).
- **Parts of a whole** ride on bars: ranked bars or a segmented bar for one whole, and a stacked bar chart when the split changes over time or several totals are compared. The donut isn't in the kit.
- **Scatter** is a dot chart on two value axes, with targets of 24px or a nearest-point layer, and at most three categorical colours.

## What Silt draws for you

A `data` artifact's blocks are these components, drawn by Silt from values: `stat` is a stat tile, `table` a table, and `chart` with `line`, `area`, `bar`, `dot`, `scatter`, `heatmap` or `segmented` the matching chart, each with a hover readout, a table view and its source. When those fit, use them; when they don't, build the HTML chart from the parts above.

## The checks behind the kit

- `silt design palette` checks a categorical palette or an ordinal ramp (`colour.md`).
- `silt design contrast` checks a text colour on its ground, and chooses the ink for a label inside a fill.
- `silt_artifact_check` renders the page in both themes and at both widths, and reports colours that ignore the theme and text under 4.5:1. It reads CSS `color`, so give SVG text `fill="currentColor"` and set its `color` from a token; a literal `fill` on text escapes the check.
