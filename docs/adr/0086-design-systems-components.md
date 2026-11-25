# 0086: A design system's components

- Status: accepted
- Date: 2026-11-17
- Amends: [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md) (what a design system holds, what the guide gives, what syncs)
- Amended by: [ADR 0087](0087-no-built-in-design-systems.md): Nous moves into Silt's repository, Plain is removed, and the guide lists no built-in systems
- Amended by: [ADR 0088](0088-design-systems-in-settings.md): components' guidelines; touch sizes on touch screens only

## Context
ADR 0071 made a design system a README and tokens. A system's components were described only in its README, in prose. So an agent mocking up a product rebuilt each button, row and card from that prose, and its mockups looked close to the product without matching it. The agent needs the real components to use instead.

A second problem showed in use: with no system applying, the guide listed the built-in `plain` as one choice among others, and plain's README asked to be chosen whenever a project had none. Agents took it, so mockups and pages came out in the same neutral look.

## Decision
**A system may hold its components.**
- **The files:**
  - `components.css`: the components' rules, built on the system's tokens, at most 128 KiB;
  - `components/<name>.html`: an example of one component, its variants and states as the product draws them, at most 16 KiB each. There are at most 48 of them, and at most 128 KiB together.
- **Names:** a component's name follows a system's name: lowercase letters, digits and hyphens. A leading `<!-- … -->` comment in its file is its one-line usage note.
- **Classes:** a system prefixes its classes so they don't meet a page's own. Nous uses `n-`. Silt doesn't enforce it.

**The frame gets the components' CSS.**
- It's compiled after the tokens, unlayered. A library's layered reset, such as the Tailwind CDN's preflight, doesn't undo it, and the page's own rules come later, so they win at equal specificity.
- A file holding `@import`, `</`, or a `url()` that isn't a `data:` URI is left out whole, and the compile reports it, as a dropped token is reported.
- The CSS is part of the compiled copy each version keeps (ADR 0071), so a version renders the same after a handoff, behind a read-only link and offline.

**The guide gives the markup.** `silt_artifact_guide` lists each component of the system that applies, with its note and its markup. `artifact-design` and `artifact-canvas` say to use that markup and change only content and state.

**They sync with the rest of a person's system.** `DesignSystemCopy` gains `components_css` and `components`, and the protocol goes to 16. A server keeps each copy as it's sent, so an older server would drop them.

**Nous carries Silt's own components**, written from `web/src/ui`: buttons, an icon button, a field, a select, tags, chips, a key hint, a status dot, a pill, a section label, a card, a list row, a banner, an empty state, a diff block, a status bar and its icons. Plain has none. A test checks that every custom property its CSS reads is a Nous token, and that every `n-` class its examples use is defined.

**When no system applies, the guide says where the look comes from:**
1. a mockup of the project's product matches the product, from its stylesheets and component code;
2. anything else gets a look chosen from the subject.

The built-in systems are listed apart, as for pages about Silt (`nous`) or a document the person asked to be plain (`plain`), and not as a fallback.

## Consequences
- Mockups of a product with a system that has components match it to the pixel the CSS gives, without the agent re-deriving each control.
- A system's components show only through mockups for now. The web UI has no page that previews them.
- The guide grows by the components' markup, up to 128 KiB for a system that uses the whole allowance. Keeping each example to its variants keeps it short.
- Syncing components is new on the wire, so every machine and the server update together.
- Pages made with no system no longer share plain's look. They depend on the agent's choice, which the guidance steers but doesn't check.
