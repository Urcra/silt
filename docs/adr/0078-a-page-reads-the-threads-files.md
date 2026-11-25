# 0078: A page reads the thread's files

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0072](0072-what-an-artifacts-page-can-do.md) (the capabilities)

## Context
Some pages are views of the code they came from: a coverage map, a browser for a folder of docs, a page that shows a config file's settings, or a benchmark report that reads the results file a run left in the thread. Today the agent has to copy the file's contents into the page, and the page goes stale as soon as the file changes.

In Silt a thread's files are already in its log. They sync with the thread, unredacted, as files follow git (ADR 0019), and both serving modes can read them: the daemon answers `/api/threads/{id}/files/at`. So a page can read the files without touching any checkout, and the server can serve them too.

## Decision
**`files`: the thread's files, read from its log, as the viewer.**
- **Declaring:** `capabilities: {files: {paths?: ["docs/**", "target/criterion/**"]}}`. `paths` are globs, and without them the whole tree is readable.
- **The functions:**
  - `files.list({path, recursive, cursor})` gives entries with their size and the delta that last changed each;
  - `files.read(path, {offset, length, at})` gives `{bytes, size, revision}`. At most 4 MiB per call, at the thread's head by default, or `at` a delta;
  - `files.history(path)` gives the turns that changed it, as `/files/history` does;
  - `files.watch(path, handler)` calls the handler when a new revision arrives.
- **What's read:** the thread's tree in its log, never a checkout. A page so reads the same bytes on every machine and on the server, and nothing a checkout holds outside the thread:
  - ignored files have left the thread (ADR 0060);
  - folders a workspace denies are refused, as for `silt_artifact`'s `path` (ADR 0065);
  - so is anything under Silt's data dir.
- **Who may:** anyone who can read the thread, with an account. Read-only links get `null`, since a link opens a conversation, not a tree.
- **The server:** it gains `/app/api/threads/{id}/files/at`, `/files/list` and `/files/history`, so the hosted app's Changes view can use them too.
  - **A new projection is needed.** The server has never projected file text: it builds no `Family` and has no git texts for files to start from.
  - **What the projection uses:** a family projection of the deltas it holds, with base texts read from its git mirror of the family (ADR 0042).
  - **Bases the mirror doesn't have:** a file whose base isn't there yet answers `not_found` until the mirror fetches it.
- **Downloads:** `files.download(path)` goes through `downloads` (ADR 0072). Executable types are refused.
- **`sample`'s tools:** `files.asSampleTools()` gives `list` and `read` as tools for `sample`, so a question asked from a page can read the code.

**What the page may do with what it reads.**
- A page can write what it read into its own data, which that data's readers then see (ADR 0072's rules). Some of them may not see the tree. The `artifact-design` skill says not to copy file contents into page data unless the person asks.
- Files are peer content, so pages insert them with `textContent`.

## Consequences
- **Pages that show code, results or docs stay current** with the thread, on every machine and in the hosted app.
- **No checkout is ever exposed to a page.** What a page reads is what a reader of the thread already could read.
- **The server serves file reads it didn't serve before,** from data it already holds. The hosted app's views can use them too.
- **The bridge's message limit means large files are read in pieces.**
