---
name: artifact-capabilities
user-invocable: false
description: What an HTML artifact's page can do beyond drawing itself, through silt.use - shared data, saving itself, who's viewing, downloads, comments, asking an agent, files people add, the thread's files, voice, connectors and live rooms - with the rules for each. Load it before writing any page that declares capabilities on silt_artifact or calls silt.use.
---

# Capabilities in a page

A page in Silt runs sealed: no network, no Silt address, no token. What it can do beyond its own scripts it asks for through `window.silt`, and the page showing it answers each call after checking it. `silt-artifact.d.ts` beside this skill types every function, with its limits and refusal codes. Read the parts you use before you write the code; they win over anything you remember.

Load `artifact-design` too. It covers the page itself; this skill covers what the page can reach.

## Declaring and asking

**Declare per version.** Pass `capabilities` to `silt_artifact` as `{name: config}`, such as `{"db": {}, "user": {}}`. `true` reads as `{}`.
- Leave `capabilities` out of a new version to keep the last version's.
- `{}` clears them.
- An object replaces the whole set, so restate every one you keep.
- The names are `artifact`, `assets`, `comments`, `db`, `dictation`, `downloads`, `files`, `mcp`, `room`, `sample`, `tts` and `user`. `permissions` is built in: call it, never declare it.
- Declare only what the page calls. Every name you add is something a viewer may be asked about.

**Ask with `silt.use`.** `const db = await silt.use("db")` resolves to the capability's functions, or to `null`.
- `null` means one of four things the page can't tell apart: not declared, not served in this view, failed, or nothing answered within 10 s. Treat them all as "absent".
- Render the whole page first without the capability, then light the feature up when the promise resolves. Never hold the first paint for it, and never show an error for `null`: hide the control, or show the read-only view.
- `use` never asks the viewer anything. Consent prompts, rate limits and refusals arrive on the first call that needs them, so put that call behind the viewer's own click wherever it can ask.
- Calling `use` again for the same name is free; it's the same promise. Keep the object it gives you. It's frozen and Silt's: call its functions, never assign to it or patch it. Wrap it in your own helper if you want another shape.

**Where it's served.**
- The web UI on a person's machine and the hosted app serve every capability, with the exceptions noted below (`dictation` isn't served in the hosted app, and `sample`'s tools aren't either).
- In the hosted app, `sample`, `files` and `mcp` run on one of the viewer's own computers through their server. They fail when none is online, and `sample` and `mcp` also need the computer's Devices settings to allow them.
- A read-only link and the offline copy resolve every `use` to `null`. So does `silt_artifact_check`'s headless browser. A page written for absence opens everywhere.

**Refusals.** Every rejection is an `Error` with a `code` and a `message` fit to show:
- `not_granted`: not declared, not allowed for this viewer's level, or the viewer said no. Degrade that feature for the rest of the load; don't ask again.
- `rate_limited`: over the capability's rate. Wait; never retry in a loop.
- `conflict`: someone saved first. Only `artifact.publish` has it.
- `too_large`: the call or its answer is over its size. A call and its answer are each at most 1 MiB, except an upload (20 MiB) and a download (16 MiB).
- `refused`: the call was malformed or what it asked was refused. The message says why.
- `unavailable`: this view can't do it now.

Catch every call. A rejection you don't catch leaves a control that does nothing.

**Contract versions.** `silt.version` names the runtime contract a version was made for, kept from one version to the next. Leave `contract` off `silt_artifact` so a revision keeps its page's; pass `"latest"` only when you move a page to the newest contract on purpose and have checked it against this skill. Only `1` exists today.

**Consent.** Acting on the viewer's behalf beyond the thread asks them, in the page holding the frame, never in a browser dialog.
- `sample`, `dictation`, `mcp`, `files`, and `comments` in its full form ask once per artifact and keep the answer, on the viewer's machine or server, for what the version declares. A later version that declares more asks again, and so does any later version of the page for `sample`, `mcp` and `files`, which run its own code with what's the viewer's. A "no" holds for the page's load: asking again shows nothing until the page is opened anew.
- `downloads`, `files.download` and every `mcp` action that isn't a read ask each time.
- `db`, `artifact`, `user`, `assets`, `room` and `tts` ask nothing. The viewer's level decides what they may do.

**Levels.** What a viewer may do follows their level in the thread, lowest first: `read` (anyone who can read it), `comment` (anyone signed in whom the thread's visibility admits), `collaborator` (the owner and the thread's collaborators), `owner`. Rules in `db` and `room` name these. On a machine with no server there's one person, the owner.

## Which one for which job

| The page should | Use |
|---|---|
| keep records that people add to, or the agent seeds and reads | `db` |
| be the record itself, saved whole when someone changes it | `artifact` |
| know who's viewing, show names, decide what to offer | `user` |
| hand the viewer a file it made | `downloads` |
| let someone comment, or leave a comment from a control | `comments` |
| ask an agent to summarise, classify or draft | `sample` |
| take photos or files people add | `assets` |
| show the thread's own files: results, docs, code | `files` |
| take spoken words, or read text aloud | `dictation`, `tts` |
| show or act on the viewer's own tools | `mcp` |
| show who's here now, cursors, a shared timer | `room` |
| say what was allowed, or ask for it all at once | `permissions` |

A per-viewer convenience (a remembered tab, a draft) needs none: `localStorage` works, kept for that viewer and that artifact only.

## permissions (built in)

`const permissions = await silt.use("permissions")` reports what the viewer allowed this page, for the capabilities that ask once: `sample`, `dictation`, `mcp`, and full `comments`; one that never asks, such as `db`, reads `granted` wherever this view serves it.
- `state()` gives every declared one; `state(name)` gives one. States are `granted`, `prompt`, `denied` and `unavailable`. Neither asks.
- Prompts are lazy by default: each capability asks at its first use. A page that needs several up front may call `request()` once at startup, which asks for every one still at `prompt` in a single prompt. `request(names)` asks for those only.
- `request` resolves with the states after asking. A "no" isn't an error: branch on the states and hide or disable what was refused. Never call it in a loop, and never put a "try again" button on a refusal.
- `manage()` asks the viewer whether to forget every answer for this artifact. Offer it as a button ("Reset permissions"), never on load.
- `state` says nothing about `db`, `downloads` and the others: they answer `unavailable` there. Use `use()` and the calls themselves for those.

## db

Data that lives outside the page: shared rows, each person's own entries, records the agent seeds or reads back. It's kept in the thread's history, so it hands off between machines, forks with the thread, rewinds with it, and syncs to the server.

**Declare** `{"db": {}}`, usually with `"user": {}` beside it. Without rules, everyone who can read the thread reads, and anyone who may comment writes.

**Rules** set the levels per path: `{"db": {"rules": [{"path": "...", "read": "...", "write": "..."}]}}`.
- A rule covers its path and everything below it. For each path the rule with the most segments that covers it applies. A rule's `read` defaults to `read` and its `write` to `comment`.
- `{self}` in a path matches the writer's own id. "Everyone reads, each writes their own, the owner tidies up" is two rules: `{"path": "votes", "read": "read", "write": "owner"}` and `{"path": "votes/{self}", "write": "comment"}`.
- Keep `"write": "owner"` (or `collaborator`) on the collection by default. Sign-ups, polls and guest books need someone who can remove spam.
- `self` is a level nobody holds, the owner included. Put it on a collection only when the request says nobody else may change an entry: a sign-off or a consent that must stay the person's own. `{"path": "signoffs", "read": "read", "write": "self"}` with `{"path": "signoffs/{self}", "read": "read", "write": "comment"}`. The collection's `read` then says who sees the entries: `read` everyone, `collaborator` the owner and collaborators, `self` nobody else.
- `data/users/<id>/` is private to that person, from everyone, the owner and the agent included, whatever the rules say. Use it for each viewer's own notes or settings that should follow them between machines. `<id>` is `await user.id()`.
- Rules aren't encryption. A collaborator can publish a version with looser rules, then read what was written. Say so if the request depends on privacy.
- A path starting `_` is Silt's and refuses writes.

**Paths.** Segments alternate collection and document: `db.doc("tasks/t1")`, `db.collection("tasks")`, `db.collection("data/users/" + id + "/notes").doc(key)`. Each segment is 1 to 64 of `[A-Za-z0-9_-]`, 2 to 8 segments in all. A document is at most 64 KiB; an artifact holds at most 10,000 documents and 16 MiB.

**Reading.**
- `get()` reads once. `onSnapshot(fn)` calls `fn` now and on every change, and returns the function that stops it.
- Subscribe once per query, when the page starts. Never subscribe inside a render function: each call adds a watch, and a page keeps at most 32.
- A refused watch never calls back, so give the loading state a 10 s timer that you clear on the first data, and show "Couldn't load" when it fires. Don't reload the page.
- Queries read the documents directly in one collection: `where(field, op, value)` (`==`, `!=`, `<`, `<=`, `>`, `>=`, `in`, `contains`), one `orderBy`, and `limit` (1,000 at most). Keep answers well under 1 MiB, or they're refused.

**Writing.**
- `set` replaces a document; `update` merges its fields, nested objects too, a `null` removing one (`{score: {ben: 2}}` keeps `score.ann`; two people changing different keys of one top-level field at once keep the later write's whole field); `delete` removes it; `collection.add(value)` makes one with a fresh id. `db.batch([...])` makes up to 100 writes, all or none.
- To change part of a long text field (notes, a description, a list kept as text), `doc.strReplace(field, old, new)` sends only the change. `old` must occur exactly once in the field, or the call rejects and nothing is written: give more of the surrounding text. The agent does the same with `silt_artifact_data`'s `str_replace` (`path`, `field`, `old`, `new`).
- Write on a person's action, one write per change, and only when the value changed. Don't write on load, on a timer, or on every keystroke: debounce text fields and save on blur or pause.
- A write the rules refuse rejects `refused` with a message naming the path. Show the read-only view rather than the error. `user.can("data.write")` says beforehand whether the viewer may write at all.

**Seeding.** Never write records into the page's source. Publish the page, then seed it with `silt_artifact_data` (`op: "batch"`). Seed what the person gave you, or examples marked as examples for a tool only they will use. For a page others will fill, seed nothing they didn't ask for: no invented people, votes or entries.

**Empty first.** The first frame renders before the data answers, and on a fresh page there's none. Design that state: say what will appear and how to add the first entry.

**Untrusted.** Every value came from a person or a page that may be hostile. Show it with `textContent`, never `innerHTML`. Keep secrets out: everyone who can read the thread can read what the rules let them.

## artifact

The page is the record, and saves itself as the thread's next version: a checklist, a sign-up sheet, a board, a document edited in place.
- Declare `{"artifact": {}}`. `const artifact = await silt.use("artifact")`, then `await artifact.publish(html)`. Every open view, this one included, reloads to it.
- Nothing a viewer types, ticks or drags is kept unless the page publishes it. So keep the shared state as data inside the document (a `<script type="application/json">` block), render the page from it, and when an action completes, update the state, regenerate the whole document from it and publish. Never serialise the live DOM.
- `html` is a whole document, doctype first. Keep its head, its token block, its structure and its title as the agent wrote them, and change only the data. The bridge takes at most 1 MiB in one call, so keep a self-saving page small.
- Publish after a viewer acts, never on load. Batch quick edits into one save, after a short pause. At most 12 saves a minute.
- `conflict` is routine: someone saved first, and this view reloads to their version, dropping this edit. Don't retry.
- Only the thread's owner and collaborators save. Others get `not_granted`; check `user.canEdit()` first and render a read-only view for them.
- Each save is a version in the thread, marked as the viewer's. Data the agent seeds, reads back or keeps beyond what the page shows belongs in `db` instead.

## user

Who's viewing, and the names behind the ids the page keeps.
- Declare `{"user": {}}` for ids, or `{"user": {"scopes": ["profile"]}}` when the page shows names or uses `search`: without the `profile` scope every name is `""`. Silt keeps no email addresses, so an `email` scope adds nothing. `id()` is 16 hex characters, the same for one person on every machine and the server, and shows no name; with no server it's `local`. `me()` gives `{id, name}`.
- `isOwner()`, `canEdit()` (the owner or a collaborator) and `can("data.write")` decide what to offer. `can` answers `null` for a name it doesn't know: keep the control, and let the write's own refusal decide.
- Store ids, never names: names change and read differently per viewer. Resolve them each time you render, `const people = await user.profiles(ids)`, then `people[id].name || "Someone"`. Use `||`: an unknown name is `""`. It's cheap to call again.
- `search(query)` finds up to 20 of the thread's people by the start of a word in their name, everyone when it's empty. Call `search("")` when a picker opens. Silt knows the names of the thread's owner, its collaborators and the viewer, no one else.
- Set names with `textContent`.

## downloads

A file the page made: a CSV export, a filled template, an image.
- Declare `{"downloads": {}}`. `await downloads.save({filename, data, type})`, with text, bytes or a Blob, at most 16 MiB.
- The viewer confirms every save and may decline (`not_granted`). Offer it on their click, and treat a refusal as quiet: no error banner.
- Runnable types (`.exe`, `.sh`, `.js`, `.bat` and the like) are refused, and the name is cleaned.
- A plain `<a download>` does nothing in the frame. This is the only way to hand over a file.

## comments

An entry point to Silt's own comments. A comment is a thread on the artifact, shown by Silt, which the agent hears like any other.
- `{"comments": {"composer_only": true}}` serves `openComposer({text})`: it opens Silt's composer on this version, quoting `text` (500 characters at most), and asks nothing. Prefer it for a "Comment on this" button beside a row or a figure.
- **On an element:** `openComposer({element})` puts the comment on that element of the page, quoting its text. Silt keeps the element's anchor, `anchorFor(element)`: its `data-comment-anchor`, or an ancestor's, else its `#id`, else a CSS path, which breaks when the page's structure changes. Give what people comment on a `data-comment-anchor` naming it (`data-comment-anchor="total-row"`) so comments follow it to later versions. When the viewer asks to see such a comment, Silt scrolls to the element and outlines it.
- **Drawn pages:** a page that draws on a canvas, or anything without elements to point at, passes its own names as `anchor` (`openComposer({anchor: "point:Q3", text: "Q3 revenue"})`) and calls `customAnchors(({anchor}) => …)` at load to show the commented thing itself when asked.
- `{"comments": {}}` adds `add({text, quote, element, anchor})`, `reply({id, text})` and `resolve({id})`, written as the viewer once they agree for this artifact. Use it when the page's own control should leave a comment, such as "Flag this row", which then reaches the agent as a request.
- Call the writing verbs only from a viewer's deliberate action, never on load. `reply` and `resolve` take only the ids of comments on this artifact.
- Never build a list of comments in the page. Silt shows them.

## sample

A question for an agent: summarise what the page holds, classify a row, draft a reply, turn a sentence into structured data.
- Declare `{"sample": {}}`, or `{"sample": {"images": true}}` to show it pictures. `const sample = await silt.use("sample")`, then `const {text, truncated} = await sample(input, opts)`. `sample.json(input)` parses a JSON answer, a code fence allowed; it throws a plain `SyntaxError` when the answer isn't JSON, so catch that too.
- `input` is text, or a list of `{role, content}` turns ending on the user's. At most 256 KiB.
- It has no memory and no session, and no tools but those the page passes. Each call carries the instructions, the data and the format you want back. Ask for JSON with the fields named when the page will read the answer.
- The viewer's own machine answers, with their agent, at their cost. The first call asks the viewer, once per artifact. Call it from a click, or once from a stable load step the page clearly announces. Never call it in a loop or on every change: at most 4 questions a minute per artifact (`rate_limited`).
- `onText({text, delta})` streams the answer: `text` is the whole answer so far, so assign it rather than appending. Show "Thinking…" until the first words.
- `images`: up to `(await sample.limits()).images` Blobs (4), PNG, JPEG, GIF or WebP.
- `tools`: up to 16 of the page's own functions the agent may call while it answers, each `{name, description, inputSchema, execute}`. `execute` runs in the page with the agent's input; return small data, or throw with a message the agent can act on. Offer only what the question needs, and make them read the page's state rather than change it unless the request says otherwise. In the hosted app one of the person's computers answers, and its tool calls come back to the page through the server.
- `modelTier`: `quick` for a short, cheap answer (a label, a yes or no), `complex` for a hard one, `default` otherwise. Name a `model` (such as `claude-sonnet-5-5`) only when the person asked for that model; it wins over the tier. The answer's `modelApplied` says what was asked for.
- `cache`: the same question, with no tools or images, asked again within five minutes gets the answer it had without asking; pass `cache: false` for a chat, whose turns must each be answered. `signal`: an `AbortSignal`, a new `AbortController` per call; aborting rejects the call with `aborted`, stops `onText` and ends the agent's run on the viewer's computer, for a Stop button or a question the person has moved past.
- One computer answers at most three pages' questions at once; a fourth waits.
- Hide the feature on `not_granted`. Back off on `rate_limited`, and say when it can be asked again. On `truncated`, show what came and say it was cut off.
- What comes back is the model's text: show it with `textContent`, and check a JSON answer's shape before you use it.

## assets

Files people add to the page: photos on a board, a PDF on a record, a logo for a mockup.
- Declare `{"assets": {}}`: the owner and collaborators add files. Only when the request wants others to add them, declare `{"assets": {"upload": "comment"}}`, which lets anyone who may comment add one. Only the owner and collaborators delete.
- `upload(file)` takes a Blob or a File, at most 20 MiB, and resolves `{id, url, sizeBytes, contentType}`. Images, video, audio, PDFs, fonts, CSV, Markdown, plain text and JSON; never a page or a script.
- Keep the `id` in a `db` document as the lasting pointer. Use `url` (`_assets/<id>`, relative to the page) as an `<img>`, `<video>`, `<audio>` or `<a>` source. An SVG someone adds is kept without scripts, handlers or outside links. Show it with `<img>` all the same, never inline.
- `list()` gives every file and the bytes they use. `delete(id)` removes one for good: only on a deliberate action, then update the documents that pointed at it.
- Hide the upload control when `use` resolves `null`, and catch `not_granted` for viewers below the level.

## files

The thread's own files, read from its history, the same on every machine: benchmark results, a report a script wrote, docs, a config.
- Declare `{"files": {"paths": ["bench/*.json", "docs/**"]}}` with globs for what the page needs (`*` within a segment, `**` across them). Without `paths` the page may read the whole tree; name them anyway. Folders the workspace denies are refused whatever you declare.
- `list({path, recursive, cursor})` pages through 500 entries at a time. `read(path)` gives `{text, size, revision}` for a text file, at the thread's head, or `{at: revision}` for an earlier one. Keep what you read under 1 MiB, the bridge's limit.
- The first `list` or `read` asks the viewer, once per artifact, so make it from a click or a clearly announced load step. `download(path)` asks the viewer to save the file, each time.
- To keep a file on screen current, `watch(path, fn, {every})` reads it every `every` ms (2,000 at least, 5,000 by default) and calls `fn` when its revision changes. Start one watch per file the page shows, and stop it when the file leaves the screen. A failed read is skipped silently, so show when the file was last read. Don't build your own polling loop.
- `asSampleTools()` gives `list_files` and `read_file` as tools for `sample`, so the agent answering a page's question can look through the thread's files itself rather than the page pasting them into the question. Declare `sample` too; tools only run on this computer's agent, not in the hosted app.
- In the hosted app a read goes through one of the person's computers, so it fails when none is online. Show what you last read, and say it couldn't refresh.
- Don't copy a file's text into `db`, assets or a version unless the person asks: anyone who can read the thread can read those.
- Show what you read with `textContent`, or parse it; never insert it as HTML.

## dictation

The viewer's speech as text, turned into words on their own machine.
- Declare `{"dictation": {}}`. It's `null` in the hosted app and wherever the browser has no microphone access: hide the microphone and keep typing.
- Call `start()` from the viewer's click, and catch it. The first start asks the viewer. `not_granted` means they said no: keep the text box, and don't offer the microphone again this load. `unavailable` means the speech model isn't downloaded yet or the microphone failed: say so in a line. `refused` means a take is already running.
- `onPartial(({text}) => …)` is a live guess that replaces the last; show it greyed. `onFinal(({text}) => …)` is a settled phrase: append it. `onState(({state, reason}) => …)` drives the button, `listening` until `idle` with `stopped` or `failed`.
- `await stop()` waits for the last words. A take also stops when the tab is hidden.
- Prefer it to anything else when a page only needs words. Keep a text box beside it always.

## tts

Text read aloud in the browser's own voices, which differ between machines.
- Declare `{"tts": {}}`. It's `null` where the browser can't speak: show the text and hide the speaker control.
- `await speak(text)` reads up to 2,000 characters and resolves when it's done; catch it. A new `speak` replaces the one under way, whose promise rejects. `add(text)` queues after it.
- Several speakers or a long text: pass lines, `speak([{text, voice, id}, …])`, one paragraph or one speaker's turn each. `onLine(({line, id}) => …)` fires as each starts, to highlight it, and `onPosition(({ms, line}) => …)` says how far into it the reading is, for a progress mark.
- Voices are `buttery` (the default), `airy`, `glassy` and `rounded`, each mapped to one of the browser's voices. `rate` is 0.7 to 1.5. `language`, such as `pt-BR`, picks a voice for it: give it for text you wrote, not for text a viewer typed.
- `onState` drives the control: `speaking`, `paused`, or `idle` with `completed` or `viewer_stopped`. `pause()`, `resume()` and `stop()` do what they say.
- While the viewer dictates, `speak` rejects `busy`; once they stop a reading, `viewer_stopped`. Speak only what the viewer asked to hear, and keep the words on screen.

## mcp

The viewer's own tools: MCP servers they set up on their machine under Settings › Appearance › Connectors for pages, each tool off until they tick it.
- Declare each server and only the tools the page calls: `{"mcp": {"servers": [{"server": "tracker", "tools": ["list_issues", "close_issue"]}]}}`. `server` is the connector's name as the viewer gave it. A call reaches a tool only if the version declares it and the viewer ticked it.
- Most viewers won't have that connector. Design the page to work without it, and light the feature up after the first call succeeds. `listTools(server)` lists what the version declares, not what's ticked.
- **Showing data:** `watchTool(server, tool, input, handler, {refetchInterval})` reads a tool the server marks as only reading, now and then every interval (at least 30 s), or once without one. What the viewer's app read last for the same call comes first, with `cache.stale` when it's more than 30 s old, and a fresh read follows; show when the data came back (`cache.storedAt`), and mark stale data as such until the fresh read lands.
- **Acting:** `callTool(server, tool, input)` once, from a click. Anything not marked as only reading asks the viewer each time, showing its input. A decline rejects `not_granted`: leave things as they were.
- A tool's own failure comes back as `isError: true` with its content; a failed call rejects. `unavailable` means the connector isn't set up or doesn't start here. `not_granted` means not declared, not ticked, or the viewer said no: show the feature as unavailable rather than retrying.
- Don't guess a tool's arguments. Read the schema in your own session if you have that server, or have the page call `describeTool(server, tool)` and build the form from `input_schema`. If you can't know the shape, say so when you publish.
- Results are the viewer's data and untrusted. Show them with `textContent`, keep them out of `db` unless asked, and never embed values you saw while writing the page as sample data.

## room

Whoever has the page open right now: other people on any machine or in the hosted app, and the thread's agent when it speaks through `silt_artifact_room`.
- Declare `{"room": {}}`. `const room = await silt.use("room")`, `null` when it can't connect.
- `emit(topic, data)` sends a moment to the others; `on(topic, (data, from) => …)` hears theirs. A view never hears its own moments, so apply your own change locally when you send it.
- `presence(patch)` merges into this view's state (a cursor, a selection, a colour) and sends it whole. `onPeers(peers => …)` gives everyone here, this view marked `you: true`, on every change. Render them all. Presence lapses for the others 15 s after a view last sent anything, so send it again while it matters.
- Nothing persists and moments can drop. Send absolute state ("slide 4"), never deltas ("next"). What a late joiner needs (the current slide, the board) is a `db` document; lasting content is a new version.
- Topics need a collaborator by default. Open one to commenters only when the request wants it: `{"room": {"topics": {"cursor": "comment"}}}`. Don't open topics to `read`.
- A refused moment is dropped, so keep moments small (16 KiB at most) and under 30 a second. `onRefused(({code}) => …)` hears why (`not_granted` for a topic this viewer's level may not send, `too_large`, `rate_limited`), so the page can say so rather than look stuck.
- Everything heard is untrusted, and presence is never authority: anyone can claim anything in it. Decide who may do what from `user` and `db` rules.
- `join(name)` gives a sub-room heard only by those who join it, for a game or a breakout. Names aren't secret. The room you start in is the lobby.
- A board people arrange together is a `whiteboard` artifact, which has its own room. Use `room` for live touches on a page.

## Where a page keeps its state

- **A per-viewer convenience** (a remembered tab, a filter, a collapsed section, an unsent draft): `localStorage`, kept for that viewer and artifact only, at most 1 MiB. Wrap reads and writes in `try`, and render correctly without them.
- **The page is the record** (a checklist, a sign-up sheet): `artifact`. Collaborators republish the whole page from its state; a concurrent save loses with `conflict`; other viewers read.
- **Data outside the page** (the agent seeds or reads it, it's more than the page shows, it's private per viewer, or many people write at once): `db`, under rules, live through `onSnapshot`, kept across versions.
- **State that must survive a new version arriving while the page is open** (a game, a long form): `silt.hot.snapshot(() => state)`, and boot through `silt.hot.ready(start)`. The holding page takes a snapshot every 2 s, at most 1 MiB, and hands the last one to the next version.

## Check before you hand over

A page that declares capabilities gets two looks, and no more.
1. **The render.** `silt_artifact_check` after the first version, as `artifact-design` says. Its headless browser serves no capabilities, so it shows the page as a viewer with none sees it. That's the state you designed for first: it should read as complete, with an empty state and no dead controls.
2. **The data, for `db`.** One `silt_artifact_data` `list` of each collection the page writes, to see that what you seeded is there and shaped as the page reads it. Where the rules hide something from some viewers (`self` entries, a collaborator-only collection, `data/users/`), make the same read with `as_level` set to the lower level, `comment` or `read`. It reads as someone else at that level and must not show what the rules hide. A write with `as_level` is refused.

Then tell the person in one line what you checked and what you couldn't: a consent prompt, dictation, a connector they haven't set up, or a room with two people can't be tried from here. Don't loop on checks. They'll try the page live, and their comments and their browser's problems come back to you.
