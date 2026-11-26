# The web UI

`silt open` opens the web UI of your own machine on a new thread, in the project you ran it in. From there you start agents and work with them, and find your threads, their history and files, merge requests, search and the inbox. The server serves the same app for browsers and phones, see [the hosted app](hosted-app-and-phones.md), and your machine serves it to phones on your network with no server ([a phone on your network](hosted-app-and-phones.md#a-phone-on-your-network-with-no-server)).

With the desktop app installed (`silt-desktop` beside `silt` or on your `PATH`, which `just install` sets up from a clone), `silt open` shows the project there instead of in your browser; an app already open switches to it. `silt open --no-browser` prints the address instead.

## The first time
With no projects yet, the new thread lists the folders on this machine where your agents have run sessions, found without asking. **Start here** adds one as a project. You can also start a new project, open another folder or clone a repository. Silt offers to import the sessions it found, as `silt import` does; nothing is imported until you say so.

A folder that isn't a git repository is listed but can't start threads yet: ask an agent to `git init` it first.

## The sidebar
Every page shares the sidebar:
- **New thread** (`n`), **New chat** (`c`), **Search** (⌘K, the palette), **All threads**, **Merge requests** and **Inbox**.
- **Needs you:** an agent waiting on a permission or a plan, a merge request waiting for your review, or a mention. Answer a permission or approve a plan right there.
- **Pinned.**
- **Chats:** your chats, in the same order as a project's threads (see [chats](#chats)).
- **Projects:** each project's threads. Those with activity in the last five minutes, or an agent at work, come first by title, so agents answering in turn don't swap places; the rest follow, newest first. Pinned threads are ordered the same way. A spinner marks an agent at work; amber marks one waiting on you. Snoozed and settled threads, and those untouched for two weeks, are behind a "Show" link under the project. Projects with nothing newer than that fold into one **Older projects** row at the end. Anything running or waiting on you stays in view however old. A project's `+` starts a thread in it.
- At the bottom, this machine and its running agents, and **Settings**.

What changed since you last looked is marked unread.

Right-click a thread in the sidebar or on All threads for its menu; its **⋯** on hover, or a long press on a phone, opens the same one. From there, or the palette:
- **Pin** a thread to keep it at the top.
- **Snooze** it for an hour, until tomorrow, or until something happens in it.
- **Settle** it once it needs nothing more.

These are yours alone, and follow you to your other machines through the server. The same menu has **Copy thread ID**, the id `silt resume` and `silt fork` take in a terminal, **Fork**, **History graph** and **Join live**.

The top of the sidebar is the workspace you're looking at: its name and its server. Pick it to narrow everything to one server, one org, or this machine (see [several servers](servers.md)); its menu also shows which account you're signed in as on each server.

The bottom of the sidebar is the machine this window shows, and how it stands: `Synced · 2 running`, or `Sync failing` when it can't reach its server. Its menu opens **Devices and sync** and, in the desktop app, your other machines. An update in progress shows just above it. A thread another of your machines holds says so: `On homelab · 2d`.

## The palette
⌘K (Ctrl+K elsewhere) opens the command palette. It holds actions, threads and history, and shows each action's shortcut.

## Starting a thread
The new thread asks "What should we work on in *project*?". The project's name is the picker: the projects on this machine, a new project, a folder to open, a repository to clone, or one of your workspace's projects to clone here. `n` starts in the current project; `shift+n` opens the picker. In the picker, type a project's first letter to move to it, and type the letter again to move on to the next project that starts with it.

**New** makes a project from nothing: give it a name and the folder to put it in, and Silt makes the folder with an empty git repository. Its first commit and its remote are up to you and the agent. A repository without a remote is named by its folder, and its threads stay on this machine; once it has a remote, its threads move to the remote's project ([ADR 0053](../adr/0053-projects-without-a-remote.md)).

In the composer:
- **Agent:** any ACP agent ([which agents](live-and-agents.md)), and its **mode**.
- **Permissions:** one of the presets below.

Under it, where the agent works:
- **Local**, the default: in your clone as it is.
- **Worktree:** a checkout of its own under `.silt/checkouts`, from a base branch.
- **Cloud:** a Claude Code cloud session (see [cloud sessions](cloud.md)).

The project can suggest the agent and permissions (`silt project set`). The agent's model and thinking level are offered as it last ran. **Commands:** typing `/` at the start offers the agent's own commands, such as Claude Code's and your skills, or Copilot's, as it lists them over ACP: ↑ and ↓ move, ⏎ or Tab fills one in, Esc closes the list. The prompt goes to the agent as you wrote it, which runs the command. ⏎ starts the thread and opens it; ⇧⏎ starts a new line. The composer's **+** attaches files, offers the commands, starts the thread in the background, or saves the prompt as a draft. On a touch screen, where ⏎ starts a new line, an arrow starts it. Drafts and the project's recent threads are listed under the composer to pick up. Every composer grows with what you write, up to half the page, and keeps it when you go elsewhere and come back, in that browser tab ([ADR 0063](../adr/0063-the-composer-grows-and-keeps-its-draft.md)).

## Chats
A chat is a thread that belongs to no project: a question, some research, a quick script. **New chat** (`c`) asks "What do you want to know?" over the same composer as New thread, with only what a chat needs: attachments, the agent and its model, and the permissions. There's no project, no Local, Worktree or Cloud, and no branch.

- **Its folder.** Each chat works in a folder of its own, a small git repository under `chats/` in Silt's data directory, where the agent can save what it finds. The chat's **Files** panel shows them. Settings › Chats can put new chats' folders somewhere you browse, outside any repository that would take them in.
- **Private, and on your other devices.** A chat is visible only to you until you share it, and it syncs to your server like any thread, with its files: **Resume** on another machine makes a folder there with them, and **Fork** gives the fork a folder of its own.
- **Full access.** A chat starts with full access, as its folder holds only what its agent wrote. Settings › Chats changes the default; each chat can pick another.
- **Into a project.** Right-click the chat in the sidebar for **Move to a project…**: **Make it a project** makes the chat's folder a project of its own, with the chat in it, and **Continue in a project** starts a thread in one of your projects from the chat's conversation, with its files copied to a folder there.

From a terminal, `silt chat` runs Claude Code (or another agent CLI, `silt chat codex`) in a new chat; `--about` names its folder. `silt claude` run outside any repository says so and points to `silt chat`. The design is in [ADR 0059](../adr/0059-chats-without-a-project.md).

## Working with an agent
On a thread's page the composer sits under the conversation, one line tall with **+** and the microphone under it, and the agent's work shows as it happens: its thinking, folded with how long it took; its tool calls, folded in one card that says what they did ("Searched 1 pattern, read 2 files"), with the running one's output; its edits as diffs; and how long the turn has run, with how much of its context it uses when the agent says.

Commands stand on their own, as `$ command` with the last lines of their output, whether they failed and with what exit code, how many tests passed, and how long they took. A question the agent asked shows the answer taken, with its choices a click away, and its plan shows each step ticked off. Images go both ways: those pasted into a prompt show under it, and those a tool made under the tool, each with its type and size and opening at full size. The turn bar adds up the tokens the turn took. Times are when the agent says things happened, where it stamps its transcript (Claude Code, Codex), so an imported session keeps its own durations.

**Subagents and background commands.** A subagent, or a command the agent left running in the background, is a card of its own that says how it stands, and what still runs is listed above the composer. While the agent runs on this machine, **Stop** on a running command's card or line ends the command and what it started; the agent then hears it ended, and Silt keeps it from going on about it. A command Copilot left running is marked done once its process is gone, as Copilot may never say so. What an agent the web UI started leaves running ends with it: when you stop the agent, when the daemon stops, and after the machine went down, its threads say so (ADR 0057).

**Grouping and detail.** The agent's work between two things it says to you is a run: one closed line that counts what it did ("Edited 2 files, ran 3 commands · thought for 12s"), with its thoughts, commands and edits inside. A question, a plan and a failed command stand on their own. Settings › Text and layout sets, for this machine, how the work is grouped, and so does the palette: **By turn**, the default, folds a finished turn under one "Worked for" line and leaves its reply, and **By run** shows every run. It also sets how much each step shows: **Normal** keeps them closed and **Verbose** opens every fold. **Shift+T** opens every thought, and closes them again.

**Messages.** Hovering a message copies it. On a prompt after the first, the pencil forks the thread from the turn before it and puts the prompt in the fork's composer, to edit and send again.

**Changes.** The panel's toggle in the header counts what the thread changed, and opens the changes beside the conversation; **Expand** in the panel's header shows them across the page. On a fork, the header has **Offer as merge request**. Details names the branch the thread's latest commit went on, beside its commits.

**Prompting.** ⏎ sends and ⇧⏎ starts a new line. There's no send arrow, except on a touch screen, where ⏎ starts a new line and the arrow sends.
- **Send** prompts the agent now.
- While it works, **Queue** (⏎) holds your prompt until it's ready. Queued prompts can be edited, reordered and removed until they're sent.
- **Steer now** (⌘⏎, Ctrl+⏎) tells the agent your prompt at once. An agent that can take it mid-turn (Codex) hears it in the turn it runs, and the command it's running goes on. Any other's turn is cancelled, which ends what it was running, and your prompt is sent next.
- **Stop the turn** (Esc, or the square beside the microphone), or **Stop agent** at the bottom of the agent's menu, its logo under the composer.

**Attachments.** Paste an image or file into either composer, drop it on one, or pick it from the composer's **+**; New thread sends it with the first prompt. Any file goes: images, text and PDFs go in the prompt as far as the agent takes them, and anything else, such as a spreadsheet, a Word document or a long log, is saved on the agent's machine under `$SILT_HOME/attachments/` and the agent is told where. Files have no size limit: each uploads to the machine as you attach it, with its progress on its chip, and the prompt goes once they're all there. A video goes to the agent as a file it can read, as no agent takes video in a prompt, and an image larger than 2048 px goes as a 2048 px copy. They're kept with the thread, and images and videos show in your message.

**Dictation.** The microphone in either composer dictates, on the machine running Silt; so does holding Ctrl+M (Ctrl on a Mac too) while the composer has the focus. What it hears shows in grey at the cursor as you talk, and the row under the composer shows what the microphone hears, with ✕ and ✓. ✓, the microphone again, or letting go of Ctrl+M puts the words into your draft there; ⏎ puts them in and sends; Esc or ✕ throws the take away. A tap of Ctrl+M starts a take that another tap ends. If the microphone hears nothing at all, the row says it may be muted. The first time, it asks to download a 103 MB speech model, which stays on the machine, as does your voice: the audio is never stored or synced. Dictation is English only, and needs `silt-voice`, which `just install` installs beside `silt`, and `just install-wsl` installs in WSL and on Windows. In the Linux desktop app it also needs GStreamer's good plugins, `gst-plugins-good` on Arch or `gstreamer1.0-plugins-good` on Debian and Ubuntu ([ADR 0061](../adr/0061-dictation-on-this-machine.md), [ADR 0062](../adr/0062-the-composer-plus-and-enter.md)).

**The row under the box.** Both composers, New thread's and a thread's, show the same row: the agent by its logo, the permissions (Edits), its model with a bar for each level of how hard it thinks (none where the agent decides), and its plan usage. Hover the logo for the agent's name, or open it to pick another; while the agent runs, **Stop agent** is at the bottom of that menu. The model chip opens the agent's models and its effort steps. Model and effort are its own, as it lists them; New thread offers them as the agent last ran on this machine. An agent's own modes, which say how it works rather than what it may do, such as Copilot's Autopilot or the agents you defined in OpenCode, are in the permissions menu under the agent's name, after Full access; Shift+Tab steps through them too. Autopilot asks nothing and keeps going until the work is done; in a mode Silt can't read, every request the agent makes asks you. The agent's default mode and its plan mode don't show: the presets and Plan are them. Choosing another agent in a thread stops the one running there and starts the other with the thread.

**Permissions.** Every agent has the same five presets; ⇧Tab moves to the next. Choosing one moves an agent with modes of its own, such as Claude's, into the matching one (ADR 0114):
- **Plan:** every new prompt gets a plan to approve before the agent changes anything. Approving it picks the preset the thread moves to.
- **Ask for everything.**
- **Ask before commands**, the default: edits, reads and searches are allowed, and anything else asks.
- **Auto:** the agent's own auto mode lets safe actions through and asks about risky ones. Only agents with an auto mode offer it, Claude Code among them.
- **Full access**, for a machine you trust.

When the agent waits on you, the header says so and the composer turns amber.

**Signing an agent in.** When an agent's own sign-in has expired, or was never made, the thread shows "Claude needs you to sign in" instead of an error, with a button for each way the agent offers. The sign-in runs on the machine the agent runs on, without a terminal there:
- Claude shows a link: open it in any browser, sign in, and paste the code its page shows into the card.
- Copilot shows a link and a code to enter on that page.

Once it's signed in, the agent starts again and goes on with the prompt it refused. Prompts sent meanwhile wait for it. A wrong code ends the sign-in, says why, and offers the buttons again. **Cancel** ends it, and stopping the agent drops it. The hosted app and your other devices show the same card, through the server (ADR 0056). What a sign-in shows isn't recorded in the thread.

**What the agent shows you.** An agent the web UI runs can show you an image or a video, which appears in the conversation, and make artifacts: a page, chart, diagram or document, as HTML, SVG, Markdown, Mermaid, an image or a video, which it can update in new versions. An artifact is a card in the conversation; it opens in the panel's **Artifacts** tab, by itself on a desktop when it arrives while you watch.
- **The panel.** Its header says `‹ Artifacts / title v2 ▾`. **Artifacts** goes back to the list and the other tabs, the title picks another artifact, and the version picks an earlier one. Preview or source, reload and zoom follow in the same bar. When the panel is narrow they fold into **⋯**, so nothing covers the page.
- **Full view.** Drag the panel's edge to widen it, or use full view (F) to give it the conversation's space too.
- **Comments.** Select text in the artifact to quote it, write what to change, and **Send to the agent**: the comments go as one prompt asking for a new version. On a phone, the box under an open artifact does the same.
- **Sharing.** **Share** makes a read-only link to the artifact alone (see [sharing](sharing.md)), copies a link to it in the thread, or downloads it, as its file or as one HTML file that opens anywhere.
- **Problems.** If your browser blocks something the page loads, a script in it fails, or it's wider than the panel, a notice under the page says so, and **Send to the agent** asks for a version that fixes it.
- **Images and videos.** Neither has a size limit. An image shows as a small preview, and opens in a viewer over the whole window. There you zoom with the wheel, a pinch, + and − or a double-click, pan by dragging or with the arrows, press 0 to fit it and 1 for actual size, and **Full resolution** (or F) loads the original. A video plays where it is, with play, seek, volume, speed and full screen; its expand button opens it in the viewer with loop and picture in picture too, and **?** lists its keys. A video plays as it arrives, so a long one starts at once. One another machine made shows how much of it has arrived until it's all here.
- **Safety.** An HTML artifact runs in a sandbox that reaches nothing of Silt's. It may load scripts from a few public CDNs (cdnjs, jsDelivr, unpkg, Tailwind's and jQuery's) and fonts from Google Fonts, so opening one can contact those hosts; it can't send anything anywhere else. SVG, Mermaid and images show as images, so nothing in them runs.

Artifacts are kept in the thread's history, so they hand off, fork and sync with it, and are found by their titles in search. [Artifacts](artifacts.md) covers the kinds there are, what a page can do, and the Artifacts page. An agent in a terminal (`silt claude`) isn't offered these tools, as nobody is looking at the web UI for it ([ADR 0068](../adr/0068-artifacts.md)).

**How artifacts look.** Before making one, the agent reads Silt's design guidance and the design system that applies, so its pages work in light and dark, at a phone's width, and look like your project rather than a template ([ADR 0071](../adr/0071-artifact-design-guidance-design-systems-and-libraries.md)).
- **Design systems.** A design system is a folder with a `README.md`, rules for the agent, and a `tokens.json` of colours for each theme, type, spacing and radii. It may also hold its components: a `components.css`, an example of each in `components/` and its guidelines beside it, so a mockup uses the real buttons, rows and cards rather than look-alikes; a cover; and its logos and icons in `assets/`. Silt gives an HTML artifact's page its tokens and components' CSS and follows your light or dark theme in it.
- **Which one applies.** The one the agent asks for, else the one the artifact's last version used, else the repository's default, else the workspace's. A repository keeps its own in `.silt/design-systems/<name>/` and names its default in `.silt/design-systems/default`. Your own go in `design-systems/` in Silt's data folder. Silt ships none of its own: its own look, Nous, is a system in Silt's repository like any project's.
- **When none applies,** the agent takes a mockup's look from your project's own stylesheets and components, and chooses a look from the subject for anything else. A default that names a system this machine doesn't have is passed over, and the agent is told so. Pick a workspace's default in Settings › Workspaces, under "New threads start with".
- **Per project.** A project's row in Settings › Projects can pick a design system for its artifacts on this machine, above the repository's default and the workspace's.
- **Settings › Design systems** lists every design system here: each project's own, each chat's, then yours, with a strip of its colours and what picks it as a default. Open one to see its page: its cover, its components drawn live at a desktop's or a phone's width in light and dark, with their markup and guidelines, its tokens with their values, its rules and its files.
- **Making one.** **New design system** asks for a name and what to build it from: a project's code, a website, a brand guide or screenshots you attach, or a description. Keep it in the project's repository, as part of the thread's changes, or as yours, synced to your devices. An agent builds it in a new thread, with Silt's design-system skill, and the thread's panel shows it as it's written. Asking an agent for one in a thread of your own works too: the panel shows what it writes once it loads the skill.
- **Made in a chat.** A chat's agent keeps a design system as yours, as it's often one for a project you haven't started yet. One a chat keeps in its own folder is listed under that chat, and **Keep as mine**, on its page or beside the chat, copies it to yours, so every project can use it and it syncs.
- **Changing one.** **Change it with an agent** on a system's page starts a new thread for that change, so each change has its own review. The agent reads the system first and changes only what you ask.
- **Your own follow you.** The design systems in your data folder sync through each server you use, so a system made on one machine is there on the others, and one removed is removed everywhere.
- **Each version keeps its copy** of the tokens it was made with, so it looks the same after a handoff, in the hosted app and behind a read-only link.
- **Libraries and fonts through Silt.** Settings › Appearance can load artifacts' libraries and fonts through Silt, which fetches each file once and keeps it, so opening an artifact never contacts a CDN and pages you've opened before work offline. A server has the same setting for the hosted app and its links.
- **The agent checks its pages.** Where Node and Playwright are installed, the agent renders a page at a phone's width and a desktop's, in light and dark, before telling you about it, and reads an earlier version back to revise it after a handoff.

**Questions.** An agent run from the web UI can ask you a question with choices. It waits in the conversation where it was asked: pick a choice, by click or by its number, type your own answer under the choices, which goes to the agent as a steer, or **Skip** it (Esc). The hosted app and your other devices answer it as well, through the server. A question a terminal session asks is answered in the terminal; the thread shows it, and the answer once given.

## A thread's page
- **Header.** The title, then Fork, Share, and the panel's toggle, which opens Changes, Artifacts and Details beside the conversation. The rest is on the thread's right-click in the sidebar.
- **Composer.** With no agent running, pick one from the composer's chip and send: the agent starts in the thread with your draft as its first prompt.
- **Turns.** The conversation, by turn, with what each turn changed. Each turn also has "fork from turn N", "revert to turn N" and "copy link".
- **Changes.** Under the panel's tabs, a toolbar says how many files the diff shows and how much changed, and what it compares: the last turn, the thread's start or `main`. Its view button sets the layout, Unified or Split, and what the diff leaves out. **Hide git's changes** leaves out what git rewrote when the agent, or you between turns, ran a pull, checkout, rebase, reset or stash, here, in the expanded review, in the turns' changes and in merge requests, until you untick it. On a thread another agent worked beside, **Hide other agents' changes** does the same for that agent's edits. Comments made with either on land on the right lines. To review it, the "+" beside a line comments on it, a drag from it takes several lines, and deleted lines take comments too. Add single comment keeps one at once; Start a review keeps comments pending, in this browser. A pending comment follows its text as the agent edits the file; one whose lines are deleted shows apart, with the lines it was on. **Finish review** opens over the diff with a note and asks what the agent gets:
  - **Request changes**: the note and every comment as one prompt, sent now or once the running turn ends;
  - **Request changes, stopping the turn**, while the agent works;
  - **Mark reviewed**, in the expanded review: nothing to change, and the next review starts after this turn;
  - **Keep as comments**.

  With no agent running from Silt, the review is copied to paste into the agent's terminal. A sent comment says Sent and stays open until you resolve it.
- **The expanded review.** ⇧D, or the expand button in the panel's header, opens the changes across the page, at `/t/<id>/review`; Esc or **Conversation** goes back. On the left are the files, with Seen to tick each off as you look at it: a file ticked off comes back half-ticked when the agent changes it again, and the lines new since are marked at their edge. The diff is split, follows the agent's edits as they land, and each hunk names the turn that wrote it, a link to it in the conversation. **Turns** shows one turn's changes, or a range of them with shift-click; it starts after the turn you last marked reviewed. Its keys: j and k for files, n and p for hunks, ] for the next file not seen, c to comment, v for Seen, t for the hunk's turn, s for the layout, / to filter and ⌘⏎ to finish. Seen and the turn reviewed through are yours, and follow you to your other machines.
- **Artifacts.** What agents made for you in the thread, and the images and videos they showed; each artifact opens beside the conversation. The tab shows once there is something in it.
- **Details.** The handoff; the thread's lineage, what it was forked from and its forks, when it has any; its commits and branch; comments on code; and what its agent's sandbox reaches, the grants in force and the plugin calls made.

On a phone, the list of threads is the home page and New thread opens from it; the menu at the left of each page's header opens the sidebar. A long press on a thread opens the menu a right-click does.

[History and search](history-and-search.md) and [forks and merge requests](forks-and-merge-requests.md) have the details.

## Settings
Settings, the cog at the sidebar's bottom right, holds **Projects** (with their defaults and adding one), **Devices**, **Sync**, **Orgs**, **Chats**, **Notifications**, **Appearance** and **Text and layout**. Appearance picks a light and a dark theme from Silt's own and yours, imported from VS Code or Zed or written yourself; Text and layout sets the zoom, text sizes, fonts and spacing over it; see [Themes](themes.md).

The design is in [ADR 0044](../adr/0044-the-standard-agent-workspace.md), [ADR 0022](../adr/0022-agent-runtime.md) and [ADR 0024](../adr/0024-per-person-state.md).
