# Command-line reference

Every `silt` command, as `silt <command> --help` describes it. Generated from the commands themselves by `silt reference`, so it matches the version it came with.

## `silt run`

Run an agent CLI (claude, codex, qudiks, gemini, agy, opencode, copilot) with Silt recording the session and every file edit

- `<AGENT>`
- `--isolated`: Work in a managed checkout instead of the current one
- `<ARGS>`: Arguments passed on to the agent

## `silt chat`

Start a chat: an agent CLI in a new folder of its own rather than a project, recorded as a private thread

- `<AGENT>`: The agent CLI to run (claude, codex, qudiks, gemini, agy, opencode, copilot)
- `--about <ABOUT>`: What the chat is about, naming its folder
- `--workspace <WORKSPACE>`: The workspace it's in, by id; the default workspace otherwise
- `<ARGS>`: Arguments passed on to the agent, after `--`

## `silt claude`

Run Claude Code with Silt recording the session and every file edit

- `--isolated`: Work in a managed checkout instead of the current one
- `<ARGS>`: Arguments passed on to the agent

## `silt codex`

Run Codex with Silt recording the session and every file edit

- `--isolated`: Work in a managed checkout instead of the current one
- `<ARGS>`: Arguments passed on to the agent

## `silt qudiks`

Run Qudiks, the Codex fork for GitHub Copilot and local endpoints, with Silt recording the session and every file edit

- `--isolated`: Work in a managed checkout instead of the current one
- `<ARGS>`: Arguments passed on to the agent

## `silt gemini`

Run Gemini CLI with Silt recording the session and every file edit

- `--isolated`: Work in a managed checkout instead of the current one
- `<ARGS>`: Arguments passed on to the agent

## `silt agy`

Run Antigravity CLI with Silt recording the session and every file edit

- `--isolated`: Work in a managed checkout instead of the current one
- `<ARGS>`: Arguments passed on to the agent

## `silt opencode`

Run OpenCode with Silt recording the session and every file edit

- `--isolated`: Work in a managed checkout instead of the current one
- `<ARGS>`: Arguments passed on to the agent

## `silt copilot`

Run GitHub Copilot CLI with Silt recording the session and every file edit

- `--isolated`: Work in a managed checkout instead of the current one
- `<ARGS>`: Arguments passed on to the agent

## `silt track`

Start recording the most recent session of an agent in this directory

- `--agent <AGENT>`
- `--session <SESSION>`: Record this session rather than the most recent one
- `--from-hook`: Record the session whose hook input is on stdin, and capture that hook; for a `SessionStart` hook in an environment that starts the agent itself, such as a cloud session

## `silt import`

Import the agent sessions already on disk, each as a thread kept on this machine until sent

- `<PATHS>`: Only sessions that ran in or under these directories
- `--agent <AGENT>`: Only this agent's sessions

## `silt send`

Send imported threads to a server, redacted: some, a project's, or all

- `<THREADS>`: The threads to send
- `--project <PROJECT>`: All imported threads of this project, as `silt project` names it
- `--all`: Every imported thread kept on this machine
- `--server <SERVER>`: Send them to this server rather than the default one

## `silt resume`

Continue a thread here: its code, its conversation and its agent session

- `<THREAD>`
- `--here`: Use this clone instead of a managed checkout (needs a clean tree)
- `--agent <AGENT>`: Continue with another agent: its own session translated from the conversation, or a new one primed with it
- `<ARGS>`: Arguments passed on to the agent

## `silt fork`

Fork a thread, yours or a teammate's, with its code and conversation

- `<THREAD>`
- `--private`: Only you see the fork, until a merge request offers it back
- `--turn <TURN>`: Fork from where this turn ended, as the thread's page numbers them
- `--here`: Use this clone instead of a managed checkout (needs a clean tree)
- `--agent <AGENT>`: Continue with another agent: its own session translated from the conversation, or a new one primed with it
- `<ARGS>`: Arguments passed on to the agent

## `silt pull`

Bring a fork's source's newer code and conversation into the fork

- `<THREAD>`: The fork; defaults to the thread checked out here

## `silt revert`

Take a thread's files back to how a turn left them; later turns stay in its history, marked rewound

- `<THREAD>`: The thread; defaults to the one checked out here
- `--turn <TURN>`: The turn to go back to, as the thread's page numbers them

## `silt mr`

Merge requests from forks back to their source threads

### `silt mr create`

Offer a fork's work to the thread it was forked from

- `--thread <THREAD>`: The fork; defaults to the thread checked out here
- `--title <TITLE>`
- `--summary <SUMMARY>`: Defaults to the changed files and the fork agent's last message

### `silt mr list`

List merge requests, all or those of one thread

- `<THREAD>`

### `silt mr show`

Show a merge request: summary, conflict hints, diff and the fork's conversation

- `<MR>`
- `--hide-git`: Leave out what git itself rewrote in the fork's tool calls
- `--hide-others`: Leave out the edits of other agents working in the fork's clone

### `silt mr approve`

Merge it into its target (on the device working on the target)

- `<MR>`

### `silt mr decline`

Decline it; both threads carry on

- `<MR>`
- `--reason <REASON>`

### `silt mr request-changes`

Keep it open and ask the fork for changes first

- `<MR>`
- `<REASON>`

### `silt mr guide`

Write its change guide for reviewers, or without text ask the fork's agent running in the web UI to write it

- `<MR>`
- `<TEXT>`

## `silt visibility`

Set who on the server sees this project's threads, or one thread's: `private`, `org`, or `team:<name>`

- `<VISIBILITY>`
- `--thread <THREAD>`: Set it for one thread (and its forks that have no setting of their own)

## `silt auth`

Connect this machine to a Silt server

### `silt auth login`

Sign this machine in: by approving it in the browser, with a password (`--user`), or with a code from `silt server pair`

- `--server <SERVER>`
- `--code <CODE>`: A pairing code from `silt server pair`
- `--user <USER>`: Sign in as this local account, asking for its password
- `--password-stdin`: Read the password from standard input
- `--name <NAME>`: This device's name; defaults to the host name
- `--no-browser`: Only print the sign-in link

### `silt auth logout`

Sign this machine out of a server

- `--server <SERVER>`: The server to sign out of; defaults to the first signed in to

### `silt auth status`

Show which servers this machine syncs with

## `silt thread`

Delete, restore or purge a thread

### `silt thread delete`

Move a thread of yours to the trash, on every device

- `<THREAD>`

### `silt thread restore`

Bring a thread back from the trash

- `<THREAD>`

### `silt thread purge`

Remove a thread in the trash for good, keeping only what forks of it build on

- `<THREAD>`

### `silt thread erase`

Erase a Local only thread and its forks from this machine for good, with what agents kept of them

- `<THREAD>`
- `--yes`: Erase it; without this, says what would go

### `silt thread trash`

List the threads in the trash

### `silt thread share`

Let someone edit a thread of yours with you, or stop it with --remove

- `<THREAD>`
- `<USER>`
- `--remove`

### `silt thread link`

Make a read-only link to a thread of yours, for anyone holding it

- `<THREAD>`
- `--days <DAYS>`: How long it works: 1, 7 or 30 days
- `--frozen`: Show the thread as it is now, not as it goes on

## `silt inbox`

What you should hear about: merge requests, comments and mentions

- `--all`: Include notifications already read

### `silt inbox read`

Mark notifications read

- `<IDS>`
- `--all`: Every unread notification

## `silt lan`

Phones on your local network, with no server: answer agents and review from the sofa

### `silt lan on`

Let phones on this network reach this machine, over HTTPS with a certificate of its own

### `silt lan off`

Stop letting phones on this network reach this machine

### `silt lan status`

Whether phones can reach this machine, where, and which are paired

### `silt lan pair`

Show a code a phone pairs with, then allow or deny the phone here

- `--prompts`: Let the phone send prompts too, not only answer permissions and review

### `silt lan revoke`

Sign a paired phone out

- `<PHONE>`: Its id, as `silt lan status` lists it

## `silt org`

Orgs on the server: who is in them and what they default to

### `silt org list`

List your orgs with their members, teams and projects

### `silt org create`

Create an org (server admins)

- `<ORG>`

### `silt org add`

Add a user to an org, or change their role

- `<ORG>`
- `<USER>`
- `--owner`: Make them an owner, who manages the org

### `silt org remove`

Remove a user from an org

- `<ORG>`
- `<USER>`

### `silt org default`

What new projects of the org default to: `private`, `org` or `team:<name>`

- `<ORG>`
- `<VISIBILITY>`

### `silt org policy`

Set the org's policy for agents started in its projects on members' machines (owners); rules not given restrict nothing

- `<ORG>`
- `--agents <AGENTS>`: The agents allowed, comma-separated (claude, codex, qudiks, copilot, gemini, agy, opencode)
- `--no-custom-agents`: Refuse the agents of a machine's agents.json
- `--max-preset <MAX_PRESET>`: The most an agent may do without asking: ask_for_everything, plan, ask_before_commands, auto or full_access
- `--sandbox`: Agents run only with their own sandbox on
- `--silt-sandbox`: Agents run in Silt's own sandbox, and don't start on a machine that can't make one
- `--backend <BACKEND>`: What makes Silt's sandbox on members' machines: bubblewrap (the only one so far)
- `--hide-other-workspaces`: The sandbox also keeps agents out of Silt's data and other workspaces' folders
- `--home-locked`: Projects keep this server: no other, and not one machine only
- `--connectors <CONNECTORS>`: The page connectors its projects' pages may call, comma-separated
- `--network <NETWORK>`: The most network its sandboxes get: none, gateway, full or host
- `--devices <DEVICES>`: The most devices its sandboxes get: none, relayed, named or all_usb
- `--refuse <REFUSE>`: What members may never hand a sandbox, comma-separated: ranges, ports, paths, gpu, docker, ssh_agent
- `--no-nested`: Agents' own sandboxes may not run inside Silt's
- `--ask-folders <ASK_FOLDERS>`: The folders agents may ask for, comma-separated, each `root:allow|ask|never`, with `:rw` where writing may be given; members' own rules only tighten these
- `--plugin-install <PLUGIN_INSTALL>`: A plugin the org installs and keeps on in its workspaces, as `<id>=<git-url>#<commit>`; give it again for each
- `--plugin-allow <PLUGIN_ALLOW>`: The only plugins that may be on, comma-separated: an id, `publisher:<name>` or `git:<url>`
- `--plugin-pin <PLUGIN_PIN>`: The one version a plugin may run at, as `<id>=<version>`; give it again for each
- `--plugin-rule <PLUGIN_RULE>`: An org's rule on a plugin's action, as `<id>:<action>[:<field>=<pattern>]=ask|never`; give it again for each
- `--refuse-opaque`: No plugin call may have an effect Silt can't check, such as TCP, SSH or a local MCP server's
- `--refuse-reach`: No plugin may widen an agent's sandbox, as Folders does

## `silt team`

Teams within an org, for `team:<name>` visibility

### `silt team create`


- `<ORG>`
- `<TEAM>`

### `silt team delete`


- `<ORG>`
- `<TEAM>`

### `silt team add`


- `<ORG>`
- `<TEAM>`
- `<USER>`

### `silt team remove`


- `<ORG>`
- `<TEAM>`
- `<USER>`

## `silt project`

The project (repository) this clone belongs to

### `silt project move`

Move this clone's project into an org

- `<ORG>`

### `silt project set`

Name this clone's project and say what its new threads start with; an empty value clears a setting

- `--name <NAME>`
- `--agent <AGENT>`: The agent's registry id, such as claude-acp
- `--preset <PRESET>`: ask_before_commands, ask_for_everything, auto or full_access
- `--git-notes <GIT_NOTES>`: Git notes on its threads' commits: off, local, or push to origin too
- `--forge-comments <FORGE_COMMENTS>`: Whether the server comments on its threads' pull requests: on or off

### `silt project home`

Choose where this clone's project's new threads sync: a server this machine is signed in to, or `local` to keep them on this machine. Threads kept here move to the server chosen

- `<HOME>`

## `silt model`

Local models this machine runs with llama.cpp, for Silt's own agent and for Claude Code

### `silt model list`

List the models here and what this machine can run

### `silt model search`

Find GGUF models on Hugging Face, the most downloaded first; with no words, the most downloaded of all

- `<WORDS>`

### `silt model files`

List a Hugging Face model's quantizations, with whether each fits here

- `<REPO>`

### `silt model pull`

Download a model: `owner/repo:file.gguf` from Hugging Face, or a GGUF file you have

- `<MODEL>`

### `silt model rm`

Remove a model; a GGUF file of your own stays where it is

- `<MODEL>`

### `silt model runtime`

Download llama.cpp's latest build of llama-server for this machine

## `silt workspace`

Workspaces: which projects belong together, the server their threads sync with, and the rules for agents started in them

### `silt workspace list`

List the workspaces, their servers, rules and projects

### `silt workspace new`

Make a workspace

- `<NAME>`
- `--server <SERVER>`: The server its threads sync with, one signed in to
- `--org <ORG>`: The org on that server its projects belong to, whose policy then applies
- `--local`: Keep its threads on this machine
- `--local-only`: Keep its threads on this machine, with only local models answering
- `--remote <REMOTES>`: Put projects whose remote matches in it, such as `github.com/acme/*`; repeatable
- `--folder <FOLDERS>`: Put clones at or under this folder in it, such as `~/work`; repeatable
- `--default`: Make it where projects nothing matches go

### `silt workspace rule`

Add or remove a workspace's match rule

- `<WORKSPACE>`
- `--remote <REMOTE>`
- `--folder <FOLDER>`
- `--remove`: Remove the rule rather than add it

### `silt workspace assign`

Put this clone's project, or `--project`'s, in a workspace; `--rules` puts it back under the match rules

- `<WORKSPACE>`
- `--rules`
- `--project <PROJECT>`: The project, by its normalized remote; this clone's by default

### `silt workspace which`

Say which workspace this clone is in, and the rules in effect there

### `silt workspace policy`

Change a workspace's rules; those its org sets apply as well

- `<WORKSPACE>`
- `--agents <AGENTS>`: The agents allowed, comma-separated (claude, codex, qudiks, copilot, gemini, agy, opencode), or `any`
- `--custom-agents <CUSTOM_AGENTS>`: Whether agents from agents.json are allowed: on or off
- `--max-preset <MAX_PRESET>`: The most an agent may do without asking: ask_for_everything, plan, ask_before_commands, auto, full_access, or `any`
- `--sandbox <SANDBOX>`: Agents run only with their own sandbox on: on or off
- `--silt-sandbox <SILT_SANDBOX>`: Agents run in Silt's own sandbox, and don't start on a machine that can't make one: on or off
- `--backend <BACKEND>`: What makes Silt's sandbox once it's on: bubblewrap, the only one so far
- `--hide-other-workspaces <ISOLATED>`: The sandbox also keeps agents out of Silt's data and other workspaces' folders: on or off
- `--home-locked <HOME_LOCKED>`: Projects keep the workspace's server: on or off
- `--local-only <LOCAL_ONLY>`: Nothing Silt holds of its threads leaves this machine, and only local models answer: on or off
- `--wide-history <WIDE_HISTORY>`: In a Local only workspace, agents' history tools read every project in it, not only their own: on or off
- `--connectors <CONNECTORS>`: The page connectors its pages may call, comma-separated, or `any`
- `--network <NETWORK>`: What its sandboxes connect to: none, gateway, full (the default) or host
- `--hosts <HOSTS>`: The hosts "through Silt only" reaches, comma-separated, such as `*.github.com`, or `any`
- `--devices <DEVICES>`: Which devices its sandboxes get: none, relayed, named or all_usb
- `--device-nodes <DEVICE_NODES>`: The device nodes `--devices named` gives, comma-separated, or `none`
- `--ranges <RANGES>`: Address ranges its sandboxes reach past those always blocked, such as a VPN's, comma-separated, or `none`
- `--ports <PORTS>`: Ports on this machine its sandboxes reach, comma-separated, or `none`
- `--paths <PATHS>`: More folders for its sandboxes, comma-separated, each read-only or with `:rw` writable, or `none`
- `--ask-folders <ASK_FOLDERS>`: The folders its agents may ask for, comma-separated, each `root:allow|ask|never`, with `:rw` where writing may be given; or `none`
- `--gpu <GPU>`: Its sandboxes get the GPU: on or off
- `--docker <DOCKER>`: Its sandboxes get the Docker socket, which is root on this machine: on or off
- `--ssh-agent <SSH_AGENT>`: Its sandboxes get your SSH agent, which signs as you anywhere: on or off
- `--refuse <REFUSE>`: What may never be handed its sandboxes, comma-separated: ranges, ports, paths, gpu, docker, ssh_agent; or `none`
- `--nested <NESTED>`: Agents' own sandboxes may run inside Silt's: on or off

### `silt workspace env`

Set or unset a variable given to every agent started in a workspace, such as its own CLAUDE_CONFIG_DIR

- `<WORKSPACE>`
- `<VARIABLE>`: `NAME=value` to set, or `NAME` with --unset
- `--unset`

### `silt workspace default`

Make a workspace the default, where projects nothing matches go

- `<WORKSPACE>`

### `silt workspace remove`

Remove a workspace; its projects go to the others by the rules

- `<WORKSPACE>`

## `silt access`

Silt's sandbox on this machine: whether it works, and how to fix what doesn't

### `silt access doctor`

Check that Silt's sandbox can run here, and say how to fix what can't

### `silt access show`

What reached a thread's sandboxes through Silt: plugin calls, grants and what the gateway refused

- `<THREAD>`

### `silt access release`

Let go of a device relayed to a thread's sandboxes, ending its session

- `<THREAD>`
- `<DEVICE>`

### `silt access unfolder`

Take a folder back from a thread's agents; they lose it when their turn ends

- `<THREAD>`
- `<PATH>`

## `silt credential`

Credentials Silt's gateway adds for a workspace's sandboxed agents, which never see them

### `silt credential add`

Add one; the secret is read from stdin, unless `--command` prints it

- `<NAME>`: Its name, such as `work-github`
- `--workspace <WORKSPACE>`: The workspace whose agents it's for, by id
- `--host <HOST>`: The host it's sent to, or `*.example.com` for a domain
- `--path <PATH>`: Only requests whose path starts with it
- `--header <HEADER>`: The header it goes in
- `--scheme <SCHEME>`: Put before the secret in the header, such as `Bearer` or `token`
- `--command <COMMAND>`: A command run on this machine, outside the sandbox, that prints the secret when a run starts, such as `gh auth token`
- `--placeholder <PLACEHOLDER>`: A variable the sandbox gets a placeholder in, for tools that want one before they send anything, such as `GH_TOKEN`

### `silt credential list`

List them, without their secrets

### `silt credential remove`

Remove one, and its secret from the keyring

- `<NAME>`

## `silt plugin`

Plugins: what a workspace's sandboxed agents reach production and other systems through

### `silt plugin add`

Add a plugin from a git repository (`URL#tag` to pin one), showing everything it gets first

- `<SOURCE>`
- `--workspace <WORKSPACE>`: Turn it on in this workspace too; may be given more than once
- `--yes`: Agree to what it gets without being asked

### `silt plugin dev`

Add or rebuild a plugin from a folder you're developing

- `<FOLDER>`
- `--workspace <WORKSPACE>`
- `--watch`: Keep watching it, building it again whenever it changes, until stopped

### `silt plugin update`

Update a plugin from where it came from, showing what the new version adds

- `<ID>`
- `--yes`: Agree to what it adds without being asked

### `silt plugin trust`

Trust a publisher, so its plugins' actions start from their own defaults rather than asking

- `<PUBLISHER>`
- `--off`: Stop trusting it

### `silt plugin environment`

Set the patterns that pick one of a plugin's environments, such as `acme/*,acme-labs/*`; `none` removes it

- `<ID>`
- `<NAME>`
- `<PATTERNS>`

### `silt plugin credential`

Say which of `silt credential`'s credentials is one of a plugin's own

- `<ID>`
- `<CREDENTIAL>`: The plugin's own name for it, from its manifest
- `<NAME>`: The credential `silt credential add` keeps
- `--environment <ENVIRONMENT>`: For this environment only

### `silt plugin rule`

Allow, ask or never one of a plugin's actions; with `--where field=pattern`, only for calls that match

- `<ID>`
- `<ACTION>`
- `<RULE>`: allow, ask or never; `reset` removes your rule
- `--where <MATCHING>`

### `silt plugin set`

Set one of a plugin's settings to a JSON value; `null` resets it

- `<ID>`
- `<KEY>`
- `<VALUE>`
- `--environment <ENVIRONMENT>`: For this environment, for a setting that's set per environment
- `--workspace <WORKSPACE>`: For this workspace, for a setting that's set per workspace

### `silt plugin secret`

Keep one of a plugin's credentials as its own secret, read from stdin into the keyring, or printed by `--command` when it's needed; Silt adds it to that plugin's requests alone

- `<ID>`
- `<CREDENTIAL>`: The plugin's own name for it, from its manifest
- `--environment <ENVIRONMENT>`: For this environment only
- `--command <COMMAND>`: A command run on this machine that prints it, such as `gh auth token`
- `--remove`: Forget it

### `silt plugin list`

List this machine's plugins and the workspaces each is on in

### `silt plugin remove`

Remove a plugin

- `<NAME>`

### `silt plugin enable`

Turn a plugin on in a workspace, whose sandboxed agents then get its tools

- `<NAME>`
- `--workspace <WORKSPACE>`

### `silt plugin disable`

Turn a plugin off in a workspace

- `<NAME>`
- `--workspace <WORKSPACE>`

## `silt approvals`

Plugin calls waiting for your approval

- `--all`: Answered and expired ones too
- `--thread <THREAD>`: Only this thread's

## `silt approve`

Approve a plugin call, after reading exactly what it runs; or reject it

- `<ID>`
- `--reject`
- `--grant <GRANT>`: Also let calls like it run without asking: for the rest of this `run`, the `thread` a while, or `always`, by a rule of yours; or, for a folder, make it the `workspace`'s own
- `--minutes <MINUTES>`: How long a thread's grant lasts, in minutes

## `silt revoke`

End a grant, so the calls it let through ask again

- `<GRANT>`

## `silt design`

Check chart palettes and text contrast, as artifacts' design guidance asks

### `silt design palette`

Check a chart palette: lightness, chroma, separation under normal vision and colour-vision deficiencies, and contrast

- `<COLORS>`: The colours in the order they're assigned, comma-separated, such as `#c25a3a,#5b7a8a`
- `--surface <SURFACE>`: The chart's surface, such as the design system's `bg`
- `--theme <THEME>`: The theme the surface belongs to, which sets the lightness band
- `--pairs <PAIRS>`: Which pairs must be told apart: neighbours, as in bars and lines, or all, as in scatter plots and maps
- `--ordinal`: Check an ordinal ramp instead: one hue stepping steadily in lightness
- `--json`: Print the checks as JSON, `{"passed": …, "checks": [{"name", "verdict", "detail"}]}`

### `silt design contrast`

Text contrast of one colour on another: WCAG 2's ratio and APCA's lightness contrast

- `<TEXT>`: The text's colour, `#rrggbb`
- `<GROUND>`: The colour behind it, `#rrggbb`

## `silt theme`

Themes for Silt's UI: list, start, import from VS Code or Zed, and check them

### `silt theme list`

List the themes for Silt's UI, Silt's and yours, with the light and dark ones picked

- `--json`: Print the list as JSON, as the web UI reads it

### `silt theme new`

Start a theme of your own from one of Silt's or yours, in your themes folder

- `<ID>`: The new theme's id: lowercase letters, digits and hyphens
- `--from <FROM>`: The theme to start from
- `--name <NAME>`: Its name, as Settings shows it; from the id when left out
- `--force`: Replace a file of yours with the same name

### `silt theme import`

Read a VS Code colour theme or a Zed theme family into Silt's format, in your themes folder

- `<FILE>`: The theme's JSON file; VS Code's may have comments
- `--to <TO>`: The id of the theme, or the stem of a family's ids; from its name when left out
- `--appearance <APPEARANCE>`: Whether a VS Code theme that doesn't say is light or dark; guessed from its background when left out
- `--fix-contrast`: Move the colours it sets that fail as text until they reach 4.5:1
- `--force`: Replace a file of yours with the same name

### `silt theme check`

Check a theme file in Silt's format: what Silt can't read in it, and its text's contrast

- `<FILE>`
- `--fix-contrast`: Move the colours it sets that fail as text until they reach 4.5:1, and save the file
- `--json`: Print the warnings and checks as JSON

### `silt theme path`

Print the folder your themes go in

## `silt log`

List recorded threads, or with --commit the threads and turns that made a commit

- `--commit <COMMIT>`

## `silt show`

Show a thread's conversation and the files each tool call changed

- `<THREAD>`

## `silt diff`

Show what a thread, or one of its turns, changed

- `<THREAD>`
- `--turn <TURN>`: Only the changes made in this turn (1-based)
- `--to-turn <TO_TURN>`: With --turn, the changes of every turn from it to this one
- `--base <BASE>`: Diff from merge-base (with the fork's source), last-commit, last-turn or commit:<sha>; by default from the thread's start. Or against a branch as git has it now: main or branch:<name>
- `--hide-git`: Leave out what git itself rewrote in a tool call, as a checkout, pull, merge, rebase, reset or stash does; not against a branch
- `--hide-others`: Leave out the edits of other agents working in the same clone

## `silt search`

Search conversations: your team's through the server when connected

- `<QUERY>`
- `--repo <REPO>`: Repository, as its normalized origin URL (github.com/org/repo)
- `--author <AUTHOR>`: The owning user
- `--status <STATUS>`: active, idle or merged
- `--branch <BRANCH>`
- `--file <FILE>`: Threads that changed this file, or anything under this directory
- `--since <SINCE>`: YYYY-MM-DD, or a number of days ago such as 7d
- `--until <UNTIL>`: YYYY-MM-DD, or a number of days ago such as 7d

## `silt open`

Open the web UI, in the desktop app when it's installed: threads, search, history, merge requests and comments

- `--no-browser`: Print the address instead of opening the app or a browser

## `silt history`

A thread's full history as numbered entries, across compactions and forks

- `<THREAD>`
- `--offset <OFFSET>`
- `--limit <LIMIT>`

## `silt comment`

Comment on lines of a file or on a history entry of a thread

- `<THREAD>`
- `--file <FILE>`: A file of the thread, with --lines
- `--lines <LINES>`: A line or a range such as 10-14
- `--entry <ENTRY>`: A history entry, numbered as `silt history` shows them
- `--base <BASE>`: Number --lines as the file was at this diff base (start, last-turn, last-commit, merge-base, commit:<sha>), to comment on lines the thread deleted
- `--hide-git`: Number --lines as `silt diff --hide-git` shows them
- `--hide-others`: Number --lines as `silt diff --hide-others` shows them
- `--reply <REPLY>`: Answer this comment, by its id as `silt comments` shows it
- `<TEXT>`

## `silt comments`

List a thread's comments, or resolve one

- `<THREAD>`
- `--resolve <RESOLVE>`
- `--base <BASE>`: Also say where comments on code are at this diff base
- `--hide-git`: Number lines as `silt diff --hide-git` shows them
- `--hide-others`: Number lines as `silt diff --hide-others` shows them

## `silt in-flight`

Other threads changing a file or directory of this repository

- `<PATH>`

## `silt blame`

Each line of a file with the thread, turn and prompt that wrote it: from the log when a thread here has the file, else from the commits' notes in refs/notes/silt

- `<FILE>`
- `--notes`: Read only the notes, as a clone without Silt would

## `silt server`

Run or administer a self-hosted sync server

### `silt server init`

Write the server's config and create its first admin

- `--data <DATA>`: Server data directory; defaults to `server` in the Silt data directory
- `--listen <LISTEN>`: Address to listen on
- `--public-url <PUBLIC_URL>`: The URL people and devices reach the server at, such as https://silt.example.com
- `--admin <ADMIN>`: The first admin's user name
- `--password-stdin`: Read the admin's password from standard input instead of generating one
- `--systemd <SYSTEMD>`: Also write a systemd unit for the server to this path
- `--service-user <SERVICE_USER>`: The system user the unit runs the server as
- `--force`: Replace an existing config

### `silt server run`

Serve the sync API and the sign-in pages

- `--listen <LISTEN>`: Address to listen on; overrides `listen` in server.toml
- `--data <DATA>`: Server data directory; defaults to `server` in the Silt data directory

### `silt server pair`

Print a one-time code a device uses with `silt auth login --code`

- `--data <DATA>`
- `--user <USER>`: The user the device will belong to; created if missing

### `silt server user`

Manage local accounts

- `--data <DATA>`

#### `silt server user add`

Add a local account; prints a generated password unless one is given

- `<NAME>`
- `--admin`
- `--password-stdin`

#### `silt server user list`


#### `silt server user passwd`

Set a user's password

- `<NAME>`
- `--password-stdin`

#### `silt server user admin`

Make a user a server admin, or `--revoke` it

- `<NAME>`
- `--revoke`

#### `silt server user disable`

Stop a user signing in and sign them out everywhere, or `--enable` them again

- `<NAME>`
- `--enable`

#### `silt server user sign-out`

Sign a user out of every device and browser, such as after a lost laptop

- `<NAME>`

### `silt server backup`

Copy the server's data to a new directory; safe while it runs

- `<DEST>`
- `--data <DATA>`

### `silt server restore`

Put a backup in place as the server's data; stop the server first

- `<BACKUP>`
- `--data <DATA>`
- `--replace`: Replace the data that is there

### `silt server invite`

Print a sign-up link for one person

- `--data <DATA>`
- `--org <ORG>`: Org the person joins
- `--days <DAYS>`

## `silt sync`

Push what this machine recorded to the server once, in this process, as is done when no daemon runs

- `<THREAD>`: One thread's family; defaults to every thread here
- `--once`

## `silt update`

Update Silt: a build of a clone pulls its `main` and builds it again with `just update`, then restarts the daemon once no agent is mid-turn

- `--check`: Only say whether an update is out

## `silt daemon`

Manage the background daemon

### `silt daemon run`

Run the daemon in the foreground

### `silt daemon start`

Start the daemon in the background if it isn't running

### `silt daemon stop`

Stop the daemon

### `silt daemon restart`

Restart the daemon on this build: at once when no agent works in it, else once none does

- `--if-replaced`: Only when an install replaced its program since it started, as `just install` and closing the desktop app ask

### `silt daemon status`

Show whether the daemon is running

### `silt daemon compact`

Give back the space a store from an older Silt holds: compress its old chunks and shrink the file. The daemon does it once by itself

## Run for you

Silt runs these itself, from agents' hooks and configuration and for the docs site, so they're left out of `silt --help`.

### `silt agent`

Silt's own agent, which the daemon runs over ACP

#### `silt agent local`

Silt's own agent, on a model this machine runs through the daemon's gateway

### `silt hook`

Called by agent hooks; reads the hook input on stdin

- `--agent <AGENT>`: The agent whose hook format the input and output use
- `<EVENT>`

### `silt reference`

Print the command-line reference in Markdown, for the docs site

### `silt mcp`

Serve Silt's history tools to an agent over MCP on stdio

- `--thread <THREAD>`
- `--checkout <CHECKOUT>`: The checkout whose skills to offer; defaults to the current directory
- `--token <TOKEN>`: The run's token, which limits answers to its workspace
- `--ui`: A person watches the run in the web UI: offer the tools that show them images and artifacts

### `silt sample-tools`

Serve the tools a page gave its question to the agent answering it

- `<FILE>`

### `silt sandbox-init`

Inside Silt's sandbox: relay ports to sockets outside, then run a program

- `--relay <RELAY>`: `PORT=SOCKET`: the sandbox's own 127.0.0.1:PORT reaches SOCKET outside
- `<COMMAND>`

### `silt sandbox-run`

Run a program in Silt's sandbox, as the plan Silt wrote says

- `--plan <PLAN>`
- `--in-network`: Already in the sandbox's own network namespace
- `<COMMAND>`
