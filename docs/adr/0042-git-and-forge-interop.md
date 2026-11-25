# 0042: Git notes, the forge bridge and the Agent SDK session store

- Status: accepted
- Date: 2026-10-22
- Builds on: [ADR 0030](0030-commits-and-turns.md) (commits and turns), [ADR 0031](0031-permalinks.md) (permalinks), [ADR 0023](0023-projects.md) (projects)

## Context
A commit made in a thread's checkout is linked to its turn inside Silt (ADR 0030), but nothing outside Silt sees the link. ADR 0030 keeps trailers out of commit messages, so the link has to sit next to the commit. Three places matter:
- **git itself:** `git log`, `git blame` and whatever reads them;
- **the forge:** the pull request a thread's branch is reviewed in;
- **apps built on the Claude Agent SDK,** whose sessions would otherwise live only on the host that ran them. The SDK takes a `SessionStore` adapter (`append`, `load`, and optionally `listSessions`, `delete` and `listSubkeys`) keyed by a project key, a session id and a subpath for subagent transcripts.

## Decision
### Settings
Two settings on the server's project record, beside its default agent (ADR 0023), set with `silt project set --git-notes … --forge-comments …` or from Add a project's defaults:
- **`git_notes`:** `off` (the default), `local` (write notes in each clone) or `push` (write them and push `refs/notes/silt` to origin);
- **`forge_comments`:** off by default.

A project no server knows, or a machine with no server, reads `git config silt.notes` in the clone instead, with the same three values.

### Git notes
**Where.** `refs/notes/silt`, one note per commit a thread recorded, written by the machine that recorded the commit (`git notes --ref=silt add -f`). With `push`, the daemon then pushes the notes ref to origin, off the event loop. A push that fails is logged and retried with the next commit; git's own credentials are used.

**The note** is text, readable in `git log --notes=silt` and parsed by `silt blame`. Its first line names the format, so it can change:
```
silt-note 1
thread 0f3c… https://silt.example/app/t/0f3c…
title Fix the login redirect
turn 3 Fix the login redirect when the session expired
file src/login.rs
10-24 turn 3 agent claude-code
30 turn 4 person
31-33 thread 77a1… turn 2 agent codex
```
- **`thread`:** the thread's id, then its permalink (ADR 0031) when it has a server.
- **`turn`:** each turn that wrote a line of the commit, with its prompt's first line, clipped. A turn's link is the thread's link with `#turn-N`.
- **`file` and ranges:** the lines the commit added or changed, numbered as in the commit, grouped into runs with the same author. A run names its thread when another thread of the family wrote it, such as the fork a merge brought in. Lines that came from git, or that Silt can't trace, are left out.
- **Unknown keys are skipped,** so later versions can add some. A path holding a control character is left out rather than escaped.

**Line attribution** comes from the text CRDT. Each character carries the delta that inserted it. A line belongs to the latest insertion among its characters, and that delta gives the thread, the turn (counted as ADR 0030 counts them) and the author. The text is the thread's as of the commit's `Committed` delta. When it differs from the committed file, as with uncommitted edits or a partial commit, lines are matched by a line diff, and the rest are left out.

### `silt blame <file>`
- **From the log:** inside a checkout of a thread on this machine, each line of the file as the thread has it now, with its thread, turn, prompt and author.
- **From the notes:** anywhere else, and for lines the log has from git, it runs `git blame --porcelain`. It then finds each line's commit, and the line's number in that commit, in the notes. That works in any clone that fetched `refs/notes/silt`, with no daemon and no server.
- **`--notes`** uses the notes only.

### The forge bridge
**The server posts.** It sees every thread, fork and merge, so it can keep one comment current from any machine. A daemon would update it only while its machine is on.
- **Tokens:** `server.toml` gets `[[forges]]` entries, each with a `host`, a `kind` (`github`, `gitlab` or `forgejo`), an optional `api_url` and a `token` or `token_file`.
- **Which forge:** a project's repository (`github.com/acme/web`) goes to the entry for its host.

**When.** A push that brings a thread a commit, a title, a fork, a merge or a merge request marks the thread. A worker on the server then looks the thread up a few seconds later, so a burst of deltas makes one update.
1. It finds the thread's branch, from its last `Committed` or `Genesis` op.
2. It asks the forge for an open pull request from that branch.
3. It posts or updates one comment on it.

**The comment** names the thread with its link, its summary (its title, turns and commits, and the open merge request's summary if there is one), and its forks and merges, each with its state. It starts with a hidden marker, `<!-- silt:thread:<id> -->`. The server finds its comment again by the marker, so it stores nothing and edits rather than posting twice.

**What it shows.** A pull request is often public, so a private thread (ADR 0013) is never commented on, and private forks are left out of the list.

**The API.** Building requests and reading answers is pure, one module per forge. The worker does the I/O. A failure is logged and tried again on the thread's next change.

### The Agent SDK session store
**A Silt server is a `SessionStore`.** `packaging/agent-sdk/` holds an adapter for TypeScript and one for Python. Each is a thin client of these routes, signed in with a device token:

| Route | Does |
|---|---|
| `POST /v1/session-store/append` | `{key, entries}`: records the entries |
| `POST /v1/session-store/load` | `{key}`: the entries in the order appended, or `null` |
| `GET /v1/session-store/sessions?project_key=…` | `[{sessionId, mtime}]` |
| `POST /v1/session-store/subkeys` | the subpaths of a session |
| `POST /v1/session-store/delete` | moves the session's thread to the trash; for a subpath, hides that transcript's entries so far |

**Each session is a thread.** The first `append` for a (person, project key, session id) makes a family and a thread owned by the person. The thread is private until they share it, and its repository is the one the request names, if any.
- **Entries:** each one becomes a `ConvOp::Native` line, in the main file or, under a subpath, the subagent file it names. The server writes them as the person's own server actor (ADR 0026), through the same checks as a device's push.
- **What that gives:** another host resumes the session through the SDK, and the person's machines list, search and read it like a recorded one. `silt resume` needs the thread's base commit and files, which the server can't import, so it refuses such a thread for now.
- **Retries:** the SDK may send an entry twice, so entries with a `uuid` already recorded are skipped.
- **Redaction:** entries are redacted as a device redacts its pushes (ADR 0019). `load` gives back what was stored, so an entry holding a secret comes back with it masked.
- **Subpaths:** only `subagents/<name>` is kept, as the subagent transcript Claude Code would write. The log only grows, so deleting one hides its entries so far rather than removing them.
- **Checked:** the Python adapter passes the SDK's own conformance suite (`claude_agent_sdk.testing`) against a running server.
- **Not yet:** `listSessionSummaries` isn't offered; the SDK falls back to `listSessions` and `load`.

## Consequences
- Notes are opt-in twice over: no project writes them until the setting says so, and none leave the machine until it says `push`. A clone fetches them with `git fetch origin refs/notes/silt:refs/notes/silt`, and the guide says how to fetch them by default.
- A note shows a thread's id, title and prompts to whoever can read the repository, which is why pushing them is a separate choice.
- The server holds forge tokens. They are read like the sign-in providers' secrets, and never shown.
- Pull request comments are best-effort: a forge that is down or a token without rights only logs.
- SDK sessions recorded through the store have no checkout, so they carry no file changes, only their conversation.
- The adapters are copied into an app rather than published as packages, like the SDK's own reference adapters.
