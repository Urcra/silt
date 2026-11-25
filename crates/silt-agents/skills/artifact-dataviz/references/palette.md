# The default palette

This file fills in every parameter of the method for a page with no design system, with what `silt design palette` reports for it. The rest of the skill doesn't depend on these values. A design system gives the same token names, a repository's or the person's; its values replace these, and the same checks apply.

Use the tokens, never hex values, in a page that has a design system: Silt puts the right theme's value in each `var(--…)`. A page without one defines these tokens itself, on `:root` for light and under the dark-theme rules `artifact-design` describes, and writes against the same names. The hexes are also for an `svg` artifact, which gets no tokens.

## How tokens reach a page

Silt compiles the artifact's design system into CSS custom properties, one per token, with light values on `:root` and dark values under both of the theme rules, and sets `data-theme` from the viewer. A chart writes against roles:

```css
.chart { color: var(--fg-2); background: var(--bg); }
.chart .series-1 { fill: var(--chart-1); }
.chart .grid line { stroke: var(--chart-grid); }
```

and reads them in script with `getComputedStyle(document.documentElement).getPropertyValue('--chart-1').trim()`, again when the theme changes (watch `data-theme` with a `MutationObserver` and `prefers-color-scheme` with `matchMedia`).

## The default palette

A neutral set on white and blue-grey, checked in both themes. Use it for a page with no design system unless the look you chose for the subject needs its own colours, and then check yours the same way (`colour.md`).

**Surfaces and ink**

| Role | Token | Light | Dark |
|---|---|---|---|
| Chart surface | `bg` | `#ffffff` | `#16181d` |
| Card | `surface-1` | `#f5f6f8` | `#1e2128` |
| Primary ink | `fg-1` | `#1c2027` (16.3:1) | `#e8eaee` (14.7:1) |
| Secondary ink | `fg-2` | `#4a515e` (8.0:1) | `#b3b9c4` (9.0:1) |
| Muted ink | `fg-3` | `#666e7c` (5.1:1) | `#8f96a3` (6.0:1) |
| Gridlines | `chart-grid` | `#eceef2` | `#272b33` |
| Baseline | `border` | `#dfe2e8` | `#2e323b` |
| Dark ink inside fills | `fg-on-fill` | `#1c2027` | `#1c2027` |
| Emphasis | `accent` | `#2f62c8` | `#7aa2f0` |

**Categorical, in order**

| Slot | Hue | Light | Dark | Label inside |
|---|---|---|---|---|
| `chart-1` | orange | `#c26525` | `#ca7d3f` | white |
| `chart-2` | purple | `#8a5aa8` | `#a67ac3` | white |
| `chart-3` | green | `#348c59` | `#50a36e` | white |
| `chart-4` | blue | `#3e6faf` | `#5f91d4` | white |
| `chart-5` | rose | `#b0405a` | `#e66283` | white |
| `chart-6` | teal | `#0699a0` | `#0ca3a8` | white |
| `chart-muted` | grey | `#b8bec8` | `#4c525e` | dark ink in light, white in dark |

What the check reports on `bg`:

| Test | Light | Dark |
|---|---|---|
| Six, adjacent | all PASS; worst CVD 10.9, worst normal 18.1; tritan 4.6 (reported only) | all PASS; worst CVD 8.5, worst normal 17.8; tritan 3.4 |
| First three, all pairs | WARN on CVD at 6.9 (orange and green): label the marks | WARN on CVD at 6.9 (orange and green): label the marks |
| First four, all pairs | FAIL, CVD 2.8 and normal 11.4 (purple and blue) | FAIL, CVD 3.5 and normal 10.6 (purple and blue) |

**Sequential (blue)**

| Step | Light | Dark |
|---|---|---|
| `chart-seq-1` | `#8baeda` | `#304e75` |
| `chart-seq-2` | `#7698c3` | `#43628a` |
| `chart-seq-3` | `#6282ad` | `#5676a0` |
| `chart-seq-4` | `#4e6e97` | `#6a8bb6` |
| `chart-seq-5` | `#3a5a81` | `#7fa1cd` |
| `chart-seq-6` | `#28466c` | `#94b7e4` |
| `chart-seq-7` | `#153358` | `#aacdfb` |

`--ordinal` passes in both themes; the end nearest the surface is 2.29:1 in light and 2.09:1 in dark. Labels inside: dark ink on `chart-seq-1` in light and `chart-seq-7` in dark, white elsewhere.

**Diverging (red and blue)**

| Step | Light | Dark |
|---|---|---|
| `chart-div-neg-3` | `#8a3819` | `#fcae93` |
| `chart-div-neg-2` | `#b55f40` | `#d4896e` |
| `chart-div-neg-1` | `#e18667` | `#ac654b` |
| `chart-div-mid` | `#ccd1d9` | `#494d53` |
| `chart-div-pos-1` | `#7ba4d9` | `#5a7ca9` |
| `chart-div-pos-2` | `#557caf` | `#80a4d3` |
| `chart-div-pos-3` | `#305686` | `#a8ceff` |

**Status**

| State | Token | Light | Dark | Label inside |
|---|---|---|---|---|
| Fine | `ok` | `#2e7a4b` (5.3:1) | `#6cc28b` (8.2:1) | white in light, dark ink in dark |
| Needs attention | `warn` | `#8a5a00` (5.9:1) | `#e0ad4a` (8.7:1) | white in light, dark ink in dark |
| Serious | `serious` | `#c4530f` (4.6:1) | `#e68a58` (6.9:1) | white |
| Failed | `danger` | `#b4232f` (6.5:1) | `#f07a80` (6.6:1) | white |

## Type and figures

Figures stay in the page's text face, `--font-sans` in a design system, including the hero figure; a display face is for headings, never for numbers. Code names in a chart's labels (a crate, a benchmark, a command) take `--font-mono`. Large standalone numbers use proportional figures; table columns and ticks use `tabular-nums`.

## Texture

Where colour can't carry a series (print, forced colours, or a pair the palette check warns on), lay a hatch over its fill: `--chart-texture` at 45° and `--chart-texture-cross` at 135° where the system defines them, else a `repeating-linear-gradient` of 1.5px lines 7px apart in the theme's ink at about a third of its opacity. Use it as `background-image` in HTML, or draw the same lines as an SVG `<pattern>` (`marks.md`).

## Filters

Filters are the page's ordinary controls styled with its tokens: `surface-1` fill, `border` hairline, the system's small radius, the text size of its controls, and `accent` for the selected preset's check.
