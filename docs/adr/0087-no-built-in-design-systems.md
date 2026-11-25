# 0087: Silt ships no design system of its own

- Status: accepted
- Date: 2026-11-17
- Amends: [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md) (where design systems come from), [ADR 0086](0086-design-systems-components.md) (Nous's components)

## Context
ADR 0071 built two design systems into the binary, below a repository's and the person's own:
- **Nous:** Silt's own look;
- **Plain:** a neutral look for pages with no brand.

Plain turned out to be a trap. With none applying, agents passed it rather than choose a look, so pages came out looking the same. ADR 0086 told agents not to treat it as a fallback, which left it with no use: a page a person asks to be plain needs no system for that.

Nous is only useful for pages about Silt itself, and those are made while working on Silt. Shipping it put it in every person's list of systems, beside their own projects'.

## Decision
**Silt ships none.** Design systems come from a repository's `.silt/design-systems/` and the person's own, nothing else. `DesignSource` loses `Builtin`.

**Nous moves into Silt's repository** as `.silt/design-systems/nous/`, components included, and `.silt/design-systems/default` names it. Silt's own threads get it as any repository's threads get its default. Its tests read it from there: that its colours match `web/src/nous.css`, that its components read only its tokens, and that its chart colours pass the palette checks.

**Plain is removed.** Its chart colours, checked in both themes, become `artifact-dataviz`'s default palette in `references/palette.md`, for a page with no system. A test runs the palette checks on that table as it's written.

**A name that isn't there no longer fails.** A last version, a project pick or a workspace default may name a system this machine lacks: `plain` or `nous` from before this change, or one only another machine has.
- **A new version** goes unstyled, and `silt_artifact` tells the agent which name was missing.
- **The guide** says that default was passed over, and that none applies.
- **A name the agent asks for** must still exist.
- **The pickers** in Settings show a missing pick as "not on this machine".

## Consequences
- Every version made before keeps the CSS it was made with, so nothing already made changes.
- A workspace or project that picked Plain or Nous gets no system until someone picks another. The guide says why.
- People who liked Nous for their own pages can copy the folder into their own `design-systems/`.
- The dataviz guidance no longer leans on two systems' values; it has one checked default and the method for checking any other.
