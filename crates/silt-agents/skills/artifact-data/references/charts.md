# How a data page draws each chart

Silt draws every chart from your series alone, at the width it's shown and 260 px tall. Knowing how it draws them is how you choose the form that says what you mean.

## What every chart shares

- **The x axis is categories, in order.** Each distinct x value gets one evenly spaced position, in the order the values first appear, going through the series in turn. So:
  - sort the points before you write them, and put every x value in the first series, in order;
  - x values match across series only when they're exactly equal: `"1"` and `1` are two positions;
  - dates and times are spaced evenly whatever the gaps between them. For measurements at irregular times, say so in the title, or give a point per day with `null` where nothing was measured.
- **The y axis starts at zero,** or lower if a value is negative, and runs to the largest value (the largest stacked total for a stacked chart). It has three gridlines: the bottom, the middle and the top. A small change on a large baseline looks flat. When the change is the point, chart the change itself (a difference, or each value as a share of the first) in a series of its own, and keep the raw values in a table.
- **Colours are Silt's,** six per theme, assigned by series position. A seventh series repeats the first colour, so keep a chart to six series and put the rest in a table. Keep one entity in the same position across charts.
- **A legend** shows when there are two or more series. One series gets no legend, so the chart's title names it.
- **Hovering** a position shows its x value and every series' value there. The Table toggle shows the same as rows, x first and a column per series. Both come with every chart.
- **At a narrow width** the x labels thin out to fit, so keep them short: `a1b2c3`, `Nov 3`, `v1.4`.
- **`null`** in a series is a missing value: a bar or dot is left out there, and a line or area joins the points either side of it. Say so in the text when a gap matters.

## The forms

- **`line`:** change over an ordered x, such as time or commits, for one to six series. The default for a trend.
- **`area`:** a line with its area shaded to the baseline. Use it for one or two series where the amount under the line matters. Overlapping areas hide each other, so with more series use `line`.
- **`bar`:** comparing amounts across a few categories. Series sit side by side within each category. With `"stacked": true`, the series stack into one bar per category, for parts that add up to a whole.
- **`segmented`:** always stacked bars. Use it for the composition of each category, such as time spent per phase per commit. Keep the segments to six or fewer, and order them the same way in every category.
- **`dot`:** values as points at each x, without connecting lines. Use it when the x values aren't a sequence, so a line between them would imply a trend that isn't there.
- **`scatter`:** drawn the same as `dot`. The x axis is still categories in order, not a numeric scale, so it doesn't plot one measure against another. For a true scatter of two measures, use an `html` artifact or a table.
- **`heatmap`:** one value across two dimensions. Each series is a row, each x value a column, and each cell's strength shows its value, from zero (or the lowest negative value) to the highest, in one colour. There is no colour scale on the chart, so a reader gets exact values from hover or the Table toggle. Use it for patterns such as failures by test and day. It shows magnitude only: for values either side of zero, use bars.

## Choosing

| The question | The form |
|---|---|
| How did it change over time? | `line` (one to six series), `area` (one or two) |
| Which is bigger? | `bar` |
| What is it made of? | `bar` with `stacked`, or `segmented` |
| What are the values, unordered? | `dot` |
| Where are the hot spots? | `heatmap` |
| What's the exact value? | a `table` |
| What's the one number? | a `stat` |
| Two measures against each other | not a data page: an `html` artifact |
| Shares of one whole, one moment | a `bar` with `stacked` and one category, or a table; Silt draws no pie |

## Checking before you send

- Every chart names its source, and the title its unit.
- No chart has more than six series.
- The first series holds every x value, in order.
- Nothing has two y scales: two measures are two charts.
- Each chart is followed by a text block that says what to notice, unless its title already says it.
