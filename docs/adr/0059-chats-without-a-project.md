# 0059: Chats without a project

- Status: accepted
- Date: 2026-11-03
- Amends: [ADR 0023](0023-projects.md) (a reserved project), [ADR 0044](0044-the-standard-agent-workspace.md) (the sidebar), [ADR 0058](0058-new-threads-settings-outside-the-composer.md) (New thread's layout, which New chat narrows), [ADR 0053](0053-projects-without-a-remote.md) (what names a project)
- Amended by: [ADR 0062](0062-the-composer-plus-and-enter.md) (⏎ starts a chat, and Start in the background moves to +)
- Amended by: [ADR 0065](0065-workspaces.md): Workspaces: a server, defaults, rules and history apart for each kind of work

## Context
People ask their agents things that belong to no repository: which sensor or motor driver to buy, how a protocol works, a quick script to rename some photos. Silt starts every thread in a project, so today such a question means picking an unrelated project or making a folder, which then shows as a project of its own forever (ADR 0053).

What exists today:
- `silt claude` run outside any git repository records a thread with an empty project. Its conversation is kept and searchable, but `silt resume` refuses it ("ran outside any git clone, so there's no code to continue it from"), and the web UI shows its folder as its project.
- New thread's picker offers only projects, and `add_clone` refuses a folder without git.
- Resume and fork need a base commit (`GitOp::Genesis`) that the other machine can get: from origin, the server's git mirror, or the family's bundles.

A chat needs a folder for its agent to work in, files that travel with the thread to the person's other machines, and a way to move into a project later. The cleanest move would turn the chat's folder itself into the project, so its threads and cwd stay where they are.

## Decision
**A chat is a thread in the reserved project `silt:chats`.** It's a thread like any other, with its own family, so search, history across compaction, forks, handoff and the inbox work unchanged. Only where its files live and how it's shown differ.

**Each chat is its own small repository.**
- Its folder is `<chats dir>/<date>-<first words>-<six random hex digits>/`, made with `git init` and `silt.project = silt:chats` in its local config. `Repo::project` reads that key before origin, so the folder names the reserved project wherever it lives. The key is local config, never read from a delta.
- Its first commit is an empty root commit with fixed author, committer, date and message. Its id is therefore the same on every machine, and it is every chat's genesis: resume and fork elsewhere make the folder and the same commit, and need no origin, mirror or bundle. Later commits the agent makes travel in bundles as for any project without a remote.
- The agent's cwd is that folder, worked in Local. What it writes there is recorded as the thread's code, so files travel with handoff and forks.
- One folder per chat; chats share no scratch space. A note meant for later goes in a chat of its own, or a project.
- The chats dir is hidden by default, at `$SILT_HOME/chats/`, so it adds nothing to the person's home. Settings › Chats sets another folder (such as `~/Silt/Chats`), kept with the default permissions in `chats.json` in the data dir. Silt refuses one inside a git repository that would take in the chats' files; one the repository ignores is fine.

**Chats are private and follow the person.**
- A new chat is `Visibility::Private` and syncs from its first turn to the person's default server, so it can be picked up on their other machines and phone at once; on a machine with no server it stays on the machine (ADR 0046). Share works as for any thread.
- The server never places `silt:chats` in an org, so it's in no `GET /v1/projects` and org members see none of its threads; a chat registered without a visibility is private. Moving the project, its settings and its visibility are refused: each chat has its own.

**In the web UI, chats have their own button, view and section.**
- The sidebar has **New chat** directly below New thread, and a **Chats** section above the projects listing chats newest first.
- New chat opens `/chat`, a simplified New thread: "What do you want to know?" over the same `Composer`, with only what a chat needs. The heading has no project picker and there is no segmented control under the composer (ADR 0058):
  - in the box: attach, the agent and model, and the permission mode;
  - under it, a line naming the device it runs on, and that it's private;
  - the start button with Start and Start in the background.
- It leaves out the project picker, Local, Worktree and Cloud, the base branch, Start on another device and Save as a draft: a chat is always its own folder worked in Local, Cloud needs a repository, and a draft belongs to a project's queue.
- The permission mode defaults to **Full access**, as a chat's folder holds nothing but what its agent wrote there. The person can pick another mode per chat, and Settings holds the default for chats apart from projects'.
- A running chat is a Thread page as in ADR 0044. Its header names Chats rather than a project, and no branch; its More menu adds Make it a project… and Continue in a project…. The right panel's first tab is **Files**: everything since the chat's empty first commit, so its folder's files.
- Keys: `c` opens New chat, and the palette has it.
- Hosted mode lists chats in the same section; starting one there needs a device, as for any thread.

**From the command line:** `silt chat [agent] [-- args…]` makes a chat and runs the agent CLI in it (Claude Code by default), recording as `silt claude` does. `--about` names its folder. `silt claude` run outside a repository still records, as it did, and says that nothing can continue such a thread elsewhere and that `silt chat` would.

**A chat can become a project, in place.** **Make it a project…** in the thread's menu: Silt removes `silt.project` from the chat's folder, which is then a project without a remote named by its folder (ADR 0053), and moves the threads recorded in that folder to it. The cwd, the family and its history stay; the person can add a remote later and the threads follow again. It's refused while an agent is working in the chat. A chat syncs from the start, so its server moves it too: `POST /v1/threads/{id}/project` takes the owner's chat into the project, which joins their org as any new project does. Other machines hear of it in the listing and move their record of the thread.

**Or continue in an existing project.** **Continue in a project…** starts a new thread there, primed with the chat's transcript (within `PRIMED_BUDGET`, as translating between agents is), and offers to copy the chat's files, without its git, into a folder of that checkout, a relative path of plain names. The new thread's first prompt names the chat it continues. It can't be a fork: a family's log can't take another family's tree.

**Old threads with an empty project**, recorded outside any repository, stay as they are, listed under their folder. Showing them under Chats is left for later.

## Consequences
- `Repo::project` gains a source of truth other than remotes and folders; `prepare_thread` compares the reserved name like any other, so a chat resumes only in a chats repository.
- A reserved project name reaches the server and other machines, and an older server would place it in an org, where the org would see every chat: `PROTOCOL` bumps to 10, so a chat never reaches a server that doesn't keep it private.
- `silt:` can't start a normalized remote (no scheme or host normalizes to it), so the name can't collide with a repository.
- A repository per chat costs a `.git` of a few kilobytes each, and keeps chats apart: one chat's commits, `gc` or promotion never touch another's.
- Watchers cover the chats dir like any checkout; on WSL it must not be under `/mnt/<drive>`.
- Full access by default means a chat's agent can run any command and reach the network without asking. Its cwd is its own folder, but it isn't sandboxed: it can still write elsewhere in the person's home, as any agent on Full access can.
- The chat view and the thread view share `Composer`; the chat's options are a narrower set of the same controls rather than a component of their own, so the two don't drift.
- A chat made a project on one machine keeps its folder a chat's repository on the others, where its threads now name a folder project they have no clone of, as ADR 0053 says of any project named by a path.
- The screen references gain New chat, and every screen's sidebar its New chat row; the guide's web UI page gains a section on chats.
