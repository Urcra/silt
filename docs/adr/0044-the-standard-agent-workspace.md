# 0044: The standard agent workspace: open on the composer, one shell for every screen

- Status: accepted
- Date: 2026-10-23
- Amends: [ADR 0022](0022-agent-runtime.md) (Plan first), [ADR 0023](0023-projects.md) (a new thread's defaults), [ADR 0039](0039-the-web-ui-as-designed.md) (the Threads and New thread boards, the top bar and status bar)
- Amended by: [ADR 0054](0054-the-desktop-ui-review.md) (the sidebar's Needs you)
- Amended by: [ADR 0048](0048-an-agents-modes-are-its-permissions.md): An agent's modes are its permissions
- Amended by: [ADR 0049](0049-the-agents-work-grouped-by-turn-or-by-run.md): The agent's work, grouped by turn or by run
- Amended by: [ADR 0053](0053-projects-without-a-remote.md): Projects without a remote
- Amended by: [ADR 0059](0059-chats-without-a-project.md): Chats without a project
- Amended by: [ADR 0062](0062-the-composer-plus-and-enter.md): The composer: + for what goes with a prompt, ⏎ to send, settings under the box
- Amended by: [ADR 0067](0067-the-expanded-review.md): The expanded review: a thread's changes across the page, seen files and reviewed turns
- Reversed by: [ADR 0114](0114-one-set-of-permissions-and-agents-by-their-marks.md): One set of permissions, and agents shown by their marks
- Amended by: [ADR 0115](0115-the-sidebar-names-the-workspace-and-the-machine.md): The sidebar names the workspace at its top and the machine at its bottom

## Context
Starting a thread should take one step, from the prompt box itself: where it runs, the branch and the permissions are small controls beside it, and the first Enter starts the thread. The sidebar lists projects and their threads, and the thread page is a transcript with the composer docked under it and the diff beside it.

Silt takes three steps for the same thing. `silt open` lands on the Threads dashboard. New thread is a form at `/new` with ten controls before the first word. Only then comes the thread, whose composer follows the transcript. Every screen also has a top bar with breadcrumbs, a status bar, and a title of its own size and place.

A first open has nothing to show either. The folders where a person's agents already ran are on disk, and `silt import` reads them (ADR 0043), but the UI asks for a path instead.

## Decision
**`silt open` lands on an empty new thread in the project it ran in.**
- `/` is an empty thread: "What should we work on in `<project>`?" over the composer, with the project name as the picker. `/new` redirects there.
- When the cwd is a known clone, `open_web` puts the project in the address (`/?project=<clone>`). The desktop app gets the same through `silt open --no-browser`.
- The dashboard moves to `/threads` as All threads, a list page. Inbox stays notifications only.
- In hosted mode (`/app/`), `/` stays All threads, since no agent starts there.

**One composer, for new threads and running ones.** It sits centred while the thread is empty and docks at the bottom once it runs.
- In the box: attach, the agent and model, and the permission mode.
- Under it: Local, Worktree or Cloud as one radio group, the base branch for Worktree, and the device when there's more than one.
- The start button splits: Start (mod+⏎), Start in the background (mod+⇧+⏎), Start on another device, and Save as a draft, which replaces Stash.
- Visibility moves to the project's defaults and Share. The preset legend becomes the descriptions in the permissions menu.

**Local by default.** "In my clone" is now Local, "Own checkout" is Worktree, and "In the cloud" is Cloud. New threads start in Local unless the project's defaults say otherwise (ADR 0023). Starting a second Local thread in a clone where an agent is working says so and offers Worktree.

**Plan is a permission mode.** The presets become Plan, Ask for everything, Ask before commands and Full access. Plan does what Plan first did (ADR 0022): the agent's plan mode where it has one, else a prompt asking for a plan, and the thread waits on the plan. Approving it moves the thread to the preset the person picks on the plan card. Plan is chosen per thread and never stored as a project default.

**New projects from the picker.** The project picker has four tabs: the projects here, Open a folder (a path, with completion), Clone a repository, and projects in the workspace that aren't on this machine. Picking one adds the project and selects it.
- A folder that isn't a git repository starts in Local mode only. Worktrees, handoff and forks need git; the person can ask the agent to `git init`.

**The first open shows what's on the machine.** With no projects, the composer's place lists the folders where the agents have run sessions, found by `Agent::on_disk` without being asked, each with Start here. Under them are Open a folder, Clone a repository, and an offer to import the sessions found. The offer is a question; nothing is imported until the person says so.

**Keys.** In the desktop app, mod+N starts a thread in the current project and mod+⇧+N opens the picker for another. Browsers keep mod+N for a new window, so the web UI uses `n` and `shift+n`.

**One shell for every screen.**
- No top bar and no status bar on desktop. Each page owns a 48 px header: the title at 15 px, a mono meta line, and its actions on the right, at most one of them amber.
- The sidebar is the same everywhere: New thread, Search (mod+K, the palette), All threads, Merge requests, Inbox, Needs you, then projects with their threads newest first. A working thread shows a spinner, one waiting on the person is amber. The footer holds the device, its running agents and Settings.
- Every page is one of four kinds:
  - **Thread:** the transcript (max 720 px), the composer docked, and a right panel with Changes, Details and Lineage. New thread, a thread, a fork, and Live, which stays its own page in this layout.
  - **Review:** the thread's header, files and diff in the middle, the decision in the right panel. A merge request, Reconcile, a file at a turn.
  - **List:** one column (max 960 px), filter tabs and one primary action. All threads, Inbox, Merge requests, Search; Graph is the same but full width.
  - **Settings:** one page with its own nav: Projects, Devices, Sync, Orgs, Notifications, Appearance. Projects, Add a project, Devices and Orgs move there, and the theme switch with them.
- On a phone the sidebar is the home list, a page fills the screen, and the right panel is a bottom sheet.

**The thread header** shows the title and branch · mode · device, then Open in, Fork, Share, Open merge request and the Changes toggle with its counts. Hand off, Copy resume command, Join live and History go in its overflow menu. The right panel opens on Changes.

**A thread at work** shows its live state: thinking folded with how long it took, tool calls grouped with the running one's output tail, a working line with the elapsed time and tokens, edits as diff cards in the transcript, images inline in both directions, and a header pill and amber composer while the agent waits on the person.

## Consequences
- Getting from `silt open` to a working agent takes one screen instead of three.
- `NewThread.tsx` and the agent dock in `Agent.tsx` become one `Composer`. `Layout.tsx` loses the top bar and the status bar, and every view routes its title through `Page` at one size, which settles where every title sits and how large it is.
- A daemon endpoint lists the folders `Agent::on_disk` finds, grouped by clone, and another imports them, so the first open needs no command line.
- The `plan` preset is a new value wherever presets travel. If one reaches `silt_proto`, `PROTOCOL` bumps.
- Live state needs data the transcript doesn't carry yet: the turn's elapsed time and tokens, a running command's output, and the images agents return.
- The screen references for every screen change, and `pnpm e2e:update` rewrites them. The guide's web UI names change with them (`docs/guide/web-ui.md`).
- Local by default means two agents started in one clone share its working tree unless one is moved to a Worktree.
