---
name: artifact-diagramming
user-invocable: false
description: When a diagram helps in an artifact and how to draw one that shows the real mechanism - what to draw, Mermaid or inline SVG, and SVG that stays legible in both themes. Load it before drawing a diagram for silt_artifact.
---

# Drawing a diagram

Draw as the engineer who has to live with the decision. A diagram earns its place when it shows a cold reader a mechanism they would otherwise have to assemble from prose: where data flows, which parts talk to which, what a request passes through, what changes between two options, which states a thing moves between. If a sentence says it as fast, write the sentence.

## What to draw

- **The mechanism, not its name.** A box labelled "cache" adds nothing to the word. The path a read takes through it, the two stores it sits between, and the arrow that goes away without it say what prose can't. Draw the parts the argument turns on (the boundary crossed, the hop added, the data that moves) and leave the rest out.
- **For a choice, the difference.** Put the options side by side, drawn the same way, so the one edge or box that differs is what the eye finds. A before and an after, or two architectures, with the edge each adds or removes marked. Separate boxes per option, unconnected to the system they change, only restate a list.
- **As much as the decision needs.** A one-hop question takes three boxes. A change that reroutes writes through a queue needs the writer, the queue, the reader and the ordering. Don't force it smaller, and don't draw the whole system.
- **Labels on arrows.** An unlabelled arrow means "related somehow". `writes`, `invalidates`, `polls every 30s` and `ACP over stdio` are information. Add a legend only when one encoding (dashed, coloured, doubled) repeats; otherwise label the mark itself.
- **Real names.** Use the codebase's own names for crates, functions, tables and endpoints, in its mono face, so a reader can find each one.

## Which kind

- **A diagram that stands alone** (a sequence, a flow chart, a state machine, an entity diagram): `kind: "mermaid"`, which Silt renders in its own theme. Keep node labels short.
- **A Mermaid diagram inside a page:** a `<pre class="mermaid">` block holding the diagram's text, with no library. Silt finds each one when the page loads, draws it with its own Mermaid in the viewer's theme, and swaps the block for an image, again when the theme changes. At most 20 per page, each under 64 KiB. Give the `<pre>` an `aria-label` stating the diagram's claim: it becomes the image's alt text. As it becomes an image, its words can't be selected or commented on, so put the claim in a caption too.
- **Inline SVG** for anything Mermaid lays out poorly: a comparison drawn side by side, a diagram with a deliberate layout, one element highlighted, or labels a person will comment on. Hand-drawn SVG, as below.
- **Inside a typed artifact,** follow its own skill: a slide's or a board's `html` takes inline SVG at the stage's real size (`artifact-slides`, `artifact-canvas`); a whiteboard draws with its own blocks, arrows and sections (`artifact-whiteboard`); Markdown and docs draw no diagrams, so a diagram a doc needs is a separate `mermaid` or `html` artifact the doc names. These mechanics apply there only where those skills say nothing about diagrams.

## Inline SVG

Hand-author `<svg>` with native shapes (`rect`, `circle`, `line`, `polyline`, `path`) and `<text>`. No libraries, no runtime, no external images.

- **Size by `viewBox`.** Set `viewBox="0 0 W H"` for the content, not a preset, and `style="max-width: 100%; height: auto"`. Wide flows read left to right; layered stacks read top to bottom. A diagram too wide to read at a phone's width goes in a box with `overflow-x: auto`, at its drawn size, rather than shrinking its text.
- **Ink.** Strokes, text and arrowheads in `currentColor`, so they follow the page's text colour in both themes; set the SVG's `color` from a token (`color: var(--fg-1)`). Fill boxes with the page's surface token (`var(--surface-1)`) or `none`, and give every shape an explicit fill: SVG's default is black. Keep one literal hue, the page's `accent`, for the one element that carries the point (the option leaned towards, the hop under discussion), and check it reads on both grounds.
- **Arrowheads.** A `<marker>` in the SVG's own `<defs>`, used with `marker-end="url(#arrow)"`, its id unique on the page, its path filled with `context-stroke` or `currentColor`. Or a small `<polygon>` at the line's end. Never an image.
- **Text.**
  - 11 to 13px at the drawn scale, aligned with `text-anchor` and `dominant-baseline`.
  - Labels of a word or three. Explanation belongs in the caption below the figure, not in the drawing.
  - Fonts from the page's tokens (`var(--font-sans)`), the mono face (`var(--font-mono)`) for code names.
  - Leave room in the `viewBox` for the outermost labels; a label cut at the edge is a bug.
- **A grid.** Align boxes to shared baselines and even gaps, on a unit such as 8px. Eyeballed offsets read as noise; shared edges and centres are most of what makes a hand diagram look deliberate.
- **One figure, one claim.** Wrap the SVG in `<figure>` with a `<figcaption>` saying what it shows. Give the `<svg>` `role="img"` and an `aria-label` with the same claim, for readers who can't see it.
- **Self-contained.** No `<script>`, `<style>` or `<foreignObject>` inside the SVG, and no external images. Gradients, patterns and `<use>` refer to ids in the same SVG (`href="#id"`).
- **Simple shapes.** Long hand-written path data means the drawing wants simplifying, or a canvas.

## Before publishing

`silt_artifact_check` renders the page in both themes. It compares text by its CSS `color`, so SVG text drawn with `fill="currentColor"` is checked and text with a literal `fill` isn't: one more reason to keep literals out. Look at the screenshots for labels that collide or run past a box.

A diagram is text a person can comment on: keep labels as `<text>`, never as outlined paths, so they can be selected.
