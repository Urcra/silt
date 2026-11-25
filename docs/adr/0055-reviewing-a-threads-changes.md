# 0055: Reviewing a thread's changes

- Status: accepted
- Date: 2026-10-29
- Amends: [ADR 0011](0011-history-tools-search-and-web-ui.md) (code comments), [ADR 0025](0025-comment-replies-and-requested-changes.md)
- Amended by: [ADR 0066](0066-pending-comments-follow-their-text.md) (pending comments follow their text), [ADR 0067](0067-the-expanded-review.md) (how a review is finished)
- Amended by: [ADR 0077](0077-docs-and-sheets-edited-together.md): comments can target a doc's text or a sheet's cells
- Amended by: [ADR 0068](0068-artifacts.md): Artifacts: what an agent shows the person, beside the thread

## Context
The changes panel took a comment on a line, kept it, and could send it to the agent at once, or send every open comment. People reviewing an agent's work want:
- comments that wait until the review is done, so the agent gets one prompt rather than a turn per comment;
- a note with the review as a whole;
- comments on the lines the agent deleted, not only on what it wrote;
- comments sent once: "send all open comments" sent the same ones again each time.

## Decision
**A review is kept in the browser until it's sent.**
- Its pending comments and note are this browser's, per thread (`localStorage`), and only it shows them, marked Pending, with edit and delete.
- Finish review asks for an optional note. Each pending comment then becomes a comment on the thread, and the note, every comment and the lines it's on, quoted from the diff, go to the agent as one prompt.
- Open comments not sent yet, such as one added on its own, go with the review.
- With no agent running from Silt, as when Claude Code runs in a terminal, the review is copied to paste there instead, or kept as comments for later.
- A review in progress doesn't follow the person to another machine. Its comments are synced once it's sent; until then it's a draft, like a prompt not sent yet.

**Sent is recorded.**
- `CommentOp::Sent { comments }` is appended to `CommentOp`; it says the comments went to the thread's agent.
- A sent comment stays open until someone resolves it, once the agent has dealt with it, and isn't sent again. The CLI and the web UI say "sent". A merge request's review sends what wasn't sent yet, and records it the same way.

**Deleted lines take comments.**
- `CommentOn::Lines` takes a `base`: the diff base the lines are numbered in, such as `last-turn` or `start`. The comment's anchors are made on the text as it was at that version. Nothing new is stored: the anchors hold deleted characters, which resolve as outdated at the thread's head.
- Listing comments with a `base` says where each code comment is at that version too (`before`), where its lines are there. A diff shows a comment whose lines are deleted at the head beside its lines in the base.
- A base that is a git commit, such as `main`, isn't a version of the thread, so its deleted lines take no comments.

**Compatibility.**
- The new variant goes at the end of `CommentOp`, so stored logs stay readable (ADR 0003).
- An older peer can't decode it, so `silt_proto::PROTOCOL` goes to 8.

## Consequences
- The gutter shows a "+" on a line's hover, and a drag from it takes a range; shift-click on a number still extends one.
- `silt comment --base <base>` comments on deleted lines; `silt comments --base <base>` says where comments on deleted lines were.
- A pending comment was numbered as the diff showed it, so it could land beside other lines if the agent changed the file before the review was sent. ADR 0066 has it hold anchors from when it was written instead, and follow its text.
- Every daemon and the server update together across the protocol bump.
