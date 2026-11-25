# 0060: Files git comes to ignore leave the thread, and a large diff is read a file at a time

- Status: accepted
- Date: 2026-11-10
- Amends: [ADR 0019](0019-redact-to-gits-threat-model.md) (`.gitignore` is the boundary)

## Context
ADR 0019 says Silt takes in only what `git ls-files --cached --others --exclude-standard` lists. Capture held to that for new files, but never let go of a file once it was recorded. In a repository without commits and without a `.gitignore` yet, a conda environment and a `third_party/` checkout were created and recorded as untracked files: about 40,000 files and 12 GB. `.gitignore` named both half an hour later. Capture then left them out of what it looked at, but kept them in the thread for good.

That thread could no longer be opened in the web UI. Its diff was 448 MB. The page fetched all of it to count lines, parsed it and listed 40,000 files, and each turn's bar fetched its own turn's diff the same way. The daemon sat at 7 GB and kept rescanning the checkout.

## Decision
**A file that git ignores and doesn't track leaves the thread, and stays on disk.**
- When capture looks at a recorded file and git ignores it (`git check-ignore`, paths in git's index excluded), the checkout records the file as gone from its thread. The file on disk is left alone.
- Only files this checkout brought into the thread leave it. A file another machine or checkout brought in stays, so that what one machine's own excludes (`.git/info/exclude`, a global ignore file) leave out is never deleted for everyone.
- A changed `.gitignore` sets off a rescan of the whole checkout, as only such a rescan asks git about every file the thread has.
- Ignored files git tracks stay in the thread, as they do in git.
- Other checkouts of the thread see a delete, as with any other: their copies are removed unless they changed there.

**A diff too large to read whole is read a file at a time.**
- `GET /api/threads/{id}/diff/stats` (`Request::DiffStats`) answers a diff's files with their counts and sizes (`silt_proto::view::DiffStat`), without their lines. `GET /api/threads/{id}/diff?path=` answers one file.
- The web UI counts a thread's and a turn's changes from the stats. The changes panel and a turn's changes read the whole diff only up to 300 files and 2 MB; past either, they list the files and read each when it's opened.
- The hosted app has no diffs, as before: they need a checkout.

## Consequences
- A thread that took in a dependency directory before `.gitignore` named it recovers on its own: the checkout's next rescan takes the files out, and the diff shrinks to the real changes. History keeps the files, as it keeps any deleted file, so the store doesn't shrink.
- A file brought in by someone else that this machine ignores stays in the thread; whoever brought it in takes it out by ignoring it there.
- Counting a large diff's lines still renders it on the daemon, which takes seconds for hundreds of megabytes, but nothing that size reaches the browser.
