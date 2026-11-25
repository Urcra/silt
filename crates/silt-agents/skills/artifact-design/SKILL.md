---
name: artifact-design
user-invocable: false
description: How to make an HTML artifact with silt_artifact that reads well and looks deliberate - the frame it runs in, the design system to use, and the design rules. Load it before writing any page, mockup, report or tool for silt_artifact.
---

# Making an artifact

Use `silt_artifact` to give the person a page beside the conversation. They may open it on a laptop, in the hosted app or on a phone, possibly on another machine after a handoff. They may also share it through a read-only link. Write each page for someone reading it cold.

Choose the kind first:
- `markdown` for prose with a little structure that needs no layout, such as notes or a plan.
- `mermaid` for one diagram on its own. Silt renders it in its own theme.
- `svg` for one drawing or chart with no interaction.
- `image` for a screenshot or a rendered picture, and `video` for a recording, from `path`, with no size limit.
- `html` for anything else: a mockup, a report with charts, a tool, a page with several parts. The rest of this skill is about HTML.

For a chart, also load `artifact-dataviz`. For a diagram inside a page, load `artifact-diagramming`.

## The frame

Silt shows an HTML artifact in a sandboxed frame with an opaque origin. Design around what it allows, rather than detecting what it refuses; it refuses the same for every viewer.

**Write a whole document.** It starts with `<!doctype html>`, then `<html lang="…">` and a `<head>` holding:
- `<meta charset="utf-8">`;
- `<meta name="viewport" content="width=device-width, initial-scale=1">`;
- a `<title>`;
- one `<style>`.

Silt inserts its policy and a small script right after the doctype. Leave the rest to you.

**What loads:**
- **Scripts** load only from `https://cdnjs.cloudflare.com`, `https://cdn.jsdelivr.net/npm/`, `https://unpkg.com`, `https://cdn.tailwindcss.com` and `https://code.jquery.com`.
  - Prefer cdnjs, and use the UMD build, which defines a global. Put its `<script src>` before the inline script that uses it.
  - Pin an exact version (`d3/7.9.0/d3.min.js`, `chart.js@4.4.7`), never a bare name, a range or `latest`.
  - Pick a version you knew before this session, or one at least two weeks old, so a release published yesterday can't reach the page.
- **Stylesheets** load only from `https://fonts.googleapis.com`, with font files from `https://fonts.gstatic.com`.
- **Everything else is blocked, with no error you'll see:**
  - another CDN (esm.sh included);
  - a library's own stylesheet from a CDN;
  - images by URL;
  - `fetch`, XHR and WebSocket, including calls a library makes for itself.

  Inline your CSS, and put images, data and any other font in the page as `data:` URIs or literals.
- **Use a library only when it does real work:** a chart with many marks, syntax highlighting, a 3D scene. Don't inline a library's source and don't hand-write a substitute for one. Most pages need none.

**What works, beyond the page's own scripts:**
- **Files the person picks:** `<input type="file">`, dropping files on the page, and `FileReader`, so a tool can take a CSV or an image without fetching anything.
- **Fullscreen** from a click (`requestFullscreen`); handle a refusal, as some browsers refuse it.
- **Links to `#name` inside the page don't scroll reliably.** Scroll with `scrollIntoView` from a click instead.
- **Sound** started from a click.

- **Per-viewer storage:** `localStorage` and `sessionStorage` work, kept for this viewer and this artifact by the page showing it. Use them for conveniences (a remembered tab, a draft), never for what others must see: wrap each read and write in `try`, and render without them.
- **Copy buttons:** `navigator.clipboard.writeText` from a click; catch its refusal and select the text instead.
- **Keeping state across versions:** a new version replaces an open page live. A game or a long form can keep its state with `silt.hot.snapshot(() => state)`, and start with `silt.hot.ready(start)`.

**Files beside the page.** A page that's more than one file gives the rest as `files` on `silt_artifact`, such as `{"data.json": {"content": "..."}, "css/site.css": {"path": "build/site.css"}}`, and loads them relative to itself: `fetch("data.json")`, `<link href="css/site.css">`. A later version keeps them unless it names one (`null` removes it), and `edits` with `"file": "data.json"` change one in place. Each is at most 16 MiB, and a version 64 MiB with its page. Keep data the page renders in a file like this rather than inlining it in the HTML.

**Capabilities, through `silt.use`.** A page that should remember what people do, share data, know who's viewing, hand over a file, ask an agent, take files or speech, call the viewer's tools or show who's here declares `capabilities` on `silt_artifact` and asks for each with `await silt.use(name)`. Load the `artifact-capabilities` skill before writing such a page: it says which capability fits which job and the rules for each, and `silt-artifact.d.ts` beside it types every call. Decide this before the design, since it changes what the page shows. Two rules hold for every one:
- Each resolves to `null` where it can't run (a read-only link, the offline copy, the check's headless browser), so render the page complete without it first and light the feature up when the promise resolves.
- Records people add belong in `db`, seeded with `silt_artifact_data` after the first version, never in the page's source or in `localStorage`.

**What does nothing:**
- **Storage beyond `localStorage`:** IndexedDB and cookies throw, as the frame's origin is opaque.
- **Dialogs:** `alert`, `confirm` and `prompt`. Build confirmation into the page.
- **Leaving the frame:** popups and `target="_blank"` links, downloads other than through `downloads`, printing, and form submission. Handle a form's `submit` with `preventDefault()`. Show a URL or an address as selectable text rather than relying on a link working.
- **Embedding and devices:** frames, `<object>` and `<embed>`; reading the clipboard; camera, microphone and location.

**What the viewer does with it:**
- **Comments.** The person selects text in the page to comment on it, and the comments come back to you as a prompt for a new version. Keep text selectable (never `user-select: none` on content), and keep the words you'd expect them to quote in real text, not in images or canvas.
- **Width.** The page opens in a panel beside the conversation, often only 420–700px wide, and in a full view as wide as the window. On a phone it fills the screen, at about 360px. Design it to work at every one of those.

**Size.** A page is at most 10 MiB, everything inlined included. An image or a video shown as its own artifact has no limit.

**No redaction.** An artifact is kept and synced as written. Never put a secret, token, password, private key or anything from an `.env` file into one, even as an example.

## Name it

The `title` you pass is the artifact's name in the thread's list, on its card and in search. Make it a short noun phrase of two to four words that picks this page out among others, such as "Rescan benchmark" or "Sidebar states". Don't use a category alone ("Report", "Mockup") and don't add an explanation after a colon or dash. When the person already has a name for the thing, use theirs. Keep it the same across versions.

Put the same name in `<title>`.

## Use the design system

Silt prepends the design system's tokens to the frame as CSS custom properties, for both themes. It sets `data-theme="light"` or `data-theme="dark"` on the root, from the viewer's own Silt theme. `silt_artifact_guide` says which system applies and lists its token names. Its README says how to use them.

**Precedence:**
1. The person's own words.
2. The design system `silt_artifact_guide` says applies, or one the person names.
3. The project's own look, from its stylesheets, theme files and component code, for anything that shows or mocks up its product.
4. A look you choose from the subject and commit to.

Silt ships no design system of its own to fall back on. When none applies, don't borrow another project's system to skip the decision: that's how pages end up looking like every other page.

**Using the tokens:**
- Use the system's tokens with a fallback, for example `color: var(--fg-1, #2e2620)`. The fallback keeps the offline copy, which has no injected tokens, readable.
- Use them for colour, type, spacing and radii. Don't copy their values in as literals: the tokens are what follow the viewer's theme.
- When the page is a mockup of the product the system belongs to, it uses the system's components as its README describes them, at their real sizes. It shouldn't look like a generic web app.
- **A system with components** has their CSS in the frame already, and `silt_artifact_guide` shows each one's markup. Use that markup and its classes, changing only the content and the state shown. Don't redraw a component the system has.
- With no system, or for a page that isn't about the product, still work from a small token set of your own (below).

## With no design system

- **A mockup of the project's own product:** read the real thing before the first line. Its stylesheets or Tailwind config give the colours, type scale, spacing and radii, and the component code of the screen gives each control's padding, height, border and states. Copy those values; a mockup in a neutral look of your own tells the person nothing about their product.
- **Any other page:** settle a look you could name in a few words ("a field notebook: cream, ink blue, mono labels" or "a control room: dark slate, one signal amber"), drawn from the subject's own world. Say it in one line of your reply, and build it through the token plan below.
- **Check the plan against habit.** If a choice is the one you'd make for any page of this kind (a white ground, a blue accent, Inter throughout), change it unless the subject is truly neutral.

## Make it work in both themes

The viewer reads in light or dark. Silt sets `data-theme` in its frame. The offline copy and the read-only link may set nothing, so the system's preference has to work too.
- **Define every colour as a custom property on bare `:root`,** light values first.
- **Redefine only the tokens for dark, twice:** once under `@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { … } }` and once under `:root[data-theme="dark"] { … }`, each with `color-scheme: dark` so form controls and scrollbars follow.
- **Style every component through the tokens,** never with a literal colour that works in one theme only. Never define a colour only inside a dark block.
- **Give `body` an explicit background from a token.** Behind a transparent page the viewer sees Silt's own panel.
- **Design the dark theme;** don't invert the light one. Keep contrast and keep the accent legible on both grounds.
- **A single look is allowed only when the page deliberately commits to one,** such as a terminal screen or a print proof. Even then, set its background and every colour explicitly.

The design system's injected tokens already follow this pattern. When you use them, add only what the page needs on top.

## Lay it out for every width

- **Gutter.** At least 16px at every width, set once on `body` or one outer wrapper: `padding-inline: max(16px, env(safe-area-inset-left, 0px))`.
- **Siblings.** Lay out sibling groups with flex or grid and `gap`, not with margins on each element.
- **Narrow widths.** Let rows wrap, and stack them to one column below about 560px. A flex or grid child that holds text, code or a table gets `min-width: 0`.
- **Nothing scrolls sideways** except a wide table, code block or diagram, each inside its own `overflow-x: auto` box. Give images and fixed-ratio boxes `max-width: 100%`. Give nothing a `min-width` wider than a phone.
- **Height.** Size a hero or a header to its content, not to `100vh`. For a one-screen tool, give `html` and `body` `height: 100%`.
- **Columns of digits** get `font-variant-numeric: tabular-nums`.

## Read the request before designing

Every page is designed; the treatment changes with what it's for.
- **Most pages are working documents:** a plan, a comparison, a benchmark report, a review of options, a demo of a feature.
  - Give them real typographic hierarchy, considered spacing and a proper palette.
  - Don't over-design them: no big hero, no decoration, few flourishes.
  - A reader should find the point in the first screen.
- **A page meant to be kept, shown or played** (a landing page, a game, a tool the person will reuse, a visual identity) earns an editorial treatment. Make opinionated choices, and take one real risk where it serves the subject.
- **A mockup of a product's UI** is information design and fidelity, not art direction. See "Mockups and decks" below.

When unsure, a quiet, well-composed page is always acceptable. An over-designed one often isn't.

## Ground it in the subject

- Name the one subject, who reads the page and the single thing it's for. Everything else follows.
- Draw choices from the subject's own world: its units and scales, its documents, its vocabulary. Include at least one detail only this subject would have, as content rather than ornament. A benchmark report shows the machine and the commit it ran on; a migration plan names the tables.
- Use real content. Use the person's real data where you have it. Where you don't, use clearly marked examples, never presented as their figures. Write `[price]`-style placeholders rather than invent facts. Never use lorem ipsum.

## Plan the tokens first

Before writing the page, settle a small design plan and write it into the page as the first rule of its `<style>`:
- **A comment** of one line saying the layout idea.
- **Colour:** 4–6 named tokens covering ground, raised surface, text, muted text, accent and line. Take them from the design system where there is one.
- **Type:** tokens for at least two roles, text and headings, plus a mono or data face if the page shows code or numbers.

Then build everything from those tokens. Don't narrate the plan to the person; one plain sentence about the direction is the most to say.

For an editorial page, review the plan against the subject before building. If any part is what you'd use for any page of this kind, change that part. Then:
- **The first screen states the thesis.** Open with the most characteristic thing in the subject's world: a headline, an image, a live demo, an interactive moment.
- **Type sets the personality.** Pair the faces deliberately, avoiding the ones you'd use on any other page, and set a clear scale with intended weights and spacing, so the type itself is memorable.
- **Motion is a choice.** Where it serves the subject (a load sequence, hover detail, an ambient touch), one orchestrated moment beats scattered effects, and often less is more.
- **Match the execution to the vision.** A maximal direction needs elaborate execution; a minimal one needs precision in spacing and detail.
- **Put the boldness in one place** and keep what's around it quiet. If the accent clashes with the ground, move it to a neighbouring hue or soften it rather than replacing it.

## Type

- **Faces.** With a design system, use its faces; for the project's product, the faces it ships. Otherwise pair a text face and a heading face that suit the subject. Inter for text with Space Grotesk for headings suits a subject that's truly neutral; don't reach for it first. Load them from Google Fonts with `display=swap`, and give every face a real fallback stack.
- **Measure.** Keep running text near 65 characters wide.
- **Scale.** Pick a type scale and keep to it. Keep the number of sizes small.
- **Headings** get `text-wrap: balance`. Body text gets comfortable line height. Small uppercase labels get a little letter-spacing.
- **Weights** carry hierarchy as much as sizes do.

## Colour and surfaces

- **Neutrals.** Choose them deliberately. A grey with a slight bias toward the accent's hue reads as intended; a flat mid-grey reads as a default.
- **The accent** marks one or two things per screen: the primary action, the current item, the line that matters. If everything is accented, nothing is.
- **Semantic colour** (good, warning, bad) is separate from the accent. It always comes with a word or an icon too, never colour alone.
- **Borders, fills, radii and shadows** each say "this is a separate object". Use them by role, on the element that needs setting apart. The same card, radius and shadow on every block flattens the page.
- **Contrast.** Text holds 4.5:1 against its ground in both themes, and large text 3:1. So do borders, icons and focus rings that carry meaning.

## Repeated elements

Cards in a row, rows in a list, badges on siblings: give each the same edges, the same inner padding, the same baseline, and anything that recurs in the same place. Let content set heights. Pick a column count the items fill, so nothing stretches over empty space or sits alone in a row. Text that can outgrow its box wraps or scrolls inside it; clipped text is a bug.

## Pages that are tools or dashboards

A tool or a dashboard is scanned and operated, not read from the top.
- **Order.** Put the summary before the detail.
- **State.** Show it in form as well as in numbers: a pill, a dot, a stripe, so what needs attention is visible at a glance.
- **Interactive elements look interactive.** They're real `<button>`, `<input>` with `<label>`, and `<a href>` elements, with a visible focus state and targets of at least 32px, or 44px on touch.
- **The first view shows the tool working,** on the person's real data or on marked example data. An empty shell waiting for input shows nothing.
- **Big-number tiles** open a page only when those numbers are its point.

## Mockups and decks

- **A mockup with more than one state or size,** or options to compare, is a `canvas` artifact: load `artifact-canvas`, which holds the guidance for mockups.
- **A presentation** is a `slides` artifact: load `artifact-slides`.
- **A page of figures** such as a benchmark report or a dashboard is a `data` artifact: load `artifact-data`.

## Charts and diagrams

- **Charts:** load `artifact-dataviz` and follow it. In short: pick the form before the colours, draw to one scale, label only what the chart reaches, colour chart text from the theme tokens, and put a hover readout on interactive charts.
- **Diagrams:** load `artifact-diagramming`. Draw the mechanism (what flows where, what changes between options), with labelled arrows, as inline SVG in `currentColor`.
- **Generative or decorative graphics:** draw them with canvas or WebGL rather than long hand-written SVG paths.

## Make it complete at rest

- **What's meant to be read is visible on load.** Nothing waits at `opacity: 0` for a scroll observer. Something may animate in, but from a visible resting state.
- **Motion is optional:**
  - one orchestrated moment is worth more than many small effects;
  - nothing bounces in a working document;
  - `prefers-reduced-motion: reduce` turns motion off.

## Avoid the generated look

Generated pages fall into a few recognisable looks. Unless the person or the design system asks for one, don't use:
- warm cream with a serif display and a terracotta accent;
- near-black with one acid-green or vermilion highlight;
- a purple-to-blue gradient hero on white;
- emoji as section markers or icons;
- everything centred;
- the same large radius on every element;
- cards with a coloured bar down one edge;
- numbered markers (01, 02, 03) on content that isn't a sequence;
- a small uppercase eyebrow over every heading;
- one word of a headline in the accent colour.

Structural devices encode something true: numbers mean an order the reader needs, a divider separates things that differ.

## Write the words well

- **Name things by what the reader recognises,** not by how the system is built.
- **Use active voice and short sentences.** A button says exactly what it does ("Merge", "Retry the sync"), and what follows says it happened ("Merged").
- **An error says what went wrong and how to fix it,** without apology.
- **Avoid mannered devices:** dashes for asides, "not X but Y", a colon before a reveal, scare quotes, stock phrases.
- **Prefer the specific to the clever.**

## Build it cleanly

- Close every element and double-quote attributes. Give form controls stable `id`s.
- Watch the cascade: two rules for one element both setting spacing is how layouts break quietly. Prefer one class per job.
- Fonts that fail to load fall back silently. Check that every family you use is either loaded or in the fallback stack.
- Don't add global key handlers. The page shares the keyboard with Silt's own shortcuts while it has focus.

## Versions

- **Calling `silt_artifact` with the `id` it returned adds a version.** Change only what was asked. Keep the title, the token plan and the structure, so the versions compare.
- **Keep the page's source in a file outside the repository's checkout,** for example under the system's temporary directory, and pass its `path`. Revising the file then gives the next version. A file inside the checkout would show in the thread's changes.
- **After a handoff or a fork,** or whenever you don't have the file, call `silt_artifact_read` with the id. It writes the latest version's source to a file and says where.
- **When comments arrive,** each quotes the text it's about. Find that text and change what the comment asks. Answer in your reply anything you didn't change, and why.
- **Silt may report problems** the viewer's frame saw in a version: blocked loads, script errors, a page wider than its frame. Treat each as a bug in the next version.

## Before you publish

- **Look once.** After the first version, call `silt_artifact_check` with its id. It renders the page in a headless browser at a phone's width and a desktop's, in light and dark, and says what was blocked, what failed and whether it's too wide, with a screenshot of each. Look at the screenshots once, fix what you see in one new version, and stop; don't loop. Where no headless browser is installed, it says so.
- **For a page with real numbers,** spend that look on the chart.
- **For a page whose point is logic** (dates, money, parsing), run its core function once on a sample.
- **For a page that declares capabilities,** also make the one data check `artifact-capabilities` describes: a `silt_artifact_data` read of what it stores, and the same read at a lower `as_level` where its rules hide something. Then say in a line what you checked and what you couldn't.
- **Without a headless browser,** publish as you are. The person reviews it live, and their comments and their browser's problems come back to you.

After publishing, tell the person in a line or two what the page is and anything you assumed. Don't describe its design.
