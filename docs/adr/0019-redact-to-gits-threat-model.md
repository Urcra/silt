# 0019: Redact to git's threat model

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0015](0015-secret-overlay.md) (masks in file edits, the overlay), [ADR 0016](0016-untrusted-peer-content.md) (who the overlay serves, partly typed secrets)
- Amended by: [ADR 0060](0060-ignored-files-leave-the-thread-and-large-diffs-are-read-by-file.md): Files git comes to ignore leave the thread, and a large diff is read a file at a time

## Context
ADR 0015 masks secrets in file edits and puts them back, through a local overlay, on the machines that know them. Since then:
- **Files already follow git's rules.** Silt takes in only what `git ls-files --cached --others --exclude-standard` lists, so ignored files such as `.env` never enter a thread. A secret in a tracked or committable file reaches the remote with the next `git push` anyway. Masking it in Silt alone protects nothing git doesn't already expose.
- **Masks make files wrong.** A machine without the secret checks out a file that differs from the repository, and pushing it back to git commits the mask. Agents read the mask and build on it.
- **The overlay is a liability of its own.**
  - It keeps every secret it ever saw in plain text in the local database.
  - It needs rules about whose deltas may be filled in (ADR 0016), because a planted placeholder could get a secret written back.
  - Partly typed secrets need guessing at token prefixes.

## Decision
**Files follow git: `.gitignore` is the boundary.**
- What git would commit is synced as it is, with no masks, in text edits and in large files alike.
- What git ignores stays out of threads, as before.

**Conversation and live signals are still redacted, and nothing puts secrets back.**
- **Still redacted:**
  - transcript lines and side files;
  - titles, merge request texts and comments;
  - drafts and streaming text in presence.
- **No overlay.** Placeholders `[REDACTED:<id>]` stay placeholders on every machine, including in restored sessions.
- **Sources stay the same:** the environment, the `secrets` file and token formats.
- **Remembered secrets.** The local table of secrets is kept for redaction only. A secret redacted once stays redacted after it leaves the environment, but the table is never read to fill anything in.

## Consequences
- **Tracked secrets are visible** to everyone who sees the thread, as they would be to everyone who can clone the repository. Keep them in ignored files.
- **Resumed sessions show placeholders** where the original transcript had a secret. The agent on the other machine reads the placeholder, not the value.
- **Old masks remain.** Deltas pushed before this keep their masks in file text. Nothing reveals them any more, so a file that still holds one should be fixed by hand.
- **Less to get wrong.** No reveal rules, no masks to keep at the right length, and no guessing at partly typed tokens.
