# 0105: The plugin marketplace, self-hostable as Silt is

- Status: proposed
- Date: 2026-11-18
- Builds on: [ADR 0104](0104-one-plugin-model.md) (one plugin model)
- Planned for M35. Until then, plugins come from git (ADR 0104).

## Context
ADR 0104 makes every plugin a WebAssembly component from a git repository. People need a place to find plugins they can trust, and orgs need one of their own for plugins they don't publish.

Silt is self-hostable (ADR 0032), so the marketplace must be too.

A marketplace people can trust needs submissions reviewed by people, verified publishers, every build scanned and signed, a way to pull a bad plugin, and no update to a third party's plugin without the person's say.

## Decision
**A marketplace is a git repository of entries, built and signed by its CI, and served by any `silt-server`.**

### The index
- **What an entry is:** one file per plugin, `plugins/<publisher>/<id>.toml`, in an index repository. It names:
  - the plugin's repository;
  - each published version's tag and commit;
  - the publisher.
- **Submitting:** a publisher adds a plugin, or a version of it, with a PR.
- **The index's CI:**
  - checks out the tagged commit;
  - checks the manifest: valid, a license, capabilities with reasons, no name like another's;
  - builds the plugin with the pinned toolchain in a clean sandbox;
  - for a plugin with `mcp-process`, builds or fetches the server it declares, pinned by hash, and signs it with the plugin;
  - runs the plugin test harness against a stand-in host, to check it loads and does nothing it doesn't declare. A wrapped MCP server runs in its sandbox there, and must reach only its declared hosts;
  - records the `.wasm`'s sha256.
- **Review:** a maintainer reviews the PR. The review checks the capabilities and hosts above all.
- **On merge,** CI signs the build and the manifest with the marketplace's key (minisign), and publishes them.

### Serving it
- **Any `silt-server` can serve a marketplace** at `/marketplace/v1`, from an index repository and its built artifacts:
  - `index.json`: each plugin's manifest summary, versions, publisher, verification, capabilities and hosts;
  - `plugins/<id>/<version>.wasm`, with its signature;
  - its README and docs, for the plugin's page;
  - `blocklist.json`, signed.
- **The public marketplace** is a `silt-server` the Silt project runs, which Silt points at by default.
- **An org's server can serve the org's own marketplace,** from its own index repository. Its policy can make that the only marketplace its members' machines use, or one beside the public one.
- **The keys:** each marketplace has its own signing key. A machine trusts the keys of the marketplaces it's configured with, and nothing else.

### Publishers
- **Verified** by:
  - a GitHub organisation or user that owns the plugin's repository;
  - or a domain, through a DNS TXT record.
- **The publisher-trust prompt** from ADR 0104 names the publisher's verification and how long they've published.

### Installing, updating and the blocklist
- **Installing:**
  - The download is checked against its sha256 and the marketplace's signature before it's loaded.
  - The install dialog shows everything its manifest declares.
- **Updating:**
  - Silt checks the marketplace for updates daily, and when Settings › Plugins opens.
  - It updates a third party's plugin only when the person accepts. The update dialog shows what the new version adds.
  - Silt's own plugins update with Silt.
- **The blocklist:** a blocklisted version is turned off on every machine that fetches the list. The person is told why, and the plugin's data is kept until they remove it.

### Offline and local
- **Without the marketplace,** installed plugins keep working, and Browse says it can't reach it.
- **Installs from git and from a dev folder** never need a marketplace.

## Consequences
- **New pieces:**
  - **`silt-server`:** the marketplace's routes, an index reader and a signature store.
  - **A CI workflow for index repositories:** build, harness and sign. It's published as a reusable GitHub Actions workflow, so an org's index repository reuses it.
  - **`silt plugin search|install|update`.**
  - **Settings › Plugins › Browse.**
- **The public index needs maintainers to review submissions.** The guide sets out what review looks at.
- **The protocol:** marketplace requests are plain HTTPS, outside sync, so `PROTOCOL` doesn't change.
