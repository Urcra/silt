<!--
Title: a Conventional Commit, e.g. `fix(attachments): start a thread with large files`.
Write in plain sentences. Delete these comments and any section that doesn't apply, except Screenshots for a UI change.
-->

## Why

<!-- The problem or the need, as someone who hits it would see it. For a bug: what failed, and the cause. Link the issue, and the ADR when there is one. -->

## What

<!--
What changed, grouped by area (crate, the web UI, docs), with the files or APIs that matter.
Say so here when:
- the wire format changed and `silt_proto::PROTOCOL` went up: every machine and the server need updating together;
- what the UI reads changed: both the daemon's `/api` and the server's `/app/api`;
- a migration was added.
-->

## Screenshots

<!--
Required for any change to the UI (web/, the desktop app, the hosted app): a picture of each new or changed screen, state or dialog, pasted into this description. Not needed when nothing on screen changed.
Show dark mode and a phone's width too where the change looks different there.
Regenerated screen references (`*.spec.ts-snapshots`) don't count: say which changed and why, under Tests.
-->

## Tests

<!--
- The new tests, by crate, and what each one proves. Say if a test fails with the fix reverted.
- What you ran: CI runs formatting, lints and the tests; say whether you also ran `scripts/gate.sh`, and with `SILT_BENCH_ENFORCE=0` or a step skipped.
- What you checked by hand, and what you couldn't check (macOS, Windows, a real agent CLI).
-->

## Not in this PR

<!-- Follow-ups and known gaps, each with its issue. -->
