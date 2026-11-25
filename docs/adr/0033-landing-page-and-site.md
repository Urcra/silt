# 0033: The landing page and the site

- Status: accepted
- Date: 2026-10-21

## Context
The plan for the web UI has two landing pages on the canvas: one for a developer working alone, one for teams and agents. It also leaves open where the site and docs are hosted, and under which domain. The docs site is built (`crates/silt-site`), static, and hosts anywhere.

## Decision
- **One landing page, for the developer working alone, with teams as a section of it.** Silt's focus (ADR 0006) is handing your own work between machines, forking it and keeping its history. A team gets that too, through a server it hosts. Two pages would say the same things twice.
- **It's part of the docs site.** `docs/landing.md` is rendered by `silt-site` as the site's `index.html`, in the same Nous colours, without the sidebar. Its links are checked like every page's.
- **No domain in the site.** Every link is relative, or to the repository. Where it's hosted, and under which domain, is a deployment choice. It can be made later without changing the site.

## Consequences
- The site's root is the landing page; the guide starts at `guide/index.html`.
- A second landing page for teams can still come, if one page proves too much for both.
