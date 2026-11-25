# Choosing a form

Settle the form first and the colours later. Ask what the reader has to do with the numbers. That task decides the chart type, and quite often it decides there should be no chart.

## When the answer isn't a chart

- **A single number now, perhaps with its recent history:** a stat tile with the figure, its change and a small sparkline. A chart with one bar in it says less.
- **Four or five headline figures:** tiles in a row. Grouping them into a bar chart forces a shared scale they don't have.
- **The figure the page opens with:** a hero number of 48px or more, set in the body sans.
- **How full something is against a cap:** a meter whose empty track is a pale step of the fill's hue. Two pie slices are a worse meter.
- **How one total breaks down, six parts or fewer, seen in a glance with nothing beside it:** the tall segmented bar described in `marks.md`. If the reader has to compare the parts, see "Shares of a total" below.
- **Many classes, past about seven, each of which matters:** a table, maybe with a chart next to it. More colours won't help.
- **A list of named rows, each with its own ratio and its own limit:** a meter list. Each row has a name, a bar and the figure, with the bars lined up; when the column is narrow, the bar moves below the name. A gauge or donut on every row is clutter.
- **Values people will look up rather than compare:** a table, figures right-aligned in `tabular-nums`, units in the column header.

A `data` artifact has blocks for most of these: `stat`, `table`, and `chart` with `segmented`. Use the block when it fits rather than hand-building HTML.

## Task to type

| What the reader does | Start from | What colour encodes |
|---|---|---|
| Ranks or compares amounts | Bars or columns; a heatmap when the data is a grid | Sequential |
| Watches something change over time | A line; an area for a single series or a total | Sequential, or one categorical slot |
| Keeps several series apart | Grouped or stacked bars, or several lines | Categorical |
| Picks out one series among others | Emphasis: that series in `accent`, the others in `chart-muted` | One hue plus grey |
| Sees which side of a baseline or target | Diverging bars, or a line drawn against the baseline | Diverging |
| Reads shares on an ordered scale (a rating, agree to disagree) | A diverging stacked bar centred on the neutral answer | Diverging |
| Sees how a total splits | Bars; choose which below | Categorical, or one hue for ranked bars |
| Compares each item before and after | A dumbbell or a slope chart | Two steps of one hue |
| Sees whether two measures move together | A scatter plot | One hue, or categorical checked with `--pairs all` |
| Sees how values spread | A histogram, or a strip of dots for a few dozen values | One hue |

## Reasons behind the table

- **When unsure, encode magnitude.** A single hue that deepens with the value is hard to misread and stays legible in both themes. Move off it only when the task is about identity or about sides of a baseline.
- **Categorical colour has a price.** It suits a chart whose subject is the series themselves. It can also hide the one value the page is about. "This one rose" is a job for emphasis.
- **Emphasis gets used too rarely.** Colour the series that matters and grey out the rest. When someone asks for a clearer chart, this is often the fix, and the legend shrinks to one name.
- **An order is ordinal.** Test it by swapping two categories. If that changes the meaning, as with funnel stages, sizes, age bands, cohorts or severities, give them steps of one hue so the colour carries the order, and check the ramp with `silt design palette … --ordinal`.
- **Named bars in one series share one colour.** Teams, products and endpoints all wear `chart-1`, and the title names what they measure. Shading named bars by value repeats what their length shows.
- **Bars sit on zero.** The length is the reading. A line can zoom in on its range when the change is what matters, with the axis showing where it begins.
- **Time goes left to right, evenly spaced for even intervals.** Where data is missing, the line breaks rather than bridging the gap.
- **Skip 3D, pies past three slices, and radar charts for comparing values.** Each bends what it shows.
- **Texture is switched on for a reason:** colour-vision deficiency, print or forced colours. It isn't ornament (`marks.md`).

## Shares of a total

People asking for a pie or a donut usually want to show how a total divides. A single ratio against a cap is a meter, above. Pick one of these and leave the donut out:

- **To rank the parts, tell near values apart, or show more than six:** horizontal bars, one for each part, biggest at the top, all one hue. Parts that have their own order, such as age bands, keep it. Lengths from a shared baseline are judged more precisely than pieces placed end to end.
- **To show the split of one total at a glance, alone in its card:** the tall segmented bar. Every part is a block with its percentage or amount printed on it if the text fits, plus a legend naming the parts. Six parts at most; a long tail nobody reads goes into "Other".
- **To show the split next to figures already on screen,** in table rows or above a list giving each part's name and amount: the thin segmented bar, without text, since the figures are already there. Give the table one legend, or put a swatch on each list row in place of a legend.
- **To show the split changing over time, or several totals side by side:** a stacked bar chart, turned horizontal when categories are many or their names long. The axis gives the values.

Specs for both segmented bars are in `marks.md`, under "Figures".

## Counting series

- **One to three series:** colour alone works for every reader; label the lines or bars directly.
- **Four:** bars, stacks and lines still hold, but direct labels are now required. Charts in which any mark can land beside any other (scatter, bubbles, maps, small multiples) top out at three: merge the rest into "Other" or split into panels. The fourth colour fails the all-pairs check in the default palette, and in most others.
- **Five or six:** treat this as the practical ceiling; use a legend, or panels.
- **Seven or more:** the palette has run out. Merge into "Other", split into panels, or add a second encoding such as marker shape.

From two series on, the chart has a legend. The tall segmented bar is the one exception: its legend replaces direct labels on the parts.

Don't invent a hue to cover extra series. A seventh colour looks like one of the six to a reader with a colour-vision deficiency, and it breaks the order the checks depend on. A `data` artifact accepts up to twelve series per chart, but repeats colours after six, so keep it to six.

## Small multiples

A grid of small charts, one for each series or group, sharing axes and scale. Reach for it when lines tangle, when series outnumber colours, or when shape matters more than exact values. A shared scale is what lets the eye compare panels; say so in the caption if one panel can't share it. Each panel's title names its series, so all panels can use one colour. The grid wraps to one column on a phone.
