# 0025: Comment replies and requested changes

- Status: accepted
- Date: 2026-10-21
- Amended by: [ADR 0055](0055-reviewing-a-threads-changes.md): Reviewing a thread's changes

## Context
Comments are deltas (`CommentOp::Added`, `CommentOp::Resolved`, [ADR 0011](0011-history-tools-search-and-web-ui.md)), and merge requests are opened, approved or declined (`MrOp`, [ADR 0010](0010-forks-pulls-and-merge-requests.md)). Review needs two more things:
- **a conversation under a comment:** today a reply is a new comment beside the first;
- **a verdict between approving and declining:** "change this, then I'll take it", which keeps the merge request open.

## Decision
**Replies.**
- `CommentOp::Replied { parent, text, by }` is appended to `CommentOp`.
- `parent` is the comment's id. A reply to a reply names the first comment, so each comment has one flat list of replies, in log order.
- A reply has no target of its own: it sits where its comment does, and an outdated comment's replies are outdated with it.
- Resolving the comment resolves the conversation. A reply after that doesn't reopen it, but it shows.
- The inbox tells the comment's author, and everyone who replied before, of a new reply, like a comment on their thread.

**Requested changes.**
- `MrOp::ChangesRequested { mr, reason }` is appended to `MrOp`.
- The merge request stays open with the status `changes_requested` and the reason. It's shown with "updated since" once the fork's head moves past the verdict.
- Approving or declining afterwards works as before. A second request for changes replaces the reason shown, and every verdict stays in history.
- The fork's owner is notified, and the fork's agent hears the reason in its next prompt, as it hears of other review events today.

**Change guide.** *Amended 2026-10-21.*
- `MrOp::Guided { mr, guide, by }` is appended to `MrOp`, on the fork. The latest guide shows with the request, and earlier ones stay in history.
- A person writes it, or asks the agent running in the fork on their machine. That agent gets a prompt asking for a walk-through without changes, and its answer becomes the guide once its turn ends well.
- No hosted model writes it (ADR 0020, principle 1). Without an agent in the fork, the person starts one, or writes the guide themselves.
- It comes in the same protocol version as the rest of this ADR, as none of it was released apart.

**Compatibility.**
- The new variants go at the end of their enums, so stored logs stay readable (ADR 0003, ADR 0011).
- An older peer can't decode these deltas, so `silt_proto::PROTOCOL` goes up by one. Mixed versions get the clear "protocol mismatch" error rather than half a review.

## Consequences
- Views gain replies under comments and the new status. The CLI shows replies in `silt comments` and takes `silt comment --reply <id>`, and `silt mr request-changes <id> <reason>` joins approve and decline.
- The server's index and inbox learn the two ops.
- Every daemon and the server must update together across the protocol bump. That is the rule already, and the error says so.
