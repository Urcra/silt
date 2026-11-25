# M0 probes

## ACP smoke test, 2026-10-20
Command: `cargo run -p silt-acp --example acp_smoke -- <repo> "<prompt>"`. It uses `agent-client-protocol` 3.4 (`process` feature) with the default agent `npx -y @agentclientprotocol/claude-agent-acp@0.94.0`.

**Result: works.** The agent ran the prompt, wrote `hello.txt` and stopped with `EndTurn`.

What the adapter reported at `initialize`:
- `load_session: true`
- Prompt capabilities: image and embedded_context (no audio)
- MCP: http and sse
- Session capabilities: `list`, `delete`, additional dirs

What it streamed during the turn:
- `AvailableCommandsUpdate`: the user's Claude Code skills and slash commands. We can show these in the composer.
- `UsageUpdate {used, size, cost}`, with `_claude/model` in `meta`. This gives us the context meter for free.
- `ToolCall` (`kind: Edit`, `name: "Write"`), then `ToolCallUpdate`s carrying title, locations, `Diff {path, old_text, new_text}` content, and finally status `Completed`.
- `AgentMessageChunk`s with a stable `message_id`, which we can coalesce into one message.

Implications:
- **The file was written straight to disk.** We didn't advertise `fs.writeTextFile`, so the agent never used it. Claude Code's own tools write files directly. **The checkout watcher is therefore the primary edit path, and `fs/*` is secondary.** Attribute disk edits made during a turn using the tool call's `locations` and `Diff` content.
- **The SDK API is builder-based.** It looks like `Client.builder().on_receive_notification(...).on_receive_request(...).connect_with(agent, async |cx| ...)` and the futures are `Send`. The plan's "LocalSet per agent thread" workaround isn't needed.
- **One turn cost about $0.13** on the user's Claude account. Use `silt-fake-agent` for automated tests.


## Claude Code session portability, 2026-10-20
**Question:** can a Claude Code session move to another checkout path, or another machine, and resume with its full context? Silt's handoff and fork features depend on this.

**Session format.** Sessions are stored in `~/.claude/projects/<slug(cwd)>/<session-id>.jsonl`, where the slug is the cwd with every non-alphanumeric character replaced by `-`.
- The file is append-only JSONL. Entries are linked through `uuid` / `parentUuid`, so a session is a tree: rewinds and edits create branches.
- Every entry carries `cwd`, `gitBranch`, `sessionId`, `version`, `timestamp`.
- Entry types: `user` and `assistant` (whose content blocks are text, thinking with a signature, tool_use and tool_result), `system` (hook summaries, compaction boundaries), `attachment` (hook output), `file-history-snapshot` / `file-history-delta` (Claude's own edit checkpoints), `ai-title`, `agent-name`, `last-prompt`, `mode`, `permission-mode`, `queue-operation`.
- `<session-id>/subagents/agent-*.jsonl` (+ `.meta.json`) hold subagent transcripts. `<session-id>/tool-results/` holds large tool outputs.
- `user` entries with `isMeta: true` are text Claude Code adds as the user's turn: a skill's body, Stop hook feedback, notes on images, messages from other sessions. Nobody typed them, so Silt doesn't read them as prompts (checked against real transcripts, 2026-10-21).

**Compaction.** Entries from before a compaction stay in the file. Compaction adds a boundary and a summary but deletes nothing. Silt can therefore index and serve the full history across compactions.

**Relocation test.** Took the ACP-smoke session (39 entries) and:
1. Replaced every occurrence of the old checkout path with a new path.
2. Gave it a new session id.
3. Wrote it to the new path's slug directory.
4. Ran `claude -p --resume <new-id> "...what file did you create earlier?"` from the new checkout.

**Result: works.** The agent answered correctly from the conversation history and used the rewritten path.

Notes:
- Claude Code has `--fork-session` (resume under a new id) and `--session-id`, which help with forks.
- **Still open:** thinking-block `signature`s may be tied to the account or org. Resuming *another person's* session could fail API validation. If it does, strip thinking blocks (or downgrade them to text) when translating a session across accounts. This needs a second account to test.
- Hooks (`SessionStart`, `UserPromptSubmit`, `PreToolUse` / `PostToolUse`, `PreCompact`, `Stop`, `SessionEnd`) receive `transcript_path` and `tool_use_id`. They let Silt mark turn boundaries and attribute disk edits to specific tool calls.

## gix filter pipeline, 2026-10-20
Setup: `gix = { version = "0.89", default-features = false, features = ["attributes", "sha1"] }`, which pulls in gix-filter 0.36.
API:
- `let (mut pipeline, index) = repo.filter_pipeline(None)?`
- `pipeline.convert_to_git(reader, path, &index)`
- `pipeline.convert_to_worktree(&blob, path, Options { can_delay: Delay::Forbid, .. })`

**Method:** 256 comparisons against real git, using 8 content variants per case (lf, crlf, mixed, lone CR, NUL, no final eol, `$Id$`, UTF-8).

**Result: 244 byte-identical.** These all match git:
- `core.autocrlf` true/input; `text=auto eol=crlf`; per-extension `eol`; `binary` and `-text`
- `core.eol`, safecrlf, CRLF already in the index
- one-shot `clean`/`smudge` drivers, including `%f`
- long-running `process` drivers (protocol v2): one child per `Pipeline`, a single handshake for many files
- driver failure semantics (`required=true` errors; otherwise falls back to the unfiltered content)

Differences:
- **`ident` smudge:** gix writes `$Id: <sha>$` where git writes `$Id: <sha> $` (git has a space before the closing `$`).
- **`working-tree-encoding` UTF-16:** smudge is silently wrong (encoding_rs can't encode UTF-16, so the output is UTF-8).
- **`working-tree-encoding` ISO-8859-1:** treated as windows-1252, so results differ from git.

Implications:
- clean(smudge(x)) ≠ x in general (e.g. mixed line endings under `eol=crlf`). **The CRDT keeps blob bytes as the source of truth** and never re-cleans its own smudged output.
- clean depends on the index, so pass an index or tree that matches the thread base.
- A `Pipeline` is not `Sync`. Use one per worker and reuse it, so process filters (git-lfs) stay warm.

**Decision:** gix is the default.
- **Fallback to git** for paths whose attributes set a non-UTF-8 `working-tree-encoding` or `ident`:
  - smudge: `git cat-file --filters --path=<p> <blob>`, once per file. `--batch --filters` is broken in git 2.43.
  - clean: `git hash-object -w --path=<p> --stdin`

## Checkout strategies, 2026-10-20
Test repos:
- simc: 541k objects, 5.3 GiB packed, 1.2k files
- linux: shallow, 74k files, 1 GiB tree

Strategies compared:
- **(A)** `git clone --local --no-checkout`
- **(B)** Alternates: a separate git dir with alternates pointing at the source's objects
- **(C)** `git worktree add --detach`

| Repo, 5 checkouts | A | B | C |
|---|---|---|---|
| simc total | 5.5 s | 5.9 s | 5.9 s |
| linux total | **47.6 s** (5.2 s per clone, 222 MB each: `--local` is ignored for shallow sources) | 21.7 s | 22.6 s |
| git-dir size per checkout | 1.2 MB unique on simc, 222 MB on linux | 272 KB / 7.6 MB (mostly the index) | 144 KB / 7.5 MB |

- **Parallel materialization:** `-c checkout.workers=8` cut linux materialize from 4.2 s to 1.6 s.
- **(A)'s hidden cost:** after the source runs gc/repack, its hardlinked packs are no longer shared with the source. simc grew by +5.4 GB.

### Gc safety
Each checkout made an agent commit; then the source deleted the branch and expired its reflog.

| Source action | A | B | C |
|---|---|---|---|
| `gc --prune=now` or `repack -ad` | ok | **broken** | ok |
| same, with `refs/silt/threads/<id>` kept in the source | ok | **ok** | ok |

- Under (B), `git status` still exits 0 even when the base commit has been pruned. Health checks must use `fsck --connectivity-only` or `rev-list`.
- `gc.pruneExpire=never` does *not* protect against `--prune=now` or `repack -ad`.

### Hazards of (B), with verified mitigations
- **A git dir inside the worktree gets staged by `git add .`** in the source. Fix: put it under `$SRC/.git/silt/clones/<id>.git` (verified: the source's status stays clean and the source gc leaves it alone), or add the directory to `.git/info/exclude`.
- **Copying the source's `packed-refs` leaves dangling refs** once those branches are deleted, which breaks `git gc` and `fsck` in the clone. Don't copy them; keep only HEAD and Silt's own refs. Copy `.git/shallow` when the source is shallow.
- **Alternates are live:** the clone immediately sees objects the source fetches later.

### Push to `local` with `receivepack = git -c receive.denyCurrentBranch=updateInstead receive-pack`
- Works on a clean checked-out branch, and works on a branch that isn't checked out.
- Rejected when the source has unstaged or staged changes, or an untracked file that collides with a pushed path.
- Pushing to `refs/silt/threads/<id>` works. The same trick works with (A).

**Decision: (B), with the mitigations above** (recorded in ADR 0002). Fall back to (A), or to cloning from the server mirror, when there is no local source clone.

## diamond-types as differential oracle, 2026-10-20
**Versions**
- crates.io `diamond-types` 1.0.0 is the 2022 1.x branch (`list::OpLog`). It assigns sequence numbers automatically and **cannot take caller-chosen seqs**.
- Git `master` (2.0, unpublished; `ListOpLog`, `add_operations_remote(agent, &sorted_parents, start_seq, &[TextOperation])`) needs Rust ≥ 1.96. We pin 1.99.1, so **use master as a git dev-dependency** (pin a rev).
- Rev `c2c95b9` + `smallvec = "=2.0.0-alpha.7"` is the fallback for older toolchains.

**Results**
- TIVITY + `a`:"REA"@0 ∥ `b`:"C"@0 → **"REACTIVITY"** in every delivery order. Swapping the agent names gives "CREATIVITY".
- Delete/insert overlaps, 3-way concurrency, and forward/backward typing interleavings all converged in every delivery order.

**Ordering:** YjsMod `integrate` (`src/listmerge/merge.rs`). Concurrent siblings with the same origin_left and origin_right are ordered by agent name (byte order), then seq. **Lamport timestamps are not used.**

**Mapping Silt ids to diamond-types**
- Agent name = `format!("{:032x}", actor)`, so byte order equals numeric order. Silt's sibling tie-break must be: lower actor first, then lower id.
- **Ids are per character.** An insert of length n uses seqs [s, s+n). Feeding one id per multi-char event silently drops data.
- Seq gaps are fine, and re-delivery is idempotent (returns `EMPTY`).

**Gotchas**
- Parents must be sorted local LVs that are already in the oplog. Unsorted parents only trip a `debug_assert`, so run the oracle in debug builds.
- Positions are Unicode scalar values.
- `"ROOT"` is a reserved agent name.

## sqlx 0.9 migrations, 2026-10-20
**The installed `sqlx` CLI was built with only `postgres,rustls`.** As a result:
- `migrate run` against SQLite fails ("no driver found for URL scheme sqlite").
- Any `sqlx.toml` makes `migrate add` fail.

Fix: reinstall with `cargo install sqlx-cli --locked`. The default features include sqlite, postgres and sqlx-toml. A minimal alternative is `--no-default-features --features sqlite,sqlx-toml,completions`.

**Creating and applying**
- `sqlx migrate add -r <name> --source crates/<c>/migrations` creates `<UTC timestamp>_<name>.{up,down}.sql`. Later `add` calls stay reversible.
- Two crates can get the **same timestamp** within one second. Use per-crate tables (below) or sequential versioning.
- Applying: `sqlx migrate run --database-url 'sqlite://<db>?mode=rwc'`. Without `?mode=rwc`, a missing file isn't created. `migrate info` and `migrate revert [--target-version 0]` both work.

**Two crates in one database**
- They **collide in the default `_sqlx_migrations` table** ("previously applied but is missing").
- Fix: a per-crate `sqlx.toml` with `[migrate] table-name = "_silt_<crate>_migrations"`. This needs the `sqlx-toml` feature on both the crate and the CLI. Verified: both sets coexist and re-runs are no-ops.

**Runtime embedding**
- `sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio","sqlite","migrate","macros","sqlx-toml"] }`
- `pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");` The path needs `./`; a bare `"migrations"` is rejected.
- Use `SqliteConnectOptions::create_if_missing(true)`.
- `sqlx migrate build-script` generates a `build.rs` with `rerun-if-changed`.

**Compile-time checks**
- Root `.env` `DATABASE_URL=sqlite://target/dev.db`.
- `cargo sqlx prepare --workspace` writes the cache to the root `.sqlx/`. CI runs with `SQLX_OFFLINE=true` plus `prepare --check`.
- Run the CLI from the crate dir, so its `sqlx.toml` applies.

## File watching on WSL2, 2026-10-20
Tested with notify 8.2.0 and notify-debouncer-full 0.6.0.

**ext4**
- inotify is instant. debouncer-full at 200 ms coalesces atomic saves (50× tmp+rename becomes one `Create(File) target`).
- 20k file burst: no overflow.
- Limits: 524288 watches, 128 instances, 16384 queue.
- **Files created right after `mkdir` are missed** (993–998 of 1000 seen): rescan new dirs ourselves.

**/mnt/c (9p drvfs, statfs magic `0x01021997`)**
- **inotify delivers nothing**, not even for writes made from inside WSL. Windows-side writes are likewise invisible.
- `PollWatcher` catches changes in about 300 ms, but compares mtime in **whole seconds** (same-size writes within one second are missed).
- A scan of 10k files takes 5.1 s (6 ms on ext4).
- Writes are about 100× slower.

**Watcher design**
1. Pick a backend per watch root via statfs: inotify plus debouncer on native filesystems; polling on 9p, drvfs, fuse, cifs and nfs.
2. Treat every event as "path may have changed", then stat, read and diff against the CRDT's last-known content hash. Event kinds are unreliable.
3. Rescan subtrees on `Create(Folder)` and on Rescan/overflow.
4. Use a Silt-owned poller comparing (mtime in ns, size, inode) instead of `PollWatcher`.
5. Warn when a checkout lives under `/mnt/<drive>`. Managed checkouts stay on ext4.

## Local models on a machine (ADR 0092), optional
The models to try first on each target machine were researched rather than probed (`docs/research/local-models.md`). This script is for whoever wants to check their own machine:
```sh
scripts/local-model-probe.sh /path/to/model.gguf
```
It writes `local-model-probe-<os>-<arch>.md` in the current directory, with paths under the home directory shortened to `~`. The report covers:
- **The machine:** its GPU, driver, CUDA or Vulkan, and memory; on Strix Halo the GTT size and the TTM limits; on macOS the GPU's wired limit.
- **llama.cpp's latest release:** the assets for this OS and their digests. It downloads the Metal, Vulkan, CPU and any CUDA build, and records each archive's SHA-256, what `--version` and `--list-devices` report, and missing shared libraries. A `llama-server` on the `PATH` or in `$LLAMA_SERVER` is tried first.
- **The model on each build:** the load time and tokens per second.
- **The first build that loads it:**
  - `/props` (context and chat template);
  - a long prompt sent twice, to see the prompt cache;
  - a tool call on the OpenAI chat API and on the Anthropic Messages API.
- **Claude Code on it,** with a fresh `CLAUDE_CONFIG_DIR`:
  - whether it reads a file and answers;
  - how long that takes;
  - which endpoints it called;
  - the variables its binary reads about models, context and compaction.

Results can go here, one subsection per machine.
