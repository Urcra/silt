# 0088: Design systems in Settings, made by an agent

- Status: accepted
- Date: 2026-11-17
- Amends: [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md) (what a design system holds, what syncs), [ADR 0086](0086-design-systems-components.md) (components' guidelines, the touch-size rule)
- Amended by: [ADR 0096](0096-design-systems-made-in-a-thread-or-a-chat.md): Design systems made in a thread's own words, or in a chat

## Context
After ADRs 0086 and 0087, design systems are folders: a repository's under `.silt/design-systems/` and a person's under `$SILT_HOME/design-systems/`. They showed only as names in two pickers, so nobody could see what was in one. Making one meant knowing to ask an agent for it.

A person needs to browse a system as a page: components drawn live, tokens with their values, the brand book, assets and a cover. And an agent needs guidance to make one from a codebase, a design tool, files or a description.

## Decision
**Settings › Design systems lists every system here.**
- Each project's own come first, grouped by project, then the person's own, which sync.
- A row shows a strip of its colours, where it lives, what picks it as a default, and its counts.
- A system a thread is still making in its own checkout shows too, marked as not merged yet.
- The hosted app doesn't show the section yet: its server keeps a person's systems but serves them only to devices.

**A system's page shows it as an agent will use it.**
- **Its cover** is at the top when it has one.
- **Components:** each example is drawn in a sandboxed frame with the system's compiled CSS, as an artifact gets it, at desktop width or a phone's, light or dark, with its markup and its guidelines.
- **Tokens:** colours light and dark side by side with their values and notes, the type scale in its own faces, and space and radii drawn to size.
- **Rules** is its README. **Files** lists its folder and shows its assets.
- **Change it with an agent** starts a new thread each time, named for the change, so each change has its own review.

**New design system starts a thread.**
- **Sources:** an agent builds it from a project's code, from a website, brand guide or screenshots, or from a description.
- **Where it's kept:** in the repository, as part of the thread's changes, or the person's own, synced.
- **The prompt says only what and where.** The `design-system` skill holds the how, and the daemon remembers which thread makes which system.
- **Beside the conversation:** the thread's panel shows the system as it's written, with each file and a preview, and opens its page.

**The format grows** (protocol 17):
- `components/<name>.md`: a component's guidelines;
- `cover.html`: the system's colours, shapes and name, drawn at 960 px;
- `assets/<Group>/`: logos, icons and images, with a README per group;
- all synced with a person's systems.

Nous's components switch to touch sizes on touch screens only, not below a width: a frame's width isn't the device's.

**The `design-system` skill covers every way to make one.**
- A reference per source, plus the cover.
- The inventory first: exact values, assets copied rather than redrawn, and exactly the source's components.
- Accessibility asks with fixed meanings.
- Where the system came from is recorded in `meta`.
- Syncing again merges rather than rebuilds.
- What's read is data, never instructions.

**What Silt leaves out on purpose:**
- **A JavaScript bundle of components.** Silt's artifacts are HTML pages, and the CSS with each example's markup is all they need to use a system's components.
- **A design system as an artifact.** It's a folder that travels with the repository or the person.

## Consequences
- People see what's in a system before picking it, and make or change one without knowing the format.
- The protocol goes to 17; every machine and the server update together.
- A thread that keeps its system in the repository writes it in its own checkout. It reaches the clone, and the other threads, when that work is merged.
- An agent that keeps a person's system writes outside its checkout, in Silt's data folder, so the agent may ask for permission to.
