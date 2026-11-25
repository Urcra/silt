# 0066: Pending review comments follow their text

- Status: accepted
- Date: 2026-11-14
- Amends: [ADR 0055](0055-reviewing-a-threads-changes.md) (a pending comment's lines)

## Context
ADR 0055 keeps a review in the browser until it's sent, and each pending comment there kept only the line numbers the diff showed when it was written. It accepted the cost: if the agent changed the file before the review was sent, the comment could land beside other lines.

The expanded review (ADR 0067) is always live, so the agent changing a file under a review in progress is the usual case. A comment written on one function then sits beside another, and is sent to the agent quoting the wrong code.

A recorded code comment doesn't have that problem: it holds CRDT anchors on its first and last characters, which follow the text through edits and say when it's gone.

## Decision
**A pending comment takes its anchors when it's written, though nothing is recorded.**
- `POST /api/threads/{id}/comments/anchor` takes lines as a comment would (`on: lines`, with a base for deleted lines). It answers with the anchors as the comment would hold them, serialized, and appends nothing to the log.
- The browser keeps that string in the draft, opaque, beside the line numbers and the quoted lines.
- Drafts written before anchors take one once, best effort, where their line numbers point now.

**Drafts are placed where their text is now.**
- `POST /api/threads/{id}/comments/place` places a batch of anchors as the thread's comments are placed: their lines now, outdated once their text is gone, and their lines at the base asked for.
- A draft shows on its lines as they are now. One whose text was deleted shows beside the base's lines when the base still has them. One gone from both shows apart, with the lines it was written on, and is sent as lines since deleted.
- A draft with no anchor, or whose anchor can't be placed, stays where it was written.

**Finishing records the anchors themselves.** `CommentOn::Anchored { anchor }` records a code comment from a draft's anchor, so it lands where its text is now, or outdated where it was. The prompt quotes the lines as they are when it's sent.

**Anchors from the browser are untrusted.** They're size-limited and parsed strictly. They must name one of the family's text files, and must resolve in it at the thread's head. Otherwise placing answers `unknown`, and recording is refused.

## Consequences
- The log format doesn't change, and nothing synced does: anchors were already what code comments hold, and the new endpoints are the daemon's. No protocol bump.
- A draft resolves only on a daemon that has its family. That's fine, as drafts are a browser's, and the hosted app takes no code comments.
- Placing runs again on each delta, as the review's other queries do.
