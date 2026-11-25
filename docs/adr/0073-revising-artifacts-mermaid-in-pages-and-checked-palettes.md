# 0073: Revising an artifact in place, Mermaid in pages, notes on versions, and palettes Silt checks

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0068](0068-artifacts.md) (`silt_artifact`'s inputs), [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md) (the palette validator, `silt_artifact_check`, the design system's chart tokens)

## Context
Several small gaps make artifacts in Silt hard to revise and check:
- **Revising.** Changing one line of a page means sending the whole page again as `content`, or writing it to a file and passing `path`. After a handoff, the agent first has to call `silt_artifact_read`.
- **Mermaid inside a page.** A Mermaid diagram inside an HTML page needs Mermaid's library from a CDN. Only a whole artifact of `kind: "mermaid"` renders on its own.
- **No way to say what changed.** Versions carry a number and nothing else, so the version menu can't say what each one changed. Nothing describes the artifact on its card.
- **Theme bugs go unreported.** `silt_artifact_check` reports blocked loads, script errors and overflow. It misses the commonest bug in generated pages: a colour that ignores the theme, such as dark text on a dark ground.
- **Palettes need Node.** `artifact-dataviz`'s validator is a Node script, so an agent without Node can't check a palette, and no Rust test can check the built-in design systems with it. It also leaves out several of the checks the method relies on: a lightness band per theme, a chroma floor, ordinal ramps and text contrast.
- **No chart scales in the design systems.** Nous and Plain define a categorical chart palette but no sequential or diverging steps, which the skill tells agents to use.

## Decision
**`silt_artifact` takes `edits`.**
- **The shape:** `edits: [{old, new}]` with an `id`, in place of `content` or `path`.
- **How they apply:**
  - each `old` must occur exactly once in the latest version's source;
  - the edits apply in order, all or none, and make the next version;
  - a miss changes nothing, and the error names the edit and why it failed.
- **Where the source comes from:** the daemon reads it from the latest version's blob. That works after a handoff or a fork, without a `silt_artifact_read` first.
- **Only text kinds take edits:** `html`, `svg`, `markdown` and `mermaid`.

**Notes on artifacts and versions.**
- **A version's note:** `note`, at most 200 characters, saying what this version changed. The version menu lists it beside each number.
- **An artifact's description:** `description`, at most 300 characters. The card and the gallery (ADR 0075) show it under the title. A version without one keeps the previous description.
- **Where they're kept:** both go on the artifact record (`ArtifactView.note`, `ArtifactView.description`), with `serde(default)`, so older readers ignore them.

**Mermaid in a page, drawn by the holding page.**
- **Finding diagrams:** the frame's first script finds each `<pre class="mermaid">` once the page has loaded and sends its text to the holding page. It's at most 64 KiB and 20 diagrams per page.
- **Drawing them:** the holding page draws each with the web UI's own Mermaid (`securityLevel: 'strict'`) in the viewer's theme, and sends back SVG.
- **Showing them:** the frame swaps the `<pre>` for an `<img>` of that SVG, so nothing in it runs and it needs no network. A theme change draws them again.
- **The other surfaces:**
  - the read-only link's page does the same once it renders Mermaid (the Todo for links);
  - the offline copy gets each diagram drawn into it when it's downloaded.
- **`artifact-diagramming`:** "Mermaid in a page would need its library" becomes "use `<pre class="mermaid">` for a diagram Mermaid draws well, inline SVG for the rest".

**`silt_artifact_check` checks the theme.**
- **What it compares:** it renders in light and in dark already, and now compares the two. For each element with text, it reads the computed text colour and the nearest background behind it.
- **What it reports:**
  - `theme`: the body's background is the same in both themes, or a colour comes from no custom property and is the same in both;
  - `contrast`: text under 4.5:1, or large text under 3:1, in either theme.
- **Limits:** at most 10 of each, each naming a selector.
- **The viewer's frame doesn't run this check;** it needs both themes rendered.

**One palette validator, in Rust, as `silt design palette`.**
- **Where it lives:** `silt_design::palette`, pure, so it builds for wasm. The CLI command is `silt design palette "<hex,…>" --surface <hex> [--theme light|dark] [--pairs all] [--ordinal] [--json]`.
- **The categorical checks:**
  - **lightness band:** OKLCH L within 0.43–0.77 on a light surface and 0.48–0.67 on a dark one, replacing the 0.3 spread;
  - **chroma floor:** C ≥ 0.10;
  - **separation under colour-vision deficiencies:** protan and deutan gate (OKLab ×100: 8 passes, 6 to 8 warns, under 6 fails), on adjacent pairs or `--pairs all`; tritan is reported only;
  - **normal-vision floor:** 15, a hard failure;
  - **contrast:** 3:1 against the surface, a warning.
- **`--ordinal`** checks a ramp instead: lightness strictly monotone, each step ΔL ≥ 0.06, the light end at least 2:1, and one hue.
- **Text contrast:** `silt design contrast <text> <ground>` gives WCAG 2's ratio and APCA.
- **It replaces `validate-palette.mjs`,** so no agent needs Node, and the skill names the command.

**Design systems get chart scales.**
- **Sequential:** `chart-seq-1` … `chart-seq-7`, light to dark, with separate dark-theme values.
- **Diverging:** `chart-div-neg-3` … `chart-div-neg-1`, `chart-div-mid` (a neutral grey) and `chart-div-pos-1` … `chart-div-pos-3`.
- **Status:** a fourth step, `serious`, between `warn` and `danger`. Chart slots must never reuse a status value.
- **The built-ins:** Nous and Plain get all of these.
- **Tests:** a test in `silt-design` runs the validator over each built-in system's categorical palette in both themes, adjacent and all-pairs (the first three), and the sequential ramp as ordinal.
- **The `design-system` skill** asks for these tokens and names the command.

## Consequences
- **Small revisions are cheaper,** and the agent doesn't need the page's file after a handoff.
- **Pages can use Mermaid** with no network and no library. The diagram is an image, so its text can't be selected for a comment; the skill says to keep the claim in the caption.
- **The version menu says what changed,** and cards say what an artifact is.
- **`silt_artifact_check` catches most theme bugs** before the person sees them.
- **Checking a palette needs only `silt`.** The thresholds now live in one place, which tests use too.
- **`silt design` is a new command group,** holding `palette` and `contrast`, so the CLI reference changes. None of this changes the wire format: the new record fields are optional.
