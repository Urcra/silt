---
name: artifact-dataviz
user-invocable: false
description: How to chart data in an artifact so it reads right - choosing the form, assigning and checking colours against the design system, mark and label rules, hover, and both themes. Load it before drawing any chart, sparkline, stat tile or dashboard for silt_artifact.
---

# Charting data

A chart is read by a person and drawn by your code. This skill turns "make it look good" into steps with checks, so a chart comes out right because of how it was made, not because of taste.

The method doesn't belong to one design system. A design system supplies a few parameters: its categorical order, a sequential ramp, a diverging pair, a status scale, its surfaces and its text colours. The method uses them as they are. A project's or the person's design system fills in these names (`design-system` says how), so a chart written against the tokens works under any of them. A page with no system defines them itself, from the default palette in `references/palette.md`, checked in both themes.

The habit that matters most: **the colour part can be computed, so compute it.** Never judge by eye whether colours can be told apart. Run `silt design palette`.

## Which kind of artifact

- **A `data` artifact** (`artifact-data`) when the page is figures with their sources: stat tiles, line, area, bar, dot, scatter, heatmap and segmented charts, and tables. Silt draws those from values, with the design system's colours, a hover readout and a table view. You choose the form and the series; this skill's first step and its rules on series counts still apply, and the rest is done for you.
- **An `html` artifact** when the chart needs a form, an interaction or a layout the data kind doesn't draw. Then every step below is yours.
- **An `svg` artifact** for one static chart. Nothing runs and no tokens reach it, so it gets its own colours for both themes, fuller labels, and no hover.

## The steps, in order

Colour comes last. Most bad charts pick their colours first.

1. **Pick the form.** What must the reader do with the data: compare amounts, tell series apart, see which side of a line, read one number, follow a change over time? The job picks the type, and sometimes the answer isn't a chart at all but a stat tile, a hero figure or a table. Read `references/form.md`.
2. **Give each colour one job.** Categorical (which series), ordinal (where in an order), sequential (how much), diverging (which side of a midpoint) or status (what state). Each job has one rule. Assign categorical colours in their fixed order and never cycle them. Read `references/colour.md`.
3. **Check the palette with the command.** Don't reason about distances.
   ```sh
   silt design palette "<hex,hex,…>" --surface <bg> --theme light
   silt design palette "<hex,hex,…>" --surface <bg> --theme dark
   ```
   It reports PASS, WARN or FAIL for the lightness band, the chroma floor, separation under colour-vision deficiencies, separation under normal vision, and contrast. It exits non-zero on any FAIL. Fix every FAIL before going on. Add `--pairs all` for forms where any two marks can sit together, and `--ordinal` for a ramp. The design system's own `chart-*` tokens already pass; you check again when you add, drop, re-order or bring colours.
4. **Draw the marks and the space between them.** Bars at most 24px thick whose value end is rounded at 4px and whose axis end stays square; 2px lines; markers of 8px or more; a 2px gap in the surface colour between touching fills; a 2px ring in the surface colour around markers that overlap; labels only where they help. Read `references/marks.md`.
5. **Add the hover layer.** It's part of the chart, not an extra. A crosshair and one tooltip listing every series on lines and areas; a tooltip per mark on bars, dots and cells; targets larger than the marks; the same on keyboard focus. Filters sit in one row above the charts. Only a stat tile with no plot goes without. Read `references/interaction.md`.
6. **Check access.** Two or more series always get a legend, and up to four also get direct labels, so identity never rests on colour alone. One series needs no legend: the title names it. Every chart has a table view. The dark theme uses its own steps of the same hues, chosen for the dark surface, not an inversion. Texture is ready for forced colours and print.
7. **Render it and look.** The palette check covers colour, not layout. Call `silt_artifact_check` once before telling the person: it renders the page at a phone's width and a desktop's, in light and dark, and reports blocked loads, script errors, sideways overflow, colours that ignore the theme and text under 4.5:1. Look at the screenshots for colliding labels, clipped axes and wrong geometry. Fix what you see in one pass, then publish.

Then go through `references/anti-patterns.md`. A chart that matches an entry is wrong.

## Rules that hold in every design system

- **Categorical colours in a fixed order, never cycled.** A design system gives six. A seventh series never gets a made-up colour: fold the tail into "Other", use small multiples, or add a second encoding such as marker shape.
- **One value axis.** Never two y-scales on one plot. Two measures of different scale go in two charts, in small multiples, or indexed to a common base on one axis. This is the most common chart mistake.
- **Colour follows the entity, not its rank.** A filter that hides series never repaints the ones left. An entity keeps its colour on every chart of the page.
- **Sequential is one hue from light to dark. Diverging is two opposite hues with a neutral grey midpoint.** No rainbow scales, and no hue at a diverging midpoint.
- **Run the check before shipping any categorical palette you changed.** Separation of 8 or more (OKLab ×100, simulated for protan and deutan vision) passes. 6 to 8 is allowed only with a second encoding. Under 15 between neighbours under normal vision is a hard failure that no second encoding excuses: re-step the colour, or with `--pairs all`, cut the series. A contrast warning obliges visible labels or a table view; it can't be waived.
- **Thin marks, quiet chrome, selective labels.** A legend for two or more series, none for one. Never a number on every point. Gridlines are 1px, solid and recessive. The tall segmented bar is the one exception to several of these (`references/marks.md`).
- **Text wears text tokens, never a series colour.** Values, labels, legends and axis text use `fg-1`, `fg-2` and `fg-3`. A swatch, dot or short line beside the text carries the identity.
- **Status colours are reserved.** `ok`, `warn`, `serious` and `danger` mean a state, always with a word or an icon, and never stand in for "series 4".
- **Data is untrusted text.** Series and category names go into the page with `textContent`, never `innerHTML`.

## The parameters a design system supplies

The method stays the same; only these change from system to system. `references/palette.md` has every value of the default palette, for a page with no system.

| Parameter | What the design system gives | Token names |
|---|---|---|
| Categorical order | Six hues, in the order they're assigned, with light and dark steps | `chart-1` … `chart-6` |
| De-emphasis | The grey for context series and "Other" | `chart-muted` |
| Sequential ramp | One hue in seven steps, light to dark in the light theme and dark to light in the dark one | `chart-seq-1` … `chart-seq-7` |
| Diverging pair | Two opposite hues, three steps per arm, a neutral midpoint | `chart-div-neg-3` … `chart-div-neg-1`, `chart-div-mid`, `chart-div-pos-1` … `chart-div-pos-3` |
| Status scale | Four states, distinct from the categorical slots | `ok`, `warn`, `serious`, `danger` |
| Emphasis | The one series a chart is about | `accent` |
| Surfaces | The chart's ground in each theme, which the check needs | `bg`, `surface-1` |
| Ink | Primary, secondary and muted text | `fg-1`, `fg-2`, `fg-3` |
| Chrome | Gridlines and axis lines | `chart-grid`, `border` |
| Type | The face figures are set in | `--font-sans`, `--font-mono` |

Silt gives the frame these as CSS custom properties, `--chart-1` and so on, with each theme's value, and sets `data-theme` from the viewer. Write `var(--chart-1, #009183)` with a fallback and the right step arrives in each theme. A system that lacks a parameter (no diverging pair, say) is filled in by the steps in `references/colour.md` and checked with the command; `design-system` says how to add it to the system itself.

## Drawing it

- **Hand-written SVG** suits a few series and a few dozen marks. Draw text and axes in `currentColor` and fills from the tokens, as `artifact-diagramming` describes.
- **A library,** for more marks or richer interaction, loads from the allowed CDNs, pinned to an exact version:
  - d3 from cdnjs (`d3/7.9.0/d3.min.js`) for full control;
  - Chart.js (`chart.js@4.4.7` on jsDelivr) for standard charts.
- **Theme a library from the tokens,** never its defaults. Read them with `getComputedStyle(document.documentElement).getPropertyValue('--chart-1')`, and draw again when `data-theme` or `prefers-color-scheme` changes.
- **Data goes in the page as a literal,** or in a file of the version that the page fetches relative to itself. The frame reaches nothing else. Keep the data in one place and draw every chart and table from it.

## The references

| File | What it answers |
|---|---|
| `references/form.md` | Which chart, or whether it's a chart at all |
| `references/colour.md` | Colour's jobs, the checks, fitting a brand's colours, themes and status |
| `references/marks.md` | Mark specs, the two spacers, labels and legends, figures, texture |
| `references/interaction.md` | Tooltips, hover targets, filters and time ranges |
| `references/components.md` | The parts a chart is built from, in plain HTML and SVG |
| `references/palette.md` | Every parameter of the default palette, with what the check reports |
| `references/anti-patterns.md` | What goes wrong. Check every chart against it |
