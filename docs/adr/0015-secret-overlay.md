# 0015: Secret redaction with a local overlay

- Status: accepted
- Date: 2026-10-20
- Amends: [ADR 0009](0009-sync-and-handoff.md) (redaction)
- Amended by: [ADR 0016](0016-untrusted-peer-content.md): secrets go back only into this user's own content, and only as masks in file edits
- Amended by: [ADR 0017](0017-acp-agents-only.md): the harness, and with it redaction towards model APIs, is removed
- Amended by: [ADR 0019](0019-redact-to-gits-threat-model.md): files follow git, so edits are no longer masked, and there is no overlay; conversation text is still redacted
- Amended by: [ADR 0123](0123-redaction-ids-keyed-per-machine.md): the id is keyed with a key that stays on the machine

## Context
ADR 0009 redacts secrets when the daemon pushes: values of secret-looking environment variables and well-known token formats become `[REDACTED]` in conversation text and `*` per character in file edits. The machine that wrote them keeps the real values in its own log, so its checkout and history stay intact.

Everywhere else, the secret is gone for good:
- A person's other machine resumes a thread with `****` in its files and `[REDACTED]` in the agent's transcript, even when that machine has the same key in its environment.
- Nothing can tell which secret a placeholder stood for, so nothing can put it back.
- A token found by its format is forgotten once it is covered.
- Silt's harness sends file contents and command output to a model API as they are.

## Decision
**Placeholders name the secret without giving it away.**
- Conversation text gets `[REDACTED:<id>]`.
- File edits get a mask of the same length: `*`, the id, then `*` up to the secret's length, so CRDT positions stay valid.
- The id is the first 12 hex digits of the secret's SHA-256. That is enough to tell a machine's secrets apart. Like any hash, it lets someone with the server's data test guesses (see Consequences).
- Redaction stays deterministic, so pushing a delta again gives the same bytes.

**Secrets come from three places:**
- this process's environment, as before;
- the `secrets` file in Silt's data directory, one value per line, for secrets that live in files such as `.env`;
- token formats.

**The overlay.**
- Each machine keeps the secrets it knows in a local table. It holds the environment's and the file's secrets, plus every token that redaction found here.
- The table is never synced.
- Deltas from the server go through the overlay before they are stored and applied: placeholders and masks of known secrets are put back. The log, the checkouts, history, search and restored sessions on that machine then hold the real values. Other machines keep the placeholders.
- To give another machine a secret, add it to that machine's `secrets` file. This only affects deltas that arrive after the secret is known.

**The harness and model APIs.**
- `silt harness` redacts everything it sends to the model, with placeholders: the prompt, the project's rules files and every tool result.
- It puts known secrets back into what the model asks for (file contents, edits, commands) and into what it says to the person.
- The model can therefore copy, move and use a secret it never sees.

**Live signals.** Drafts and an agent's streaming text in presence heartbeats are redacted too.

## Consequences
- **Old forms.** Deltas pushed before this keep the old anonymous `[REDACTED]` and `****`, which nothing can put back.
- **A machine's log can differ from the server's.** On a machine that knows a secret, the bytes of a pulled delta differ from the server's copy. Deltas are identified by id, so sync is not affected.
- **Plain text, locally.** The overlay keeps secrets in the daemon's local database in plain text. That is no worse than before: the agents' own transcripts and the local log hold them already.
- **A file edited by hand on a machine without the secret** contains the mask. It is pushed as edited, as with any other text.
- **The hash is a verifier.** A short hash of a weak password still lets someone with the server's data test guesses against it. Redaction is meant for keys and tokens.
