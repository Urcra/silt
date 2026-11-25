# 0123: Redaction ids keyed per machine

- Status: accepted
- Date: 2026-11-26
- Amends: [ADR 0015](0015-secret-overlay.md) (the placeholder's id)

## Context
A redacted secret becomes `[REDACTED:<id>]`, and the id was the first 12 hex digits of the secret's SHA-256. ADR 0015 accepted that this lets someone with the server's data test guesses, since redaction is meant for keys and tokens. In practice the secrets file and secret-looking environment variables also catch passwords. A placeholder for `DB_PASSWORD=hunter22` then reaches the server, and every teammate who can read the thread, as a hash anyone can check a wordlist against offline.

The id only has to tell one machine's secrets apart and stay the same when a delta is pushed again. Since ADR 0019 there is no overlay, so nothing matches a placeholder against a secret on another machine.

## Decision
**The id is a keyed hash:** the first 12 hex digits of HMAC-SHA256 of the secret, under a key the machine makes once.
- The daemon keeps its key in `redaction-key` in its data directory, readable by its user alone, and never sends it anywhere.
- The server keeps its own in its data directory, for the placeholders it writes into what it records itself (the Agent SDK session store).
- If the key can't be kept, the daemon makes one for the run and says so. Placeholders then differ after a restart, which costs nothing but their sameness.
- The placeholder's form is unchanged, so no protocol or UI changes.

## Consequences
- **Guesses can't be checked without the machine's key**, so a weak secret in a placeholder no longer gives itself away.
- **The same secret has a different id on each machine.** Nothing compares them across machines.
- **Placeholders already pushed keep their old, unkeyed ids.** Rewriting pushed deltas isn't possible, so a secret that may have been exposed that way should be rotated.
- Deleting the key file only changes the ids of later placeholders.
