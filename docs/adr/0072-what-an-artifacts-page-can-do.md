# 0072: What an artifact's page can do: capabilities through the viewer

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0068](0068-artifacts.md) (a frame that only shows), [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md) (the frame's contract), [ADR 0027](0027-read-only-links.md) (what a link's page reads)
- Amended by: [ADR 0074](0074-an-artifacts-files-and-assets.md): `assets`, and `connect-src` to the frame's own address
- Amended by: [ADR 0077](0077-docs-and-sheets-edited-together.md): `PageOp::Text`, `Lines` and `Cell` for docs and sheets
- Amended by: [ADR 0078](0078-a-page-reads-the-threads-files.md): `files`
- Amended by: [ADR 0079](0079-voice-in-a-page.md): `dictation` and `tts`
- Amended by: [ADR 0080](0080-a-page-calls-mcp-servers.md): `mcp`
- Amended by: [ADR 0081](0081-live-in-a-page-rooms-and-whiteboards.md): `room`
- Amended by: [ADR 0082](0082-comments-inside-docs-and-pages.md): Comments inside docs and pages

## Context
An HTML artifact can only be shown. Its frame has an opaque origin and no network (ADR 0068), so storage throws and nothing connects. A poll forgets its votes, a checklist its ticks, a tracker its rows. A tool can't save a file it made, can't tell who's using it, and can't ask the agent anything. The only messages between the frame and Silt are the viewer's theme going in, and the selected text and the frame's problems coming out (ADR 0071).

A page that does real work needs more: shared documents under access rules, saving itself as a new version, knowing who's viewing, asking the agent, downloads, comments, uploads, connectors, live rooms, dictation and speech. Without them, an agent asked for a tracker makes one that only looks like one.

Silt has most of what these need already:
- the log, which a thread's data can live in and fork with;
- the server's accounts, and the record of which user owns each actor;
- `write_as`, through which the hosted app already writes comments as the signed-in person;
- the thread's agent;
- the comment store.

What's missing is a safe way for the page to ask for these.

## Decision
**A bridge through the viewer, never through the frame.**
- **Calls:** the frame's first script defines `silt.use(name)`, which resolves to the capability's functions, or `null` when this view can't run it. `null` covers four cases, and the page can't tell which: not declared, not granted, not served here, or failed.
- **Transport:** each call is a `postMessage` to the page holding the frame, which answers it.
- **Checks in the holder:** the holding page takes only messages whose source is that frame. It checks the call against the version's declared capabilities and the viewer's grants, then makes the request to `/api` or `/app/api` with its own session.
- **The frame keeps its opaque origin** and `connect-src 'none'`, and never sees a token or an address of Silt's. That stops a page fetching, not sending: it can still navigate itself to another site with what it read in the address, so a page can send out anything it can read, and each capability is granted with that in mind. What it can no longer do once it navigates is keep the bridge: every message the frame's script sends carries a nonce from its first script, the holder hears nothing without it, and after each load it sends nothing until the frame answers with it, so a document the frame navigated to gets none of the page's grants.
- **Grants are kept for what the version declared.** The person's answer is stored with the capability's declaration in the version they saw; a later version that declares more, written by anyone or by the page itself, asks again. For `sample`, `mcp` and `files`, which run the page's own code with the person's agent, connectors or files, the answer is kept for the page as well, so a new page asks again too. The daemon and the server check the grant on `sample`, `files` and `mcp` themselves, behind the holder.
- **Limits:** a message is at most 1 MiB, save where a capability takes or gives more: `artifact.publish` takes a page of up to 10 MiB, `assets` and `downloads` a file of theirs, `files.read` answers with a text file of up to 4 MiB and a `db` query with up to the artifact's 16 MiB. Each capability has a rate limit.
- **Answers:** structured-clone data only. A refusal rejects with a `code` (`not_granted`, `rate_limited`, `conflict`, `too_large`, `refused`) and words to show.
- **Where the bridge runs:**
  - the daemon's web UI and the hosted app serve the full bridge; both serving modes answer it through `silt_proto::view`;
  - a read-only link's page serves `db` reads only, frozen where the link is;
  - the offline copy gets a stub whose every `use` resolves `null`, so a page written for Silt still opens anywhere.

**Declared per version, versioned as a contract.**
- **Declaring:** `silt_artifact` takes `capabilities: {name: config}`, kept on the version's artifact record (`ArtifactView.capabilities`).
- **Changing it:**
  - leaving the field out of a new version keeps the previous one's;
  - `{}` clears it;
  - an object replaces the whole set.
- **The contract's version:** `silt.version` names it. The frame's API is typed in `silt-artifact.d.ts`, which `silt_artifact_guide` returns for the capabilities an artifact asks about and `artifact-design` points to.
- **A version keeps its contract:** it runs against the contract it was made with until a later version moves it.

**Consent where a page acts on the viewer's behalf beyond the thread.**
- **The rule:** a capability that only reads or writes the artifact's own data in a thread the viewer can already write asks nothing.
- **What asks:**
  - `sample` asks once per viewer per artifact;
  - each `downloads.save` is confirmed by the viewer;
  - a refusal is final for that page load.
- **Where grants live:** with the viewer, in `artifact_grants` on the daemon and the server, beside the person's other state. They never sync into the family log.

**Always on, not capabilities:**
- **`localStorage` and `sessionStorage` work, per viewer.** The holder writes the viewer's saved values for this artifact into the frame's first script, so the page's `localStorage` answers at once. Writes go back by message. The values are kept in the holding page's own storage, namespaced by artifact id, at most 1 MiB. Where the frame loads from a frame address (ADR 0074), they travel with the request for its key and are held with it, in memory, until the frame first loads, when the machine or server serving the frame composes them in and drops them; nothing else stores them. (Amended: they were held for the key's hour. Handing them over by message instead was ruled out, as a page reads `localStorage` as its first script runs, before any message could arrive.) Existing pages that remember a tab or a draft then just work.
- **State survives a new version arriving live.** A page may register `silt.hot.snapshot(fn)`. When a version arrives while the page is open, the holder asks the old frame for its snapshot (500 ms at most), then starts the new frame with it in `silt.hot.data`.
- **Copy buttons.** The frame is allowed `clipboard-write`; reading the clipboard stays refused.

**The first capabilities:**

- **`db`: the artifact's data, kept in the thread's log.**
  - **A new op family,** `Op::Page(PageOp)`, added last, since stored deltas name variants by position. Each op names the artifact, a document path and the writer:
    - `Set { artifact, path, value }` with a JSON value;
    - `Patched { artifact, path, fields }`, merging top-level fields;
    - `Deleted { artifact, path }`.
  - **Paths and limits:**
    - a path is 2 to 8 segments of `[A-Za-z0-9_-]{1,64}`; even counts are documents and odd counts collections;
    - a document is at most 64 KiB;
    - an artifact holds at most 10 000 documents and 16 MiB.
  - **Folding:** the data is a projection of the log, folded in the log's order in `silt-log`, so every replica agrees. Queries (`where`, `orderBy`, `limit`) are evaluated by one pure function in `silt-proto`, which both serving modes use.
  - **Live updates:** `onSnapshot` hears changes as the thread view hears new entries.
  - **It goes with the thread:**
    - a fork inherits the data as it was at the fork, and then the two diverge;
    - a merge doesn't bring the fork's data back, as it doesn't bring the conversation (`ConvOp::Merged`);
    - rewinding the thread (ADR 0029) rewinds its data with it;
    - search doesn't index it.
  - **Rules:**
    - the declaration may give `rules: [{path, read, write}]`;
    - levels are:
      - `read`: anyone who can read the thread, a read-only link included;
      - `comment`: anyone signed in whom the thread's visibility admits, as a comment needs (ADR 0014);
      - `collaborator`: the owner and the thread's collaborators, who may write the thread itself (ADR 0014);
      - `owner`: the thread's owner;
    - a `{self}` segment matches the writer's own user id, so "everyone reads, each writes their own" is two rules;
    - without rules, `read` reads and `comment` writes, so a poll works for everyone who can see it.
  - **Enforcement:**
    - The server checks each `PageOp` when it's pushed. It uses the user that owns the delta's actor, as it already does to drop deltas written as someone else's actor, and drops what the version's rules refuse.
    - A daemon checks its own writes the same way. With no server there is one person, who is the owner.
    - The `by` a delta carries is only for display.
  - **The agent:** `silt_artifact_data { id, op, … }` reads and writes the same data as the run's person, so the agent can seed a tracker and read back what people entered.
  - **Untrusted:** everything in it came from people and pages that may be hostile, so the skill tells pages to insert it with `textContent`.

- **`artifact`: a page saves itself.**
  - `artifact.publish(html, {base})` adds a version authored by the viewer, under the same rules as the agent's: at most 10 MiB, and the design system carried over.
  - It's refused with `conflict` unless `base` is still the latest version. Every open view then reloads to the winner.
  - The daemon writes it as the agent's are written. The server writes it with `write_as`, as it writes comments.
  - It needs `collaborator`, as writing the thread does. Other viewers get `not_granted`.

- **`user`: who's viewing.**
  - `isOwner()`, `canWrite()`, and `id()`, an opaque id stable per server.
  - `profiles(ids)` gives display names, which the page shows and never stores.
  - A read-only link and the offline copy get `null`.

- **`downloads`:** `save({filename, data})` asks the viewer, then saves through the holding page.
  - The file is at most 16 MiB, with a cleaned-up name.
  - Executable types are refused.

- **`comments`:** `openComposer({text})` opens Silt's own comment composer on this version, quoting `text`, as selecting text does. The comment then goes the way every artifact comment goes (ADR 0068).

- **`sample`: asking a model from the page.**
  - **Where it runs:** on the viewer's own daemon, as a one-off call through the thread's agent adapter (Claude Code's print mode first), with no tools and no session. It never enters the thread's history and never costs the thread's agent a turn.
  - **Costs:** the viewer's credentials pay, and the viewer is asked first.
  - **Limits:** at most 4 calls a minute per artifact; the answer streams back.
  - **In the hosted app:** the call goes to one of the viewer's own online devices through the control channel (ADR 0026), as a command the daemon checks like any other. It's `null` when none is online or none has the capability.
  - **On read-only links** it's always `null`.

**Decided in later ADRs:**
- `assets`: [ADR 0074](0074-an-artifacts-files-and-assets.md);
- `files`: [ADR 0078](0078-a-page-reads-the-threads-files.md);
- `dictation` and `tts`: [ADR 0079](0079-voice-in-a-page.md);
- `mcp`: [ADR 0080](0080-a-page-calls-mcp-servers.md);
- `room`: [ADR 0081](0081-live-in-a-page-rooms-and-whiteboards.md).

## Consequences
- **Pages do work in Silt.** A poll, a checklist or a tracker the agent makes keeps what people do in it, and an agent can seed and read it.
- **The data is the thread's.** It hands off, forks and rewinds with the thread. It also syncs to the server, is subject to the project's visibility, and leaves with a purge.
- **`Op::Page` is new on the wire,** so the protocol goes to 13. A server older than this drops a page's writes; a client older than this ignores them.
- **A viewer can now author a version,** so an artifact's versions name their authors, and the version menu shows which ones a page saved.
- **The holding page is now the security boundary for these calls.**
  - The frame still can't reach the daemon or the server.
  - A hostile page can still write its own artifact's data as the viewer, within that artifact's rules, and can ask for a download or a model call, which the viewer must allow.
- **`sample` costs the viewer,** on their own machine. In the hosted app it needs one of their devices online.
- **Agents in a terminal still don't make artifacts.** Pages they inherited work, though.
- **`artifact-design` gains a section on capabilities,** and Silt ships `silt-artifact.d.ts`.
