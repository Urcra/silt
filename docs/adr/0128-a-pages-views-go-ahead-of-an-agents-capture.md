# 0128: A page's views go ahead of an agent's capture

- Status: accepted
- Date: 2026-11-26
- Amends: [ADR 0127](0127-switching-threads-waits-for-nothing-else.md) (what else holds the core), [ADR 0018](0018-daemon-optional-capture.md) (where the daemon's capture lists a checkout)

## Context
ADR 0127 took the reading of long histories off the daemon's core. What an agent does still ran there, and every request of the web UI waited behind it. Measured in the perf build on a 4-core Linux machine:

- **Recording new files was quadratic.** Each file recorded listed every path of its thread to find its own: 35 ms a file in a checkout of 50,000. One Bash call writing 500 files of 100 KB and two of 50 MB, as an asset import does, held the core for 20.4 s, 17.6 s of it listing paths. One writing 2,000 such files held it for 90 s.
- **Every Bash call compares the whole checkout.** Listing 50,000 files held the core for 30 to 90 ms. On NTFS, or a Windows drive reached from WSL, each `stat` costs many times that.
- **Reading a file or writing its blob holds the core** for as long as the file takes: about 100 ms for 50 MB.

Opening a long thread for the first time also read every transcript line twice: once into entries, and once whole into a JSON value only to take its `timestamp`. That second read was 350 ms of the 1 s it took for the 100 MB session.

## Decision
**A request that only reads what a page shows goes ahead.** `Request::is_view` names them: the thread listing, a history's pages and turns, diffs, commits, comments, the queue, status and the like. The web UI's and the socket's views go to a queue of their own, which the core takes first, and the core answers those waiting between the steps of long work: after each file it records, and while a checkout is listed. Each step leaves the family as it is between events, so a view sees what's recorded so far. A sandboxed run's requests get no place ahead.

**A checkout is listed on a blocking thread,** with a `Repo` of its own that takes the walks the checkout's keeps and hands them back (`Repo::take_walks`, `give_walks`).

**A bridge keeps its thread's paths** between files, with the view they were listed in (`Family::view_key`). It patches in what it added or removed itself, and lists them again when the tree changed otherwise, or a removed name was shared with another file.

**A line's stamp is read without reading the line into values,** and a family's chunks are decoded on up to 8 threads once they pass 4 MB.

## Consequences
- With those 502 files, the capture held the core for 1.2 s instead of 20.4 s, and the thread listing, asked every 10 ms meanwhile, waited at most 129 ms, reading one 50 MB file. During a rescan of 50,000 files it waited at most 52 ms.
- Opening the 100 MB session's thread the first time takes about 0.5 s instead of 1 s.
- A view can be answered before a write sent just before it, from the web UI or another process. The write's event brings the page up to date.
- Reading a file and writing its blob still run on the core, one file at a time between views, as does writing to the store. A file of several GB holds pages that long.
