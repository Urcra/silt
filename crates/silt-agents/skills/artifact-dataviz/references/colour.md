# Colour

Colour isn't picked by eye. Every colour in a chart does one job, and a palette is used only once it passes the checks, which `silt design palette` runs. The checks are what make a palette safe to change, and why one procedure works with whatever colours a design system brings.

## The jobs

| Job | What it says | Structure | Design system tokens |
|---|---|---|---|
| Categorical | Which series | Up to six hues in a fixed order, assigned in sequence, never cycled | `chart-1` … `chart-6` |
| Ordinal | Position in an order (a funnel stage, a tier, a bucket) | One hue, lightness stepping steadily; the light end still 2:1 on the surface | Consecutive steps of `chart-seq-*` |
| Sequential | How much | One hue in seven steps; more is darker in the light theme and lighter in the dark one | `chart-seq-1` … `chart-seq-7` |
| Diverging | Which side of a midpoint | Two hues that read as opposite (warm and cool), equal steps per arm, a neutral grey midpoint | `chart-div-neg-3` … `chart-div-neg-1`, `chart-div-mid`, `chart-div-pos-1` … `chart-div-pos-3` |
| Status | State, from fine to failed | A small fixed scale with reserved meaning, always with a word or an icon | `ok`, `warn`, `serious`, `danger` |
| De-emphasis | Context, "Other" | A grey a step off the surface | `chart-muted` |

**Categorical or ordinal?** Ask whether swapping the categories' order would change the meaning. Funnel stages, sizes (S, M, L), age bands and cohorts would: they're ordinal and take one hue's steps, so the order shows up as a change in colour. Products, teams, regions and endpoints wouldn't: they're named categories. As one series, every bar takes `chart-1` and the title names it, with no legend. As several series, they take `chart-1` to `chart-N`. Never shade named bars by value: it wastes colour on what the bar's length already shows.

**An ordinal ramp from the sequential tokens.** Pick consecutive or evenly spaced steps of `chart-seq-*`, as many as the order has. In the light theme start no paler than `chart-seq-1`, which is about 2:1 on `bg` in the default palette; in the dark theme `chart-seq-1` is the darkest and still clears 2:1. Check the steps you picked with `--ordinal`.

## The checks

`silt design palette "<hex,…>" --surface <hex> [--theme light|dark] [--pairs adjacent|all] [--ordinal]` checks a categorical palette in the order given. `--surface` is required: the chart's ground, normally the design system's `bg` for that theme. `--theme` defaults to `light`, `--pairs` to `adjacent`.

1. **Fixed order.** The order is what keeps neighbours apart. It isn't measured; it's kept. Never re-sort by value.
2. **Lightness band.** OKLCH L from 0.43 to 0.77 on a light surface, 0.48 to 0.67 on a dark one, so no series looms over the rest or fades into the ground.
3. **Chroma floor.** OKLCH C of 0.10 or more. Below it a hue reads as grey and stops saying which series.
4. **Separation under colour-vision deficiencies.** The distance between colours is Euclidean in OKLab ×100, measured after simulating protanopia and deuteranopia with Machado, Oliveira and Fernandes' 2009 model at full severity. The thresholds are calibrated to that model.
   - 8 or more passes.
   - 6 to 8 warns. It's allowed only with a second encoding: direct labels, gaps or texture.
   - Under 6 fails.
   - Tritanopia is reported beside the result, not gated.
5. **Separation under normal vision.** The same distance with no simulation must be 15 or more, so neighbours stay easy to tell apart for everyone. This is a hard failure that no second encoding excuses.
6. **Contrast.** Each colour needs 3:1 against the surface so a thin mark shows. Under it warns, and obliges visible labels or a table view. That obligation can't be waived.

The report prints one line per check with the worst pair it found and why. The command exits non-zero when any check fails.

**Neighbours or all pairs.** Stacks, bars and lines put only neighbours side by side, since assignment never skips a slot, so the default checks adjacent pairs. Scatter plots, bubbles, maps and small multiples can put any two together: pass `--pairs all`, or two colours that merge go unnoticed. Testing every pair is stricter, and it limits how many series such charts can hold. No order of six hues passes it. In the default palette the first three pass all pairs with a separation warning, so those forms always label their marks, and the fourth fails in both themes; a checked system usually does the same. More than three series in such a form means folding into "Other" or faceting, not changing the palette.

**Ordinal ramps.** `--ordinal` runs the ramp's checks instead of the categorical ones:
- lightness strictly increasing or decreasing;
- each step at least 0.06 apart in L;
- the palest step at least 2:1 against the surface;
- one hue, spread within 40°.

A sequential or diverging ramp fails the categorical checks by design: it spans the band and its steps sit close. That isn't a real failure, so don't "fix" a good ramp to pass it. Check it with `--ordinal`, each arm of a diverging scale on its own.

**Text colours.** `silt design contrast <text> <ground>` prints WCAG 2's ratio, with whether it passes for text (4.5:1), for large text and marks only (3:1), or fails, and APCA's Lc. APCA tracks what people see more closely, so go by it where the two disagree. Where a page must meet WCAG 2 AA, hold text to its ratio. Compare APCA by its size: light text on a dark ground scores negative. It's also how to choose the label inside a fill (`marks.md`).

Run the checks once per theme with that theme's surface. Fix every FAIL before going on.

## Fitting a brand's colours

Given a brand's hues and a wanted order:
1. Slot by slot, choose the step of that hue whose L is inside the theme's band and whose C is at least 0.10.
2. Run the check. For a neighbouring pair under 8, move one of them a step in lightness, keeping its hue, and run it again.
3. Repeat until the worst neighbouring pair clears the target, and every pair clears 15 under normal vision. The brand's hues stay; only their steps move.

**With no order given, don't guess.** List the candidate orders of the system's hues, run the check on each, and keep the one whose worst neighbouring pair is furthest apart in both themes. Starting from a known good order and swapping hues of a similar family is a fine way in; the command still decides.

## Themes

The dark theme is chosen, not inverted. It takes its own steps of the same hues, picked for the dark band and checked against the dark surface as a set. A design system's `chart-*` tokens hold both steps, so a page that uses the tokens gets the right one in each theme, and a page that sets colours as literals must define both under the theme rules `artifact-design` gives.

The slot order is a separate choice from the hues. A design system names one order, and a page keeps it: never mix orders on one page, and never re-order a system's palette for one chart without running the check.

## Status is fixed

Status never follows the categorical order. It's a small fixed scale (`ok`, then `warn`, `serious` and `danger`) with reserved meaning, on steps kept apart from the categorical slots so a status colour never looks like a series. It always comes with an icon and a word: a `warn` amber or yellow often falls under 3:1 on a light `bg`, and the word carries it.

When a series *means* good or bad (an error rate, passes and failures, a budget's overrun), it wears status colours. When it's only "series 4", it wears categorical ones. Never both in one chart. A series colour that sits beside a status cue of a similar hue (a rust series beside `danger`, say) leans on the word and the placement, never on hue alone.

A change's colour follows whether up is good: a faster build time is good and goes in `ok`, a rising error rate goes in `danger`, and a change with no good direction stays in `fg-2`. Always print the sign as well.
