# 0029: Forking from and reverting to a turn

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0010](0010-forks-pulls-and-merge-requests.md), where a fork starts at its source's head.

## Context
An agent's turn sometimes goes wrong several turns back. Today a person can fork the thread as it is now, or undo by hand. The plan for the web UI asks for two more ways back:
- forking from an earlier turn, to try something else from there;
- reverting a thread to a turn: code and conversation.

The log keeps every version, and a fork is a new head (ADR 0001, ADR 0010). So both can be made of deltas without deleting anything.

## Decision
**A turn's point.** Turn N ends where turn N+1 begins, or at the thread's head when N is its last turn. A turn begins at its prompt-submitted mark or at the line holding its prompt, whichever the log took first, since agents record them in either order. Turn N's point is the thread's head as of that moment in the family's log: `thread_head_in` over the log up to there. That's the code and conversation the person saw when turn N was done.

**Forking from a turn.**
- The fork's `Op::Fork` delta has the turn's point as its parents, rather than the source's head.
- **Conversation:** what the fork inherits is the source's history up to its fork point, as for any fork (ADR 0010). It holds turns 1 to N and nothing after.
- **Agent:** a fork's restored session is made from that history, so the agent picks up from turn N.
- `silt fork --turn N`, and "Fork from here" on a turn in the web UI.

**Reverting to a turn.** Reverting rewinds the thread itself, with new deltas only.
- **Files:** each file that differs between the turn's point and now gets an op that sets it back to its content then. That's a create for one deleted since, a delete for one created since, and a move for one renamed. Edits made since stay in the history.
- **Conversation:** a `ConvOp::Rewound { to }` delta marks every conversation delta of the thread after `to`, the turn's last one, as rewound.
  - Views show rewound turns greyed out, after a note.
  - Search still finds them.
  - Nothing reads them as the conversation any more: the history the agent sees, a restored session, and a fork of the thread from here on.
- **Agent:**
  - a revert waits for the thread's agent on this machine to be stopped, so no turn runs across it;
  - an agent started from the web UI afterwards starts a new session primed with the conversation up to the turn, rather than loading the old one;
  - a session resumed later is restored from the history without the rewound part.
- **Repeating:** a later revert to a turn before `to` rewinds further, and a revert never brings rewound turns back. To return to them, fork from the last of them; a fork from a rewound turn sees it.
- `silt revert --turn N`, and "Revert to here" on a turn.

## Consequences
- **New op:** `ConvOp::Rewound`. Older daemons can't read it, so the sync protocol goes to version 5 and they are told to upgrade.
- **Restoring:** session restore and the history readers skip rewound deltas, so both need the mark's position in the log.
- **Merging:** a revert that a collaborator's concurrent work crosses merges like any other edits. The CRDT keeps both, and the file can end up with their edit on top of the reverted text.
- **Commits stay:** git commits made after the turn are untouched. The checkout's files go back, and the next commit records that.
