# 0023: Projects as first-class records

- Status: accepted
- Date: 2026-10-21
- Amended by: [ADR 0044](0044-the-standard-agent-workspace.md): The standard agent workspace: open on the composer, one shell for every screen
- Amended by: [ADR 0053](0053-projects-without-a-remote.md): Projects without a remote
- Amended by: [ADR 0059](0059-chats-without-a-project.md): Chats without a project
- Amended by: [ADR 0065](0065-workspaces.md): Workspaces: a server, defaults, rules and history apart for each kind of work

## Context
A project is a repository, named by its normalized remote, and it belongs to one org ([ADR 0013](0013-accounts-orgs-and-self-hosting.md)).

**What exists today.**
- The server keeps one row per project: its repository, org and visibility, made when a thread of it first registers.
- A daemon remembers one clone path per repository, from `silt open` and recorded sessions.
- Nothing lists the projects of an org, nothing holds what a new thread in one should start with, and nothing helps get a project onto a machine that has no clone of it.

**What the web UI needs** ([ADR 0020](0020-next-generation-web-ui.md), plan section E):
- the projects in the sidebar, with counts;
- AddProject: an existing clone, a clone of an org project, or a clone of a remote URL;
- NewThread preselecting what the project says.

## Decision
**The project record.** The server's `projects` row gains:
- `name`: shown in the UI; by default the remote's last path segment;
- `origin_url`: the fetch URL as the registering daemon saw it, with any credentials stripped, so another machine can clone it with its own git and credentials;
- `default_agent` and `default_preset`: what a new thread starts with, `NULL` meaning no preference.

The defaults are suggestions the UI preselects. They are not policy: a thread's own choices always win, and a daemon never refuses a prompt because of them.

**Listing.**
- `GET /v1/projects` lists, for the caller, every project of their orgs that has a thread they can see or that their org placed.
- Each entry carries its org, name, origin URL, visibility, defaults, and how many of its threads the caller can see.
- The daemon adds the counts it knows better: open merge requests, from its own view of them, and agents waiting on the person here. A count of agents waiting on other devices waits for the control channel (M14).
- Users outside a project's org see nothing of it, as before.

**Changing a project.**
- Name and defaults change through the existing org change request, as `ProjectSettings { repo, name, default_agent, default_preset }`, appended to `OrgChange`.
- The rights are those of moving a project: org owners and server admins.

**Clones on a machine.** The daemon's `clones` table stays one path per project: the clone it last saw. AddProject offers three ways in:
- **Add an existing clone:** a path, checked to be a git repository with an origin.
- **Clone an org project:** `git clone <origin_url> <path>`, run with the person's git, then remembered.
- **Clone a remote URL:** the same, for a project the server doesn't know yet. It registers when its first thread does.

Discovering clones on its own (scanning folders) is left out. It reads the disk widely, it's slow on large homes, and AddProject's path field covers the need.

**Repositories without a remote.** These are "this machine only" projects. They are part of local-only workspaces (M17), not this ADR. [ADR 0053](0053-projects-without-a-remote.md) names them by their folder.

## Consequences
- A migration adds the new columns. Existing projects get a name from their remote, and their origin URL once a daemon registers a thread again.
- The UI gains a projects API on the daemon, which proxies `GET /v1/projects` and merges in this machine's clones. Offline, it lists the clones alone.
- A stored origin URL can reveal a host name the org uses. It's visible only to the org's members, who see the project anyway.
