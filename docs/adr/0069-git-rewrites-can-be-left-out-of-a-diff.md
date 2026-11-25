# 0069: What git rewrote can be left out of a diff

- Status: accepted
- Date: 2026-11-14
- Amended by: [ADR 0089](0089-a-checkout-per-thread-in-a-shared-clone.md): A checkout per thread in a shared clone

## Context
A thread's diff compares its files at a base with its files now. When the agent runs git in a tool call (`git pull`, `git checkout other`, `git stash` then `git stash pop`, `git rebase`), git rewrites the worktree, and capture records those rewrites like any other edit, credited to that tool call. A person running git in the clone between turns has the same effect, credited to the person. A pull can bring in hundreds of files. The agent's own edits are then lost among them in the changes panel, a turn's changes and merge requests.

Telling git's edits apart from the command line is unreliable. `git pull && cargo fmt` is one tool call, and git can run inside a script. What git leaves behind is reliable: HEAD and its reflog, the stash, and the files git keeps while an operation waits on conflicts.

## Decision
**A tool call whose run moved git's refs is recorded as git's.**
- When a tool call starts, the capture step keeps the clone's git state in the store (`tool_git_states`): HEAD, how long HEAD's reflog is, `refs/stash`, and `MERGE_HEAD`, `CHERRY_PICK_HEAD` and `REVERT_HEAD`. When the call ends, or the turn stops with it open, the step compares the state again. This is the shared step, so a hook's process that sees the start and another that sees the end agree (ADR 0018).
- Git rewrote the worktree when the stash moved, when one of those heads appeared or changed, or when HEAD moved and the reflog lines added meanwhile include one not written by `commit`. A checkout, pull, merge, rebase, reset, cherry-pick or revert writes such a line. A commit, an amend or the end of a merge doesn't, as those commit files the agent already wrote. Without a reflog, a move counts unless the new HEAD's parent is the old one.
- `git stash apply` moves no ref. So when the stash held something as the call started and nothing moved, the step also asks whether every file the call edited now has a stash entry's content, and not HEAD's (`Repo::applied_from_stash`). If so, the call was an apply.
- The step then records `GitOp::Rewrote { session, tool_use_id }`. The edits credited to that call are git's (`silt_log::GitMade`).

**Git run between two looks is recorded as git's too.**
- Every rescan of a checkout keeps the git state it found, and the thread's head then, in the store (`checkout_git_states`, per checkout and thread). Rescans happen at every turn's start and end and after every tool that isn't a file tool, in the shared step.
- When the next rescan finds that git rewrote the worktree since, by the same rule, the edits recorded as the person's after that head are git's: `GitOp::Made { spans }` lists them. This covers a `git pull` the person runs between turns, and git they run in the clone while the agent is away (ADR 0047).

These two ops bump the protocol to 11.

**A diff can leave git's edits out.**
- `silt diff --hide-git`, `silt mr show --hide-git`, and `git=hide` on the daemon's `/api/threads/{id}/diff`, `/diff/stats` and `/api/mrs/{id}`, and on the server's `/app/api/mrs/{id}`, compare both sides without git's edits. The CRDT reads text, files and the tree at a set of edits that needn't be causally closed (`TextDoc::text_where`, `Family::file_where`). An edit the agent made on top of lines git brought in still shows, as an insertion.
- Against a branch (`main`, `branch:<name>`), the diff is git's own already, and the flag doesn't apply.
- In the web UI, **Hide git's changes** sits beside the changes panel's compare menu and on a merge request's Changes tab. It's off until ticked, then kept in the browser and used by every diff there, the turns' too.
- Line comments still work with it on. A code comment is a pair of CRDT anchors, character ids with a bias, so a comment made on lines of the diff without git's edits is anchored there (`TextDoc::anchor_where`), and placed in either diff (`TextDoc::resolve_where`). `CommentOn::Lines` and `GET /api/threads/{id}/comments` take `hide_git`, and `silt comment` and `silt comments` take `--hide-git`. A review's drafts keep which diff they were made in.

## Consequences
- Git's edits are left out only for tool calls recorded from protocol 11 on, so threads recorded earlier show them as before.
- With git's changes hidden, the diff no longer matches the disk. An agent that drops its own edit with `git stash` still shows it, as the stash's edits are git's. `git restore` and `git checkout -- <file>` move no ref, so the edit they undo is gone from the diff either way.
- A tool call that runs git and also edits files, such as `git pull && cargo fmt`, has all its edits treated as git's.
- Edits the person makes by hand between the same two looks as a git command of theirs are treated as git's too, like a mixed tool call.
- A stash apply that the agent follows with an edit of the same file in the same call isn't seen, as not every file the call edited matches the stash. Nor is a `git restore --source=<commit>`, which moves no ref.
- The first rescan of a checkout after updating has nothing to compare with, so git run before it isn't seen.
- A file that git created and the agent then edited is left out whole, as its creation was git's.
- A clone's worktrees share `refs/stash`, so a stash in one managed checkout marks a tool call running at that moment in another as git's too.
