# What goes wrong

Hold each chart up against this list before you publish. A chart that fits an entry needs fixing. Every entry is a mistake that has turned up in real dashboards.

## Colour and encoding

- **Two value scales on one plot.** Where the two axes line up is an arbitrary choice, so the picture suggests the measures move together when the data says nothing of the kind. Users (0 to 30k) drawn against sessions (0 to 800k) look linked only because of where the axes were set. Draw two charts, or small multiples, or rebase both series to 100 at the start on a single axis.
- **Colours assigned by rank,** so dropping one series through a filter shifts every other colour. Someone who learnt that the daemon is teal now reads the wrong line. Tie each colour to its entity; the remaining series keep theirs.
- **An extra colour past the sixth,** invented or borrowed from the start of the list. It looks like an existing slot to colour-blind readers and breaks the order. Put the tail into "Other", split into panels, or add a second encoding. A `data` artifact reuses colours after six series, so give it no more than six.
- **Checking colour-blind safety by looking.** It isn't a check. Run `silt design palette`: 8 apart for neighbours, or 6 to 8 with a second encoding, and 15 for normal vision.
- **Testing neighbours only, for a scatter plot.** Any two dots may end up together there, so the default test misses real clashes. Use `--pairs all` and stop at three series.
- **Testing one theme.** Passing on the light surface says nothing about the dark one. Run the test for each theme with its own `bg`.
- **Shading named categories darker when bigger** (products, teams, endpoints). The bar length already gives the size, so the colour is wasted, and the categorical test fails as it should. A single series wears one colour; ordered categories get an ordinal ramp checked with `--ordinal`.
- **Bending a sequential ramp until the categorical test passes.** Ramps are meant to fail that test. Use `--ordinal` on them.
- **A rainbow for magnitude.** It shows bands that aren't in the data. Use one hue from light to dark, with a scale legend.
- **A coloured midpoint, or two cool poles, on a diverging scale.** The middle should look like nothing and the two ends like opposites: slate against rust, not slate against teal.
- **Status colours on ordinary series,** or series colours standing for status. Only use status tokens when the colour means good or bad.
- **Colour doing all the work.** Two series that differ only in hue, with no legend, label, marker shape or dash, can't be told apart by everyone.
- **Hex values in a page that has a design system.** A literal that suits one theme turns into dark-on-dark in the other, and `silt_artifact_check` flags it. Use the tokens.

## Form

- **Six colours for a story about one number.** This is the usual way a chart loses its point. Highlight the one series in `accent` and grey the rest, or show a stat tile.
- **A single bar, or a pie with two slices.** A stat tile says it better.
- **Pies and donuts for similar values.** Nobody compares angles precisely. Rank them as bars, or print the figures. For up to six shares of one total seen at a glance, use the tall segmented bar.
- **Seven or more meaningful colour classes.** Beyond that, neighbouring classes run together. Show a table, or a table and a chart.
- **Bars that begin above zero.** A bar's length is its value, and a truncated axis can make a small gap look three times larger. Start at zero, or switch to dots.
- **Irregular time on a regular axis,** such as monthly data with a missing month drawn as if continuous. Space points by date and break the line where data is missing.
- **3D, drop shadows on marks, gradient bars.** All of them warp the reading and add ink that carries nothing.

## Marks and chrome

- **Heavy bars, strong gridlines, no breathing room.** At scale it looks crude and shouts. Use thin marks, faint grids and generous padding, and keep saturated colour for small marks and accents. The tall segmented bar is the exception.
- **Dashed grids or axes.** Dashes suggest a forecast or a threshold. Grids are faint solid lines; a dashed line is reserved for a target or projection and says which.
- **Values printed on every point.** The reader stops reading them. Label a few, and keep the legend from two series up. The tall segmented bar is the exception.
- **A legend holding one swatch.** It only repeats the title. A single series needs no legend.
- **Outlines around marks to keep them apart.** Leave a 2px surface-coloured gap between fills and a 2px halo around overlapping markers.
- **Labels cut off by, or spilling past, their bar or segment,** including text cropped by hidden overflow. Measure first, then move the label past the bar's end, or leave it to the tooltip and legend.
- **Line-end labels pushed so far apart they no longer touch their lines.** Use leader lines, panels, or the legend and tooltip.
- **A fixed chart height that leaves no room for the x-axis labels,** giving the card its own scrollbar. Include the axis band, or let the card grow.
- **A whole `viewBox` shrunk to fit a phone,** with text left at 6px. Redraw at the shown width and skip some x labels.
- **A display or serif font on the hero figure.** It turns the number into decoration. Use the body sans.
- **Tabular digits on a big lone number.** Fixed-width digits leave `121` looking gappy. Use proportional digits there and tabular ones in columns.
- **Hatching always on, or used as ornament.** Diagonal line fields can upset motion-sensitive readers and look busy. Turn texture on only when needed, at 45° and 135°, and order it on value scales.
- **Words printed in series colours.** Pale hues are unreadable as text. Use the text tokens and put a swatch beside the words.
- **A label for a value the axis never reaches,** or ticks computed from a different scale than the marks. Use one scale for everything.
- **Titles that just name the axes.** State what the chart shows.

## Interaction and access

- **Values you can only see by hovering.** Put each one in a label or the table view as well, and make keyboard focus show what hover shows.
- **Tiny targets,** such as an 8px dot that must be hit exactly. Give it 24px including gap and halo, or a nearest-point layer for crowded scatter plots.
- **Tooltips that run off the edge** of the chart or a phone screen. Flip them inward.
- **A filter per chart, or filters inside a card.** One row at the top scopes everything below it, so the figures agree.
- **Placeholder skeletons flashing on every redraw.** Dim the previous drawing instead, without moving anything.
- **No table twin, or a continuous scale shown by colour alone.** Every chart has a table, and every colour scale a legend with values.
- **Names pushed in with `innerHTML`.** A series called `<img onerror=…>` would execute. Use `textContent`.
- **Charts that animate onto a page meant to be read,** or that ignore `prefers-reduced-motion`. A loaded page is a finished page.
- **An empty box and no words** while data loads, or when there's none. Say what will appear and how it gets there.
