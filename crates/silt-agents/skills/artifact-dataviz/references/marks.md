# Marks and anatomy

A calm chart comes from a handful of fixed sizes and two uses of empty space. Only the data should draw attention.

## Mark sizes

These stay the same on every chart of a page:

- **Bars and columns:** 24px thick at most. Cap the thickness and let the rest of each band stay empty. The end that shows the value is rounded at 4px; the end on the axis stays square. All bars rise from the same baseline.
- **Lines:** 2px, with round joins and caps.
- **Markers and end dots:** 8px across or more (r ≥ 4), filled with the series colour, drawn only where one point matters.
- **Areas:** the series hue at roughly 10% opacity, a light wash under its 2px line. Never a solid slab of colour.
- **Gridlines and axes:** 1px solid lines in `chart-grid`, never dashed, on the value axis only. The baseline may use `border`, one step stronger.

In SVG, a bar rounded at one end only is a `<path>`; `<rect rx>` rounds every corner. On a bar thinner than 8px, shrink the radius to half the bar's thickness.

## Space that separates

- **The gap.** Where two fills touch (segments in a stack, bars side by side), leave 2px of the surface colour between them, the same 2px across the whole chart. Neighbouring shades read as separate because of that gap, with no outline needed. In SVG, either leave the 2px out of the geometry or stroke each segment 2px in `var(--bg)` with `paint-order: stroke`.
- **The ring.** Every dot and end marker wears a 2px halo of the surface colour, which keeps them readable where they sit on a line or on each other. Hover targets include the halo (`interaction.md`).

Outlines drawn in ink around marks add weight that carries no data, so leave them off and let the gap and ring do the work.

## Labels and legends

From two series up, the chart has a legend; it's the identity channel that can't fail. Labels placed on the marks add to it. A chart of one series has no legend: the title already says what's drawn, and a lone swatch repeats it.

- **Few labels.** Put text on a line's last point, on the highest or lowest value, or on the series the page discusses. Labelling every point produces a wall of numbers nobody reads. The axis, legend, tooltip and table hold the rest.
- **Order of preference:** labels on the marks first, then gridlines, and never a second value axis.
- **Fit before you place.** A label goes inside a bar or segment only if its measured width leaves padding at both sides. Measure with `getComputedTextLength()` on SVG `<text>` or `measureText` on a canvas. When it doesn't fit:
  - for a whole bar, place it beyond the bar's end, or leave it to the tooltip when that space is also taken;
  - for a segment in the middle of a stack, which has no free end, print nothing and rely on the legend and tooltip.

  Either way the table view still has the number. Don't hide overflow to squeeze a label in; a label missing its first and last letters is worse than no label.
- **Placement:** at the tip of a horizontal bar, on top of a column, after the last point of a line.
- **Ticks:** five or six round values at most, taken from the scale that draws the marks. Below ten thousand, write them out (0, 1,000, 2,000); on an axis that goes higher, abbreviate (0, 10K, 20K), and don't mix the two on one axis. On a horizontal axis with room between labels, write values out up to a million. Ticks give the reader every value you left unlabelled, so remove them only when all values carry labels. No tick may name a value beyond the data's domain.
- **Units once,** in the axis title or the subtitle ("p50, ms"), never repeated on each tick.
- **Words use text colours.** Bars, lines, dots and areas wear the series colour. Text (values, labels, legend entries, ticks) uses `fg-1` for values, `fg-2` for labels and `fg-3` for ticks. Pale chart hues can't be read as text against the page. Put a dot, a short line or a swatch next to the words to show which series they mean.
- **Text on top of a coloured fill** (a segment, a heatmap cell, a block of the tall bar) is the exception. Print it in white or the system's darkest ink, whichever scores the higher APCA Lc on that fill: run `silt design contrast "#ffffff" <fill>` and the same for the dark ink. In the default palette, white wins on all six `chart-*` fills in both themes. Dark ink wins on the faint end of a ramp (`chart-seq-1` in light, `chart-seq-7` in dark), on `chart-div-mid` and `chart-muted` in light, and on the dark theme's `ok` and `warn`. `palette.md` has the full list. The two inks are the same in both themes; only the choice per fill changes.
- **Crowded line ends:** pushing labels up and down until they clear each other cuts them loose from their lines. Draw a thin `chart-grid` leader from each label to its line, split the chart into small multiples, or fall back on the legend and tooltip. Beyond about four lines that converge, small multiples usually win.
- **Titles state the finding** ("Rescan time fell 40% after the index"). The subtitle names the measure and units; the caption gives the source and date. "Time by commit" spends the most-read line on axis names.

## Numbers shown as figures

- **Stat tile:**
  - a label in sentence case, no colon at the end;
  - the figure in the body face at semibold, abbreviated (1,284; 12.9K; $4.2M), with its unit;
  - an optional change, with its sign and the period it compares to ("−62% vs last week"), coloured by whether a rise is good (`colour.md`) and paired with an arrow so colour isn't the only cue;
  - an optional sparkline of around twelve points in `chart-muted`, ending in an `accent` dot on the latest value.
- **Meter:** the filled part shows severity, moving from `accent` to `warn` to `danger`. The empty track is a pale step of the same hue (`chart-seq-1` under a `chart-seq` fill, for example), so the whole bar reads as one state. The ratio is printed next to it.
- **Segmented bar:** a total cut into parts. Nothing anchors it to an axis, so both ends are rounded. Each piece's width matches its value, and a zero value takes no width.
  - **Thin version:** 8px high, or 4px in packed rows, with fully rounded ends and no text.
  - **Tall version:** 28px high and no taller. Every part is its own block with 4px corners. Inside each block, at its left edge, print the percentage (46%) or a rounded amount (413M) in white or dark ink as above. When a block is too narrow for its text, leave it blank; the value is in the table view, or, for a still SVG with no table, after the part's name in the legend.
  - Hovering or focusing a part, and the bar's accessible name, give its name, amount and percentage. The legend names the parts in the same order as the blocks and doesn't repeat numbers already printed on them.
  - The tall version breaks four rules on purpose: it can be thicker than 24px, it is a large saturated shape, every part carries a number, and the legend names the parts instead of labels on them. Legend and table view stay.
- **Hero figure:** the one number the page opens with, at least 48px, in `--font-sans` like everything else. A display or serif face makes it look like ornament. One per view.
- **Digit widths:** a big number on its own keeps the font's proportional digits. Tabular digits are all as wide as a zero, which spaces out a number like `121` at display size. Use `font-variant-numeric: tabular-nums` only where digits must line up in a column: table cells and axis ticks.

## Texture as a fallback

When hue can't do the job (full colour blindness, greyscale printing, forced colours), a hatch pattern tells series apart.
- Use one hatch direction per pair of slots, at 45° or its mirror at 135°. Level or upright hatching looks like grid lines or bars.
- Draw it in a deeper step of the fill's own hue, at the same strength for every slot.
- On value scales the hatch angle follows the value, and its direction shows which side of a diverging scale a value is on, so it agrees with the data.
- Turn it on with `@media (forced-colors: active)`, `@media print`, or a switch on the page, and leave it off otherwise: dense diagonal lines can trigger motion sensitivity.
- In SVG, use a `<pattern>` with `patternTransform="rotate(45)"` and a page-unique id. Under forced colours, draw `CanvasText` hatching on `Canvas`.

## Layout

- **Count the axis band in the height.** If a chart's height is fixed, it covers the plot and the x-axis labels, so the card never needs its own scrollbar. Letting the card size to its content is better still.
- **Draw at the container's width.** Read `clientWidth`, draw to it, and draw again from a `ResizeObserver`. On a phone, show every second or third x label and keep the y ticks. A `viewBox` that scales the whole drawing down leaves text too small on a phone, so redraw anything wider than a sparkline instead of scaling it.
- **Leave space for the outer labels** inside the `viewBox`, and set an explicit `fill` on every shape, because SVG fills black by default.
