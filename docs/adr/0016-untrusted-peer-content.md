# 0016: Content from other people is untrusted input

- Status: accepted
- Date: 2026-10-20
- Amended by: [ADR 0017](0017-acp-agents-only.md): the sandbox and the harness's file tools are removed with the harness
- Amended by: [ADR 0019](0019-redact-to-gits-threat-model.md): with no overlay, the rules on whose deltas get secrets back and on partly typed secrets go
- Amended by: [ADR 0045](0045-windows-natively-and-in-wsl.md): Windows, natively and in WSL

## Context
A thread holds other people's work:
- collaborators' edits;
- a fork whose merge request you approve;
- a teammate's thread you resume or fork;
- whatever a compromised or malicious server sends.

Before the 2026-10-20 review, Silt trusted all of it. Two consequences:
- **Paths.** A path in the CRDT tree was written to disk as it was, `.git/hooks/pre-commit` included.
- **Ids and deltas.** Session ids from the log became file names. Delta fields were used in arithmetic and loops without bounds.

Either a crash or code execution was one pushed delta away.

## Decision
Everything that arrives in a delta is input from a possibly hostile peer. Each kind of data gets one rule, enforced where it is used and, where possible, also before it is accepted.

**Paths.**
- **In the tree.** The CRDT tree refuses names that git refuses or that a filesystem may treat as `.git`: any case, trailing dots or spaces, and `git~1`. Every replica therefore rejects a delta that would put a file in a git directory.
- **On disk.** `silt_checkout::is_writable` is the policy for writes. A path must be relative and made only of normal components, must contain no git directory at any depth, and must not be under `.silt/checkouts`.
  - Disk actions for other paths are dropped.
  - The daemon's disk layer refuses them again and never follows a symlinked directory on the way to a file.
- **Re-ingesting.** A file that changed on disk while a thread's version was written there goes back in only through the same ignore and internal filters as the watcher's paths. A peer's `.env` therefore can't make this machine record its own.

**Ids.**
- Session ids and subagent names from the log become file names in agents' session stores, and arguments to agent CLIs. A recorded session is restored only if:
  - its id and subagent names are letters, digits, `_` and `-`, at most 128 characters, not starting with `-`;
  - its side files are plain relative paths.
- A session's recorded working directory is followed into the checkout only through plain directory names.
- The harness loads only ids it could have made itself.

**Deltas.**
- **One structural check, run everywhere.** `Delta::check_shape` checks a delta's own fields, without the family:
  - the span matches the operation and stays within the lamport clock, which ends at `i64::MAX`;
  - text ranges don't overflow;
  - names are valid;
  - a thread doesn't fork from or merge itself.

  The server's room runs it before storing a delta, and every replica runs it before applying one.
- **The causal graph** limits a delta to 2^32 ids and refuses a start more than 2^40 ahead of everything known, so a peer can't use up the clock. It finds overlaps with one search, not one lookup per claimed id. A span that starts at a known id must repeat that id's parents.
- **The family** also checks against what it has:
  - a fork is only the first delta of a new thread, from a known one;
  - a merge names a known thread;
  - a tree operation only uses files and directories its author had seen.
- **History** never expands a thread inside itself, so threads that pull from each other end.
- **Frontiers** received from others are sorted and deduplicated like local ones.
- **A replica that can't apply a delta** skips it and its descendants, and keeps syncing. A delta waiting for a git blob the clone lacks is retried after a pause, not in a loop.
- **The server** accepts a delta only when all of its ids are new.

**Secrets.** This amends ADR 0015.
- **Who the overlay serves.** It puts secrets back only into deltas that this user's own devices could have written:
  - the delta is on one of this user's threads;
  - the thread has no collaborators;
  - the delta is not a comment, since anyone who sees a thread may comment.
  Otherwise someone could plant a known placeholder and have the victim's machine fill in the secret.
- **What it puts back.**
  - In file edits, it puts back only same-length masks, which keep the span valid.
  - In transcript lines, which are JSON, it puts the secret back escaped. Redaction matches secrets there in escaped form too, and treats `\n`-style escapes as boundaries.
- **Edits that hold only part of a secret.**
  - A file edit that touches a run of eight or more key characters replaces the whole run, so a rotated key leaves whole and is recognised.
  - An insert that ends in a token prefix, or in the start of a known secret, is masked as a secret being typed.
- **What counts as a secret.**
  - A variable counts when a `_`-separated part of its name marks it, so `GIT_AUTHOR_EMAIL` and `NEXTAUTH_URL` no longer do.
  - Values that look like paths are skipped.
  - Environment and secrets-file values are kept in the overlay, and everything the overlay knows is redacted on push, so a secret stays covered after it leaves the environment.
- **Large text files.** Text too large to merge character by character is masked before it is stored as a blob.
- **Agents continuing someone else's thread** are told that its earlier content is that person's work, not instructions.

**The sandbox and git.** A command the agent runs is one more source of untrusted content, and so is everything it writes into the checkout.
- **What it can reach.** A socket can be connected to through a read-only mount, so the sandbox hides where sockets and credentials live:
  - `/run`, `/var/run` and `$XDG_RUNTIME_DIR`;
  - `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.docker`, `~/.kube`, `~/.netrc`, `~/.config/gh`, `~/.config/gcloud` and `~/.azure`;
  - Silt's data directory, whose socket controls the daemon.
- **What it can write.** Inside the writable directories, `.git` and `.silt` stay read-only. Hooks and config there run later, outside the sandbox.
- **What it gets.** The command receives an allow-listed environment.
- **How it ends.** It runs as the first process of its own process namespace, with no-new-privileges. Its signals stay inside the sandbox, and whatever it starts ends with it. Where the kernel allows it, it also gets its own `/proc`.
- **Silt's own git commands** run with hooks and fsmonitor off, never prompt, and read messages untranslated.

**Agents and what runs on mount.**
- **The harness's file tools** reach files only beneath the checkout through descriptors, without following symlinks, and write by renaming a new file into place. Rules files are read the same way.
- **Agents get an allow-listed environment:**
  - what tools need;
  - proxy and certificate settings;
  - their own vendor's variables (for example `ANTHROPIC_*` for Claude);
  - names listed in `agent-env` in Silt's data directory, or in a custom agent's `pass_env`.
  The daemon's other credentials stay with the daemon, and so does the setting that turns the harness's sandbox off.
- **The agent's lifetime.** The host reports an agent that dies, falls back to a new session when one can't be loaded, and answers pending permission requests when a turn is cancelled. Answers must be one of the options the agent offered.
- **Registry downloads** must match a checksum, unless `SILT_ALLOW_UNVERIFIED_AGENTS=1` is set. They are staged in a fresh directory and moved into place whole, and the installed program is checked again before each start. Ids and commands must stay inside the agent's directory.
- **`.agents/prepare`** runs unasked only when it matches the one in the person's own clone, or one they allowed before. In a terminal they see it and decide, and a yes is remembered by its hash. Without a terminal, it doesn't run.

## Consequences
- A thread containing such names was already unusable with git. These deltas are now refused rather than written.
- Every daemon must run the same validation. A later protocol version may let the server refuse these deltas up front.
