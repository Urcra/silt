# 0030: Commits and the turns that made them

- Status: accepted
- Date: 2026-10-21

## Context
A thread's checkout is a git clone, and people or their agents commit in it. The daemon already records each new `HEAD` of a checkout as a `GitOp::Committed` delta on the thread (ADR 0002). It pushes commits that origin lacks to the server's git mirror (ADR 0009). What's missing is showing them: which turn made a commit, the commits on a thread's page, and commit nodes in the history graph. The plan also asks whether Silt should add a `Silt-Thread:` trailer to commits.

## Decision
**A commit belongs to the turn it came in.** That's the last turn of the thread whose prompt was submitted before the commit's `Committed` delta, in the family's log order. A commit before any turn belongs to none. The link is worked out from the log, so no new op is needed, and old threads get it too.

**What's shown, and where it comes from.**
- **The thread page** lists the commits from this thread: the short id, the subject, the turn, and when.
- **The history graph** draws them as nodes on their thread's lane.
- **Subjects:**
  - a daemon reads the subject and author from the checkout's repository;
  - the server reads them from its git mirror, which has the commits origin lacked;
  - without either, the short id stands alone.

**No trailers.** Silt doesn't change commit messages: they're the person's and their agent's, and a trailer would follow the commit into reviews and history that have nothing to do with Silt. The link lives in the log instead. `silt log --commit <id>` finds the thread and turn that made a commit, as far as the person may see.

## Consequences
- The fold that ties commits to turns is a pure function in `silt-log`, shared by the daemon and the server, like the other folds.
- A commit made outside any checkout Silt knows isn't linked, because nothing records it.
- A thread whose checkout was rewound (ADR 0029) keeps its later commits in the list. They are marked as after the rewind, since git still has them.
