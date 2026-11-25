# The cover

Every system has a `cover.html`: what a person remembers the system by, its colours and its shapes, with its name as the figure. Silt draws it at the top of the system's page, 960 px wide, scaled down to fit narrower windows, with the system's compiled tokens in its frame and the viewer's theme. Write it last, from what you built.

`cover.html` is a fragment, like a component's example: no doctype, `<html>`, `<head>` or `<body>`. It opens with a comment of four lines, its derivation: the blocks (tokens by sizes), the arrangement, the pattern and the sentence that chose it, and the steps and radii used. Then one `<style>`, one inline SVG and the two text elements. At most 64 KiB.

## Three parts, every time

1. **Colour blocks.** The palette as large solid fields, not chips: three to five blocks in the system's identity colours at full value (brand, accent and signal hues, a deep ink or dark surface, one tint for relief), together a quarter to half of the cover. Weight them by identity, not by how often the UI uses them: a neutral-first system leads with its one or two hues, and a colour that only means a state (error red) stays small. Their sides are spacing steps or multiples, their corners radius tokens. Arrange them as the brand composes: a flush grid, a staggered stack, one tall slab with satellites, a strip of unequal bands, blocks bleeding off an edge. Never a row of equal squares: that's a swatch table.
2. **One pattern,** over or between the blocks, in block colours, chosen from what the system says about itself in its README and usage notes:
   - soft and friendly, large radii: discs, pills, half-rounds from the radius scale;
   - precise, technical, mono, a 4 px grid: a dot or plus grid at one spacing step;
   - editorial, serif, borders rather than shadows: a few hairline rules in its hairline token crossing the blocks;
   - loud and bold: stripes, chevrons or a checker at a spacing pitch;
   - geometric and dense: tiles cut by the radius tokens, some merged two-wide;
   - a literal motif (a wave, a leaf, a lens): arcs or that silhouette built from radii;
   - a distinctive display face: one oversized glyph used as a shape, clipped by a block.

   Shapes, not pictures: no faces, mascots, icons or scenes. A dozen to sixty units for tiles or dots, three to six rules, never hundreds.
3. **The name** in the display face (or the body face at display weight), up to 120 px, leading 0.9 to 0.95, bottom left on the ground in the ink token, with one tagline line under it at 13 to 14 px in the muted text token: the README's own, or one sentence in the system's voice. No other words. The name reads exactly as the README writes it, on two lines at most, breaking only at a space or hyphen it already has. Keep the text block within 440 px; every block and pattern unit sits right of x = 480 px so nothing crosses the text. A name too wide at 64 px takes the other layout: a band of blocks and pattern across the top, 96 to 120 px tall, with the name beneath it.

## Rules

- **Every fill, stroke and radius is a class bound to a token** (`.brand { fill: var(--accent) }`). No images, and no gradients or shadows the tokens don't define.
- **Both themes are the same file.** Look at it in dark: a block within a few percent of the ground vanishes there, so bind it to another token rather than outlining it.
- **One layout, at 960 px wide.** Nothing responsive: no width queries, no `vw` sizes.
- **No motion.**
- **Squint:** the name reads first, then colour, then pattern. If the pattern wins, halve its count. With a thumb over the name, the blocks and pattern alone should say which brand this is.
- **Avoid** the barcode (a field of thin vertical rules), the swatch table, confetti (more than five colours or two patterns), and words on blocks.

Make it again when the name, palette, display face or scales change, not for smaller edits. A person changes it by asking: keep the derivation unless the ask changes it.
