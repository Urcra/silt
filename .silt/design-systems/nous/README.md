# Nous

Nous is Silt's own look. It's warm and engineer-first, with restraint and density on paper-cream and tobacco neutrals, and a single amber accent doing most of the work. Use it for anything that shows Silt: mockups of its screens, reports about its threads, pages for its people. It should feel like a well-kept notebook of code: serious, considered, a little analog.

Silt injects every token below into the frame as a CSS custom property, for both themes, and sets `data-theme` from the viewer. Write `var(--fg-1, #4a3f2e)`, with the light value as the fallback; never paste values in as literals.

## Content

- **Tone.** Quiet confidence. Short, declarative, present tense, second person: "You direct. Your agents work." No exclamation marks, no hype words (supercharge, seamless, revolutionary, AI-powered).
- **Casing.** Sentence case everywhere: buttons, menus, headings, page titles. Title Case never appears in the product. Uppercase only for tiny labels, key hints and badges, set in `meta` with letter-spacing.
- **Code nouns are mono.** Branch names, paths, thread ids, commands, agent names and PR numbers are set in the mono face (`code`) even inside prose: "Pick up `auth-refactor`".
- **Metadata** goes in `meta` and `fg-3`, joined with middle dots: `claude · auth-refactor · 12m ago`. Units are short and lowercase: `12m ago`, `4 commits`, `3 agents`.
- **Empty states** say the next action ("Add a project to start"), not the absence ("No projects").
- **Errors** name what failed, then the fix: "Couldn't reach `origin`. Check the token in Settings › Servers."
- **No emoji** in the product or its pages. Status is a dot, a word, a small icon. Glyphs such as `→ · ⌘ ⏎ ⌥` are fine in hints.
- **Silt's vocabulary:** thread, fork, merge request, handoff, checkout, project, workspace, machine, server, turn, prompt, agent. A thread "hands off" to another machine; a fork "merges back".

## Colour

- **Ground.** `bg` is the page. Cards and panels sit on `surface-1`, hovered rows on `surface-2`, pressed states on `surface-3`. Menus, popovers and dialogs use `bg-elevated` with `border` and `shadow-2`.
- **Text.**
  - `fg-1` is body text and titles.
  - `fg-2` is secondary text and icons beside labels.
  - `fg-3` is metadata only, on `bg`; on `surface-1`, use `fg-2` instead.
- **Lines.** Every border is a 1px hairline in `border`. `border-strong` marks a selected or focused container. `border-subtle` separates items inside a card. Borders are never decorative.
- **Accent.** `accent` (amber) appears once or twice per screen: the primary button (with `fg-on-accent` text, never cream), the focus ring, the current item's marker, the series a chart is about. Accent-coloured text uses `accent-fg`. A selected row is `accent-tint` with a 2px `accent` bar at its leading edge; that's the only place a coloured edge appears.
- **Status.** `ok` (olive), `warn` (honey), `danger` (rust) and `info` (slate) are desaturated, never pure red or blue. Each goes on a 6px dot or text, always with a word (`warn` only as a dot or badge in light, as text it is too faint), and on its `-tint` for a badge's fill. Added and removed diff lines use `ok-tint` and `danger-tint`.
- **Hover and press.**
  - Hover lifts a row to `surface-2`. A button darkens slightly, with no change of hue or size.
  - Press goes one step further.
  - Disabled is 40% opacity, never desaturated.
- **No gradients, glows, glass or coloured shadows.** Modals dim the page with `backdrop`, with no blur.

## Type

- **Faces.** IBM Plex Sans for everything (`--font-sans`) and IBM Plex Mono for code (`--font-mono`). Silt loads both from Google Fonts. There's no serif and no other display face.
- **Sizes.**
  - The product is dense: `ui` (13px) is the base for rows, buttons and labels; `small` (12px) for secondary lines; `meta` (11px) for metadata.
  - Prose such as messages and documents is `body` (14px, line height 1.6).
  - Titles are `title-sm` (15px, 500) and `title` (18px, 600).
  - Documents and marketing pages may use `display` (32–64px, weight 500, tracking -0.02em, line height 1.05).
- **Weights.** 400 by default, 500 for emphasis and titles, 600 for page titles only. At 11–12px on a dark ground, use 500 to keep it readable.

## Space and shape

- **Spacing.** A 4px base. Use `space-1` to `space-12` (4, 8, 12, 16, 24, 32, 48).
  - A list row is 28–32px tall, not 48.
  - A card's padding is `space-3`, a panel's `space-4`.
- **Radii.**
  - `radius-sm` (2px) for inputs, buttons, chips and badges.
  - `radius-md` (4px) for cards and menus.
  - `radius-lg` (8px) for dialogs.
  - Nothing is rounder, except avatars and status dots (`radius-full`).
- **Shadows.** Resting cards have none. `shadow-1` is for a raised interactive card, `shadow-2` for menus, popovers and a dragged item.
- **Layout of the product.**
  - A 40px top bar, a project sidebar of 240–280px, the view, and an inspector panel of about 360px on the right.
  - Chrome sits on the page with hairlines, not shadows.
  - On a phone, the sidebar becomes a drawer, panels become full-screen sheets, and a tab bar replaces the status bar.
- **Motion.**
  - Transitions use `ease`: `duration-micro` for hover and press, `duration-panel` for panels.
  - Fades and slight slides only: no bounce, spring or scale.
  - All motion goes under `prefers-reduced-motion`.

## Components

Silt's own components are in every frame, as the `n-` classes `silt_artifact_guide` shows: `n-button` (with `n-primary`, `n-ghost`, `n-danger`, `n-sm`, `n-lg`), `n-icon-button`, `n-field`, `n-select`, `n-tag`, `n-chip`, `n-kbd`, `n-dot`, `n-pill`, `n-section-label`, `n-card`, `n-row`, `n-banner`, `n-empty`, `n-diff` and `n-status-bar`. Use them for anything Silt draws; the rules below say what they encode and cover what has no class:
- **Buttons:**
  - **Primary:** an `accent` fill with `fg-on-accent` text.
  - **Secondary:** a `surface-1` fill with a `border` hairline and `fg-1` text.
  - **Quiet:** no fill, `fg-2` text, `surface-2` on hover.
  - **Size:** 28px tall in the product (32px at most), `ui` text, `radius-sm`, and a 16px icon at `space-1` from its label.
- **Inputs.** 28–32px tall, with a `bg-elevated` fill, a `border` hairline, `radius-sm` and `ui` text. Focus is a 2px `accent` outline at a 2px offset, with no glow.
- **Rows** in lists and the sidebar are 28–32px tall. A status dot comes first, then the title in `ui`, then metadata in `meta` and `fg-3` on the right. The selected row is `accent-tint` with its accent bar.
- **Cards.** A `surface-1` fill with a `border` hairline, `radius-md` and `space-3` padding. The title is in `title-sm`, the metadata row under it in `meta`.
- **Badges.** `meta` text, uppercase optional, on a status `-tint` with the status colour for text, `radius-sm`.
- **Code and diffs.** `code` on `surface-1`, with line numbers in `fg-3`, added lines on `ok-tint` and removed lines on `danger-tint`. Comments sit under their line in a `bg-elevated` card.
- **Icons.** Outline icons with a 1.5px stroke, 16px by default (14px in dense rows, 20px in section heads), in `currentColor`: `fg-2` at rest, `fg-1` on hover. Never filled and outline icons mixed together, and never emoji. In an artifact, draw them as inline SVG.

## Charts

Use `chart-1` to `chart-6` in that order for categorical series. They pass the palette validator on `bg` in both themes. To emphasise one series, draw it in `accent` and the rest in `chart-muted`. Gridlines and axes use `chart-grid`; chart text uses `fg-2` and `fg-3`. Status colours are never series. Where colour can't carry a series alone (print, forced colours), lay `chart-texture` over its fill as a `background-image`, and `chart-texture-cross` over a second's.
