# Large working trees: plan

Status: research and a proposed design, not built. The decisions marked **Open** are the person's to make; once made, the content model gets an ADR. #311 is the bug that started it.

Silt should:
1. **Open a thread in a large folder that isn't a git repository yet**, near instantly.
2. **Open a thread in a very large git repository with many files out of sync with git** (modified, untracked), near instantly.
3. **Not need much extra disk** for either.

Today it can do none of the three. A 209 GB folder of game data (files of 0.5–76 GB, nothing ignored) never got its agent started: the daemon spent 15 minutes copying 69 GB into `blobs/`, used 28 GB of memory, answered nothing and couldn't be stopped (#311).

The idea this plan is built on came from the person: **record the thread's changes against the files as found, and pay the cost of their content only when it's needed elsewhere.** Silt already does that for files git tracks (`GitText(oid)` is a reference, read through `BaseText` when needed). The plan extends it to everything else. A second point, also the person's: **deltas only mean something for text.** Binaries are replaced whole, so for them Silt needs to record identity (old → new), not edits.

## What happens today

Traced through the code at `5bff6aee`. File references are to that commit.

### Opening a thread
- `create_thread` (`crates/silt-daemon/src/sessions.rs:342`) needs git: `Repo::discover(&cwd)?` fails outside a repository, and `Checkout.repo` isn't optional. Nothing records a folder without git (#32). A repository with no commits imports nothing, so every file in it is untracked: the most expensive case.
- It imports `HEAD`'s whole tree as one `Create` delta per file and per directory, `GitText(oid)` for files (`sessions.rs:372-404`, `crates/silt-checkout/src/import.rs:71`). That's per thread: each new thread is a new family.
- It then starts the checkout with **empty entries** (`sessions.rs:414`) and rescans it (`:416`). The size-and-mtime shortcut (`maybe_changed`, `core.rs:1955`) needs a recorded entry, so **every file is read and hashed, for every new thread**, tracked files included. Git's index already has a stat and an oid for each tracked file; nothing uses them (`silt-git/src/lib.rs:364-417` reads only the paths from it).
- **The agent starts only after that scan.** `StartSession`'s reply waits on it (the CLI waits up to 60 s), and so does the ACP start.

### Each file
`ingest_file` (`core.rs:2159`) → `disk::read` (`disk.rs:98`) → `disk_file` (`core.rs:2211`) → `Bridge::ingest` (`silt-checkout/src/bridge.rs:169`) → `persist` + `persist_entry`.
- `disk::read` is a blocking `std::fs::read` of the whole file, then BLAKE3, **on the core**. One 76 GB file can't fit in memory; a 2 GB one holds every request for as long as it takes. ADR 0128 answers views between files, not during one, and an agent start isn't a view.
- `disk_file` puts every non-text file (binary, over `TEXT_LIMIT` = 5 MiB, or not UTF-8) into `blobs/` with `put`, which rehashes it, writes it and fsyncs twice. It never produces `DiskContent::GitBlob`.
- **So an unchanged tracked binary is recorded as changed.** The family holds `GitText(oid)`, the disk gives `Blob(hash)`, and `Writer::update` emits `SetContent { Blob }` (`bridge.rs:501-503`). The first scan of a repository full of assets copies every one into `blobs/`, uploads it, and every machine that pulls stores it again: git's copy, duplicated on each machine.
- Two SQLite transactions per file, empty ones included (#26).
- A file whose mtime moved but whose content didn't is re-read on every scan: the entry isn't refreshed when the hash matches (`core.rs:2178`).
- When anything vanished, `ingest_renames` reads every unrecorded file whole into one `Vec`, at once (`core.rs:2027`).

| File | Read | Stored | Extra copies |
|---|---|---|---|
| Tracked, unchanged text | whole, BLAKE3, `to_git`, git base twice | a `disk_files` row | 0 |
| Tracked, unchanged binary or text > 5 MiB | whole, hashed twice | `blobs/`, a `SetContent` delta, the server | +1 here, +1 on the server, +1 on each machine that pulls |
| Tracked, modified text | as unchanged, plus a diff | text deltas (zstd chunks), the doc in `files/` at the next snapshot | about 2 |
| Modified binary | whole | a new blob per version, never collected | +1 per version |
| Untracked text ≤ 5 MiB | whole | `Create` + one whole-content `Insert`, and `files/` | about 2, **again for each new thread in that folder** |
| Untracked text > 5 MiB, untracked binary | whole, any size | `blobs/`, the server | +1 here, +1 on the server |

### After capture
- **Push uploads a file's blob before its delta** (`referenced_blobs`, `sync.rs:749-766`; ADR 0121). A 100 GB folder holds back all of a family's deltas, the conversation included, until every byte is up. Only attachments over 8 MiB are deferred. `remote.put_blob` takes the blob as a `Vec<u8>`.
- **Pull is lazy**: `apply_remote` fetches no blobs; `Core::blob` fetches when something needs the bytes. But `materialize` (resume, fork) writes every file of the thread, downloading each blob whole into memory. The streaming, resumable `Remote::download_blob` has no callers.
- **A managed checkout copies again.** It's `git init --separate-git-dir` with alternates, then `git reset --hard` (the tracked tree again), then `materialize` writes every untracked file again from `blobs/` (`silt-git/src/checkout.rs:29-74`).
- **Blobs are never collected on the daemon** (only `erase` removes them), and on the server only when a thread is purged. Every version of every binary is kept forever.
- **Binary merges are whole-file, last writer wins** (`family.rs:1219-1229`), which is right. But concurrent changes to one binary aren't flagged as a conflict in a merge request (`forks.rs:549-564` compares texts; `text_of` is `""` for a blob), so one side is replaced silently.
- Revert, fork-from-turn and permalinks assume every old version can be fetched. Revert aborts partway on the first missing one (`revert.rs:34-75`).

### Watching
- One recursive `notify` watcher on the root, with no ignore filter (`watch.rs:89-97`). Under inotify that's a watch for **every directory, ignored ones and `.git` included**, added while the core waits (`core.rs:1850`). Past `max_user_watches` (as low as 8192 on small machines, shared with editors) the checkout falls back to a **whole rescan every tick**: 300 ms, 2 s when idle.
- On 9p, FUSE, SMB, CIFS and NFS the `PollWatcher` runs with `with_compare_contents(true)` (`watch.rs:89`): notify rereads every file under the root every 500 ms.
- The in-memory untracked-walk cache is cleared whenever the index's mtime changes (`silt-git/src/untracked.rs:149-157`), so any `git add`, commit or `git status` means a full walk.

## What others do

Sources are linked in full in the two research reports this note was written from; the key ones are here.

### Finding changes without reading files
- **A persisted stat cache, with the racy rule.** Git's index keeps mtime, ctime, inode, mode and size per file and reads content only when they differ. A file whose mtime isn't strictly before the index's own write time is "racily clean" and compared by content ([racy-git](https://git-scm.com/docs/racy-git)). jj keeps only type, mtime (ms) and size, with the same rule against its `tree_state` file ([local_working_copy.rs](https://github.com/jj-vcs/jj/blob/main/local-working-copy/src/local_working_copy.rs)). Syncthing, Perforce (`p4 reconcile -m`) and Plastic do the same. Git's index stores size as 32 bits, so files of 4 GiB or more alias.
- **Skip unchanged directories.** Git's untracked cache and Mercurial's dirstate-v2 store each directory's mtime, a "children fully recorded" flag and a hash of the ignore rules, and skip `readdir` for directories that didn't change. On Chromium the untracked cache alone halves the untracked phase. Scalar turns it off on Windows for filesystem bugs.
- **A monitor with a cursor that survives restarts.** Git's fsmonitor (on Linux too since Git 2.55) takes `git status` on 2M files from 85 s to 0.75 s ([GitHub](https://github.blog/engineering/infrastructure/improve-git-monorepo-performance-with-a-file-system-monitor/)). Only macOS's FSEvents keeps history for an unprivileged process (event id + volume UUID, `sinceWhen`). Windows' USN journal does too, but needs Administrators. inotify and fanotify (unprivileged) keep nothing, so on Linux a restart means a stat walk, made cheap by the two caches above. Every tool treats events as hints: on overflow or loss, rescan.
- **Ask git.** Zed doesn't compute git status itself; it runs `git status --porcelain -z` scoped to what changed, and so inherits the person's fsmonitor and untracked cache. gix 0.91's status is faster than git's on a warm cache but about 5× slower cold on the Linux kernel ([gitoxide#1771](https://github.com/GitoxideLabs/gitoxide/issues/1771)), and uses neither accelerator.
- **Don't scan what's ignored.** Zed leaves gitignored directories unscanned and unwatched until opened, and outside git scans only 5 levels eagerly.

### Large files
- **jj refuses new files over 1 MiB** (`snapshot.max-new-file-size`), by `metadata.len()`, without reading them, and warns with the fixes (ignore it, raise the limit, track it). Tracked files are exempt. People complain about the warning, and refused files are re-stat'ed on every snapshot since jj doesn't cache untracked paths.
- **Server-held history.** Git LFS, Perforce (`+S`/`+Sn` keep only the last n revisions), Mercurial largefiles and Sapling keep large binaries' history on the server. The client keeps the current version plus a cache it may delete, and fetches older versions when asked. What goes remote is chosen by pattern (LFS, Perforce's typemap) or size (`hg lfs`'s `size(">20MB")`, git's `--filter=blob:limit=`).
- **Thresholds in use:** jj 1 MiB, hg largefiles 10 MB, GitHub warns at 50 MiB and refuses at 100 MiB, git stops delta-compressing at 512 MiB (`core.bigFileThreshold`).

### Storage without copies
- **Copy-on-write clones** are a metadata operation, then cost only the blocks that diverge: `FICLONE` on btrfs and XFS (`reflink=1`), `clonefile` on APFS, block cloning on ReFS (Dev Drive). `std::fs::copy` already uses them where it can, and silently does a full copy where it can't; `reflink-copy` 0.1.33 fails instead. Not ext4 (Ubuntu, Debian, WSL), not NTFS (nearly all of Windows), and never across volumes, and a games drive is rarely the volume `$SILT_HOME` is on. So they help most macOS users and a minority of Linux ones.
- **Referencing the working file** costs nothing, but a reference isn't a copy: the next in-place write changes what was "stored". Hard links are as bad. Copying just before a write needs privileged kernel hooks (fanotify pre-content events, a Windows minifilter, Endpoint Security), so it's out. The way to make a reference safe is to treat it as a cache of something durable (the server) and to say, when it changed before it was uploaded, that the old version wasn't kept.
- **Content-defined chunking** (FastCDC, Xet: 64 KiB chunks; restic, borg: about 1–2 MiB) dedups changed versions for upload and server storage: Hugging Face reports 30–85% saved on fine-tuned checkpoints. It saves close to nothing on compressed or re-encoded formats (PNG, MP4, packed archives), and no published figures exist for game assets.
- **zstd** at level 1 compresses at about 400–500 MB/s and gains nothing on already-compressed media, so test per blob and store raw when it doesn't help.

### Costs to plan for
| | 209 GB |
|---|---|
| BLAKE3 on NVMe (3 GB/s, I/O bound) | about 70 s |
| on a SATA SSD | about 6.5 min |
| on a hard disk | about 17 min |
| Upload at 100 Mbit/s | about 4.6 h |
| at 1 Gbit/s | about 28 min |

Even hashing a folder like that can't be on the way to an agent starting. Uploading it can only happen when someone asks for it on another machine.

## Proposed design

### The model: the files as found are the thread's base, held by reference
A thread records what it changes against the files as it found them, and nothing is copied until something needs it. There are three kinds of content, chosen by what the file is:

| Kind | For | Held how | Recorded on change |
|---|---|---|---|
| **Text** | UTF-8, ≤ 5 MiB | its base by reference: git's object (`GitText`) or a content-addressed, compressed blob | CRDT edits on top of the base, as today |
| **Binary** | anything else | by reference: the working file itself, or a CoW clone where the filesystem has them, or a copy when small | replaced: old hash → new hash |
| **Unrecorded** | over the record limit, or not hashed yet | path and size only | replaced: unrecorded → whatever it becomes |

**Text.** Base text has to exist for edits to apply, on every machine that applies them, so a text base is always kept. Today an untracked text file becomes a `Create` with the whole content as one `Insert`, stored in the log and again in `files/`, per thread. Instead, a text file not in git gets its found content stored once as a blob, compressed and addressed by hash, and is editable on top of it exactly as `GitText` is on top of git's object (a `BaseText` lookup by hash, deterministic ids). Text files are small, so keeping their bases is cheap, and content addressing makes a second thread in the same folder cost nothing more. Text bases upload eagerly, within a budget, so a handoff still works with the first machine off.

**Binary.** A binary is replaced whole and merged by picking one, so its history is a list of identities. The blob store gains *adopted* blobs: a hash that points at a working-tree file by (path, size, mtime, inode) rather than owning a copy. Reading one checks the stat and the hash. Where `$SILT_HOME` and the folder share a CoW volume (probed once per volume), Silt clones instead, which is safe and free. Binaries under a small size (say 1 MiB) are copied, since that's cheap and keeps their history whole. When an adopted file changes before anything uploaded it, its old version is **not kept**: history says so, and revert, fork-from-turn and permalinks to it say so too, instead of failing partway.

**Unrecorded.** A file over the record limit isn't read at all, as jj does, but it's still in the thread, by path and size, so the thread knows it's there and a later change to it is a change. Unlike jj's refusal, it's kept with its stat, so it isn't looked at again on every scan. Hashing it later turns it into a binary by a new `SetContent`; deltas never change. The same kind covers binaries the first scan hasn't hashed yet (see "Starting").

This needs one new content variant (`Content::Unrecorded { size }`) and text edits on a blob base (or a `Content::BlobText` variant): `PROTOCOL` 26 → 27, every machine and the server updated together. The full list of match sites is in the appendix.

### Uploading: eager while cheap, otherwise at handoff
- Deltas, text bases and small binaries upload eagerly, as today, so a thread can be resumed with its machine off.
- Large binaries are deferred: **after** the deltas, not before (as attachments over 8 MiB already are), and over a per-family budget, only when asked. The thread then says what is only on this machine ("3 files, 76 GB, only on desktop"), with **Hand off**, which uploads them. `BlobState`'s `refused`/`received`/`from` (ADR 0121) is already the vocabulary for "not here, and why".
- On the other machine, `materialize` writes text and small files, and fetches large ones on demand or in the background, streamed (`download_blob`). A file whose content is on a machine that's offline is left as it is on disk and shown as such, as a file `perform` couldn't write already is (`bridge.rs:190`).
- If the other machine already has the same bytes (a copied folder, assets from Perforce or Steam), hashing its files at handoff and matching by hash means nothing moves.

### Starting: the agent waits only for a listing
1. **List** (the only step before the agent starts): stat every file, and in a git repository ask git what differs from `HEAD`. For a new thread in a folder Silt has seen, a persisted, per-folder stat cache answers most of it.
2. **Start the agent.**
3. **Capture in the background**, off the core, with bounded parallelism: text files that changed or aren't in git are read and stored as bases; binaries are hashed lazily, under an I/O budget; files over the record limit are only stat'ed.
4. **Ahead of the agent:** Claude Code's PreToolUse hook (today `scan_for` ignores it, `capture.rs:199`) names the file an Edit or Write is about to change, so that file is captured first if the background hasn't got to it. ACP agents' pending tool calls name locations too, which needs checking.
5. **A race rule:** a file whose stat changed between the listing and its capture was changed during the scan. Its base is unknown, so it's recorded as found-unrecorded, then changed. That's honest, and the worst case is a diff that shows a whole replace.

The core only applies the resulting deltas, in batches (one transaction per batch, #26).

### The stat cache
A per-folder record of (path → size, mtime in ns, ctime, inode, mode, and BLAKE3 or git oid), in the store, shared by every thread in that folder, not per checkout. It's clean only if the stat matches and the mtime is strictly before the cache's own write time (the racy rule). For a tracked file matching git, it holds the git oid rather than a BLAKE3 hash, so it never has to be read. Later, a directory-level cache as in dirstate-v2 (mtime, all-children-recorded, ignore-rules hash), so a warm scan of an unchanged tree costs about one stat per directory.

### Git repositories (goal 2)
- **First scan from git's own answer.** `git status --porcelain=v2 -z --untracked-files=all` gives what differs from `HEAD`, staged or not, plus the untracked files, and uses the person's fsmonitor and untracked cache. Anything it doesn't name equals `HEAD` and isn't read. Files with `skip-worktree` or `assume-unchanged` set need handling, since git won't report them.
- **Never copy what git has.** A file that matches its base is `GitText` or `GitBlob`, not a blob. That fixes the unchanged-binary copy above on its own, before any of the rest.
- Keep importing `HEAD` as one delta per file for now. For repositories of a million files that's a million deltas per thread; a base tree expanded lazily, with ids derived from (tree, path), is a larger change to the CRDT, to measure before deciding.

### Folders without git (goal 1)
- **A hidden git directory**, under `$SILT_HOME`, with `core.worktree` pointing at the folder, and the deterministic empty root commit chats already use (ADR 0059) as genesis. The folder gets no `.git`, and everything that needs a `Repo` (listing, ignores, filters, file modes) keeps working. Every file is untracked, which the design above makes cheap: text bases once, binaries by reference, large files unrecorded.
- **Default excludes**, since without git nothing is ignored: other VCSs' directories (`.git`, `.hg`, `.jj`, `.svn`), `node_modules`, `target`, `.venv`, `__pycache__`, `build`, `dist`, Unity's `Library` and `Temp`, Unreal's `Intermediate`, `DerivedDataCache` and `Saved`. They go in that hidden directory's `info/exclude`, so they're visible and editable. Any `.gitignore` files in the folder apply as well.
- **When the person later runs `git init`** in the folder, Silt should notice and move the thread onto the real repository. To design.
- **The scan warns** when it takes in thousands of files or gigabytes, naming the directory that contributed most (#221), as git warns when enumerating untracked files takes over 2 s.

### Disk (goal 3)
In order of impact:
1. **Don't copy what git has** (above).
2. **Binaries by reference**, CoW clones where available, copies only when small.
3. **Text bases content-addressed and compressed**: once per content, not per thread, and compress `files/` too.
4. **Local blobs are a cache.** A blob the server has acknowledged may be evicted, least recently used, under a disk budget. What isn't uploaded yet, and what a checkout needs and the disk doesn't have, are kept.
5. **Managed checkouts don't write large found files.** A second thread in an assets folder copies the tracked tree (`git reset --hard`) and small untracked files, and leaves large ones out unless asked, or clones them on a CoW volume. **Open**: what's in a managed checkout.
6. **Server retention** for large binaries' versions: keep all, the last n, or those within d days, with "not retained" handled where old versions are read.
7. **Later: chunking** (FastCDC) for uploads and the server, so a changed 20 GB file uploads its changed chunks, and a resume fetches only the chunks the other machine lacks.

### Watching
- Don't watch ignored or excluded directories. Watch lazily as Zed does, and budget inotify watches.
- `with_compare_contents(false)` on polled filesystems: stat only, then hash only what changed.
- Persist the stat and directory caches so a restart is a stat walk, not a read. On macOS, persist the FSEvents event id and volume UUID too, which needs FSEvents code of our own in `silt-os` (notify doesn't expose `sinceWhen`).
- `notify` 9.0.0 final and `gix` 0.91.0 are out (Silt pins 9.0.0-rc.9 and 0.90.0). `gix-notify` 0.1.0 has the right model (paths to reconcile, losses mean rescan) and is worth a look once its API settles.

## Work, in order

**1. Fixes with no design questions** (no protocol change):
- Stream capture: hash while reading, `put_file` (and stop it hashing twice), never a whole file in memory; the same for `perform`, `disk::replace`, `ingest_renames`, `remote.put_blob` and pulls.
- Reading, hashing and storing off the core, with the core applying the deltas in batches (#26).
- Check a file's size by `metadata.len()` before reading it.
- A file matching its git base is `GitBlob`, not a copied blob.
- Refresh the entry's stat when the hash matches (`core.rs:2178`).
- `with_compare_contents(false)`, and no watches in ignored directories.
- Upload tree blobs over 8 MiB after their deltas.
- Flag concurrent changes to one binary as a conflict in merge requests.
- Collect unreferenced blobs on the daemon.
- `notify` 9.0.0, `gix` 0.91.0.

**2. A fast start** (goal 2):
- The per-folder stat cache, shared by threads.
- The first scan from `git status`.
- The agent starts after the listing, with background capture, PreToolUse ahead of the agent, and the race rule.

**3. The content model** (goals 1 and 3; `PROTOCOL` 27 and an ADR):
- Text bases as compressed blobs, editable as `GitText` is.
- Adopted binaries, CoW clones, `Unrecorded`.
- Deferred upload and Hand off; lazy materialize.
- Local eviction.
- Folders without git: the hidden git directory and default excludes.

**4. Later:**
- Server retention; chunking.
- Persisted watcher cursors; the directory cache.
- A lazily expanded base tree for very large repositories.

### Measuring it
New `silt-bench` fixtures and budgets (ADR 0034):
- a git repository of 1M files with 5% modified and 10k untracked;
- a folder without git of 100k files and 200 GB, made of sparse files, so the fixture costs no disk but reading it costs real time;
- for each: time from opening the thread to the agent starting, peak memory, extra disk as a share of the folder, and how long a view waits during the background capture.

## Open questions
1. **The record limit.** Above what size is a file only stat'ed, never read unless asked? jj uses 1 MiB for new files, #311 suggests 100 MiB. Per workspace, per project, or both?
2. **What uploads eagerly.** Above which size, or past which per-family total, does content wait for Hand off? This trades "resume with the first machine off" for upload time.
3. **Not kept.** When an adopted binary changes before it's uploaded, its old version is gone. Is that acceptable, given that revert to before that change can't bring it back?
4. **History of large binaries.** Keep all versions on the server, or only the last n?
5. **Folders without git.** A hidden git directory under `$SILT_HOME` (recommended), `git init` in the folder, or capture without git at all (the `ignore` crate for listing, and a `Repo`-less checkout, which touches much more code)?
6. **Managed checkouts in large folders.** Leave large found files out, clone them where possible, or point people to threads in place?

## Appendix: where a new content kind needs handling
Every match on the content enums (`Content`, `FileContent`, `FileView`, `DiskContent`). `Content` is postcard-encoded by position, so a new variant goes last.
- `silt-log`: `delta.rs:290-300` (`Delta::blob`, which drives upload, the server's index and read access, its GC and erase), `delta.rs:324-330`, `family.rs:64-76`, `963-987`, `1009`, `1020`, `1028`, `1148-1156`, `1178`, `1185`, `1210`, `1290-1295`, `1310-1316`, `family/snapshot.rs` (`SNAPSHOT_FORMAT`), `purge.rs:59`, `blame.rs:83`.
- `silt-checkout`: `bridge.rs:40`, `271-273`, `371-373`, `426-428`, `487-508`, `519`; `import.rs:71`.
- `silt-daemon`: `core.rs:433-441`, `2215-2219`, `2342-2344`; `views.rs:478-487`, `520-527`, `604-609`, `635`; `revert.rs:70-72`; `sync.rs:749-766`; `erase.rs:97`, `252`; `forks.rs:549-618`; the `Request::GitText` route (`protocol.rs:1099`, `core.rs:1540`, `web.rs:1013`, `lan/needs.rs:117`).
- `silt-server`: `browse.rs:2341-2345`, `index.rs:129`, `lifecycle.rs:111-113`.
- `silt-wasm`: `lib.rs:96`, `105-107`.

Invariants to keep: deltas never change (bytes may only arrive later behind a hash); the projection stays pure and builds for wasm32; capture works without a daemon (ADR 0018); reading bytes is allowed by the delta that names them (`readable_blob`); writes go through `is_writable`, and a file not written is never recorded as deleted; both serving modes answer alike through `silt_proto::view`.
