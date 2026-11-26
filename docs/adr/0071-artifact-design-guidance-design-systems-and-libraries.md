# 0071: Artifact design: built-in guidance, design systems, and libraries from CDNs

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0068](0068-artifacts.md) (the frame's network, redaction of artifacts)
- Amended by: [ADR 0072](0072-what-an-artifacts-page-can-do.md): the frame's contract gains `silt.use`, per-viewer storage and `clipboard-write`
- Amended by: [ADR 0073](0073-revising-artifacts-mermaid-in-pages-and-checked-palettes.md): the palette validator moves into `silt design palette`; `silt_artifact_check` checks the theme; chart scales in design systems
- Amended by: [ADR 0074](0074-an-artifacts-files-and-assets.md): the frame's policy names its own address
- Amended by: [ADR 0076](0076-artifact-types-slides-canvases-and-data-pages.md): types' guides and skills
- Amended by: [ADR 0086](0086-design-systems-components.md): a system's components; what the guide says when none applies
- Amended by: [ADR 0087](0087-no-built-in-design-systems.md): Silt ships no design system; Nous moves into Silt's repository and Plain is removed
- Amended by: [ADR 0088](0088-design-systems-in-settings.md): components' guidelines, a cover and assets; Settings shows and makes systems
- Amended by: [ADR 0132](0132-no-telemetry-and-an-offline-switch.md): artifacts' libraries load through Silt by default

## Context
Agents in Silt make poor pages, mockups most of all, because of the context they get: one sentence, a frame with no theme, and no network. A good page needs a page contract and design guidance before its first line, the person's design systems, and scripts and fonts from a known set of CDNs.

## Decision
**Built-in skills, written for Silt.**
- **The skills:**
  - `artifact-design`: the frame's contract and the design guidance;
  - `artifact-diagramming`;
  - `artifact-dataviz`: the method, the active system's palette and a palette validator;
  - `design-system`: how to write one.
- **How they ship:** in the binary, as a lowest-precedence `SkillScope::Builtin`, so a repository's skill of the same name overrides one.
  - They're offered only with `silt mcp --ui`.
  - The files a skill names are written under `$SILT_HOME` once per Silt version, so an agent can run them.
- **One step before the first line:** `silt_artifact_guide { kind }` returns the guidance for that kind, the design system that applies (its README and token names) and the frame's rules. `silt_artifact`'s description says to call it first.

**Design systems are a README and tokens.**
- **What one is:** a folder holding `README.md`, `tokens.json`, `fonts/` and `assets/`. A repository names its default in `.silt/design-systems/default`.
  - `README.md` is the brand book: usage rules that name tokens.
  - `tokens.json` lists themes, then colours (each with a value per theme), type, spacing, radii and shadows. Each token has a usage note.
  - DESIGN.md is imported and exported, not stored.
- **Where they come from, highest precedence first:**
  1. a repository's `.silt/design-systems/<name>/`;
  2. the person's `$SILT_HOME/design-systems/`;
  3. built-in ones: Nous (Silt's own look, its colours checked by a test against `web/src/nous.css`; ADR 0039) and a plain neutral one.
- **A person's own sync through each server they use** (protocol 12). A device sends a system's README, tokens and fonts, stamped when it last changed there, and the copy changed last wins. One removed travels as a tombstone. The server keeps each person's as files in its data folder.
- **Which one applies to a new version:** the one `silt_artifact`'s `design_system` names (`none` for none), else the one its last version used, else the one the person picked for the project on this machine, else the repository's default, else the workspace's (ADR 0065).
- **Each artifact carries its copy.** Each version names its system and the blob of the compiled tokens it was made with, attached beside the session like its content. It renders the same after a handoff, in the hosted app and behind a read-only link.
- **The frame gets the theme:**
  - The compiled tokens are prepended as CSS variables for both themes, with `data-theme` and `color-scheme` set to the viewer's Silt theme.
  - A theme change is sent again by `postMessage`.
  - Variables use the system's own names. A system's `aliases` map gives tokens second names, and the built-in ones map MCP Apps' standard names (`--color-background-primary`, `--color-text-primary`, `--border-radius-md`, …) onto theirs, so a page written for an MCP host reads them too.

**Libraries and fonts load directly from a CDN allowlist.**
- **The frame's CSP allows:**
  - scripts from `cdnjs.cloudflare.com`, `cdn.jsdelivr.net/npm/`, `unpkg.com`, `cdn.tailwindcss.com` and `code.jquery.com`;
  - stylesheets from `fonts.googleapis.com`;
  - fonts from `fonts.gstatic.com`;
  - fonts, images and media from `data:`.
- **Still closed:** `connect-src` stays `'none'`, and the frame keeps its opaque origin.
- **Read-only links:** `/r/<token>/content` sends the same CSP.
- **Versions:** the guidance asks for exact versions at least two weeks old. Silt doesn't enforce it: a page that breaks the rule loads what it names.
- **Fonts:** a face with a file under the system's `fonts/` is inlined into the compiled tokens as a `data:` URI. A face without one, Nous's IBM Plex included, loads from Google Fonts by its family name.
- **A proxy, opt in.** Direct loading is the default, so pages load their libraries with nothing to set up or store. A machine (Settings › Appearance) or a server (`artifact_proxy` in `server.toml`) can load through Silt instead. It fetches each pinned file of the allowlist once, keeps it (at most 512 MiB), and serves it from `/vendor/`. The frame's addresses are rewritten to it, and its policy then names only that route. The daemon's route sits outside its token check, as a frame has no token, but takes only pinned files of the allowlist. The offline copy keeps the CDNs' addresses.

**Artifacts aren't redacted.**
- An artifact is shown, not run, and redaction corrupted pages: a long base64 `data:` URI, an SRI hash or a hex id can read as a secret.
- `put_artifact` keeps the content as written.
- Prompt attachments stay redacted.

**The viewer checks the page.**
- A probe in the frame reports to the UI:
  - blocked loads (`securitypolicyviolation`);
  - script errors;
  - horizontal overflow.
- They show on the version, and Send to agent sends them as comments are sent.
- **The agent checks too:** `silt_artifact_check` renders a version where Node and Playwright are installed, at a phone's width and a desktop's, in light and dark, in a sandboxed frame like the panel's, and returns what went wrong and a screenshot of each. `silt_artifact_read` gives back a version's source, to revise it after a handoff.

## Consequences
- Pages in Silt get design guidance from the start, and match Silt or the project's own brand without the agent being told each time.
- **Network privacy is reduced by default.** A viewer who opens an artifact contacts the CDNs it names, and so does anyone with a read-only link, unless the proxy is on. A page that uses no library or web font contacts nothing. On a machine without internet, such a page renders without its libraries or fonts, unless the proxy kept them.
- **Secrets can sync.** A secret an agent writes into a page syncs to the server as written. The agent is told so in `artifact-design`.
- The tokens copy is another attachment beside the session. Syncing a person's design systems is new on the wire, so the protocol goes to 12.
- **A repository's systems travel with the repository, and a person's through their servers.** A system made on a machine with no server stays there.
- A repository can override any built-in skill or design system by name.
