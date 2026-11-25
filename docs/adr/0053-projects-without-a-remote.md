# 0053: Projects without a remote

- Status: accepted
- Date: 2026-10-30
- Amends: [ADR 0023](0023-projects.md) (repositories without a remote), [ADR 0044](0044-the-standard-agent-workspace.md) (the project picker)
- Amended by: [ADR 0059](0059-chats-without-a-project.md): Chats without a project

## Context
A project was named by its clone's `origin`, as `silt_git::normalize_remote` reduces it. A repository without one had no project:
- the web UI refused to add it as a project ("has no origin remote"), so New thread couldn't start in it;
- a thread recorded in one, by `silt claude` or an import, had an empty project, which every such repository shared.

So someone starting something new had to make a folder, `git init` it and add a remote somewhere before Silt would start a thread in it. People would rather leave the repository, its remote and its first commit to the agent they work with.

## Decision
**A repository without a remote is the project of its folder.** `Repo::project` names a project by its origin as before, and without one by the repository's own folder, reduced the same way (`/home/me/code/app`; a Windows drive's path as WSL names it). That is also how a clone of that folder names it, as its origin is the folder. The project's name is the folder's. A managed checkout's project is its source's.

**New threads of such a project stay on this machine** unless the project was given a home: no other machine could clone it. The Projects page says so, and the home can be changed as for any project ([ADR 0032](0032-several-servers-and-local-projects.md)). A repository whose origin is a folder (another clone, a bare repository, a Windows drive from WSL) keeps the default server, as it always has.

**Threads follow their clone to the project it names now.** When a repository without a remote gains one, its project changes to the remote's. The threads recorded in it move with it, as do its clone and the home given its old project, when:
- the daemon starts;
- the daemon first sees the clone under its new name, such as when a thread starts there.

The same move gives threads recorded with an empty project the project of their folder. Threads that sync with a server stay where they are, as others hold them under that project there.

**New thread can start a project from nothing.** The picker's **New** tab, and **New project** on the first open, take a name and the folder to put it in. The daemon makes the folder, with an empty repository (`git init`), and adds it as a project (`POST /api/projects/new`). Silt records on top of git, so the repository is the least it needs. A first commit and a remote are left to the person and their agent. Until the first commit, a thread works in the clone itself (Local), as a worktree starts from a commit. The folder's parent must exist, and the folder must not exist or must be empty, as for a clone.

## Consequences
- A folder that is moved or renamed is a new project, as its path is its name. Its old threads keep the old project.
- A project named by a path stays on its machine by default. Moving it to a server shares its threads, but other machines can't resume them without a clone.
- A clone of the folder, with the folder as its origin, is the same project, and stays so after the folder gains a remote. Its threads stay under the folder's project, apart from the folder's own.
