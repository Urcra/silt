# Workspaces: work and personal projects apart

A workspace is a set of projects with three things of its own:
- the server its threads sync with, or none;
- what new threads start with;
- the rules for the agents started in it.

Use one for work and one for personal projects, say. Work then syncs with your company's server and uses only the agents its agreements allow, while personal projects sync with your own server, or stay on this machine. An agent in one workspace never sees the other's threads through Silt.

Until you make a second workspace, everything is in **Default**, which syncs with the first server you signed in to, as before.

## Make a workspace
```bash
silt workspace new Work --server https://silt.acme.dev --org acme --remote 'github.com/acme/*' --folder ~/work
```
- `--remote` puts in it the projects whose remote matches: `*` is one path segment, and a trailing `**` is any number of them.
- `--folder` puts in it the clones at or under a folder.
- `--server` and `--org` name where its threads sync and whose policy applies; `--local` keeps them on this machine.

The most specific rule decides: a remote beats a folder, and a longer match beats a shorter one. Projects nothing matches go to the default workspace; `silt workspace default <name>` changes which that is. To put one project somewhere by hand, run `silt workspace assign <workspace>` in its clone, and `silt workspace assign --rules` to give it back to the rules.

`silt workspace which` says which workspace a clone is in, and why. `silt workspace list` lists them all, with their projects. In the web UI it's Settings › Workspaces, and the top of the sidebar names the workspace shown and switches to another, or to all of them.

A project whose threads already sync with a server can only move to a workspace that syncs with the same server: threads never move between servers.

## Rules
```bash
silt workspace policy Work --agents copilot --max-preset ask_before_commands --sandbox on
```
- `--agents`: the agents allowed, such as `claude`, `codex`, `copilot`, `gemini`, `agy` and `opencode`, or `any`. One name covers an agent's command-line program and its web UI adapter. `--custom-agents off` refuses the agents in your `agents.json`.
- `--max-preset`: the most an agent may do without asking: `ask_for_everything`, `plan`, `ask_before_commands`, `auto` or `full_access`. It also caps the agent's own modes, such as Claude's auto mode and bypass permissions, and Copilot's Autopilot.
- `--sandbox on`: agents run only with their own sandbox on, and fail rather than run without one. Silt's sandbox (below) also counts.
- `--hide-other-workspaces on`: the sandbox also keeps the agent away from Silt's data and other workspaces' folders. It turns the agent's own sandbox on too. Silt's own sandbox always does. It isn't needed to keep history apart: Silt's history tools never show an agent another workspace's threads.
- `--silt-sandbox on`: agents run in Silt's own sandbox (below).
- `--home-locked on`: the workspace's projects keep its server, with no other and not this machine only.
- `--connectors tracker,calendar`: the page connectors its artifacts' pages may call, by name, or `any`. A page calling another is refused. An org's list applies too, so a page may call only what both allow.

A workspace's defaults only preselect. In Settings › Workspaces, under **New threads start with**, choose the agent and permissions new threads and chats start with, and for each agent its model, effort and mode. Silt offers the choices an agent listed when it last ran on this machine, so run an agent once before setting its defaults.

Rules refuse; they don't just preselect. `silt claude` in a workspace that doesn't allow Claude stops and says which rule, and `silt claude --dangerously-skip-permissions` under a cap does too. So does the web UI, which also greys out what the rules refuse. Silt also passes each agent its own locks: Claude Code gets settings with bypass permissions turned off and its sandbox on, Codex its sandbox and approval modes.

## Silt's sandbox
```bash
silt workspace policy Work --silt-sandbox on
```
A workspace with Silt's sandbox on runs every agent Silt starts in it, whichever agent it is and whether it starts from the terminal or the web UI. In the web UI it's **Run its agents in Silt's sandbox**, under Settings › Workspaces › What its agents can reach. One thread can choose otherwise: the shield before the agent's mark in its composer turns the sandbox on or off for that thread and sets what it reaches, starting from the workspace's rules, and **Make it the workspace's** saves the choice back. An org's policy still holds a thread's choice, so a sandbox it requires can't be turned off there. It wraps the whole agent: its tools, its MCP servers and everything they run. It keeps an eager agent away from production, your credentials and the rest of your machine; it doesn't make code the agent wrote safe to run, so review its changes before you trust them. [Silt's sandbox: what it protects, and what it doesn't](sandbox.md) says more.

**Isolation** says what makes the sandbox (`--backend`). For now that's bubblewrap, on Linux, and it's picked once the sandbox is on: Silt uses the system's (`apt install bubblewrap`), or a `silt-bwrap` beside `silt`. Containers and virtual machines are to come, and Settings lists them already. Where it can't be made, agents don't start in that workspace, and the refusal says why. `silt access doctor` checks a machine, and Settings says so too.
- **What it sees:** a root of its own, holding the system and your toolchains read-only, and the thread's checkout and the agent's own folder (such as `~/.claude`) writable. Caches such as `~/.cache`, `~/.npm` and `~/.cargo/registry` are writable too. Of the agent's folder, it sees this project's sessions only, not other projects'.
- **What it never sees:** your SSH, cloud and forge credentials, the agent's stored sign-in, keyrings (the kernel's too) and password managers, the rest of your home folder, your sockets (`ssh-agent`, Docker, D-Bus), and Silt's data. Of the variables in the shell that starts it, only an allowlist goes in: the locale, the terminal and your toolchains' settings, but never one named as a token, key, secret or password (`CARGO_REGISTRY_TOKEN`, `NODE_AUTH_TOKEN`). Those the workspace sets (`silt workspace env`) and those named in `agent-env` in Silt's data go in as well.
- **What it can't change:** the checkout's `.git` itself, its `config`, `hooks` and `commondir`, and `.silt`, which git and Silt would later run outside it; and the agent's settings, memory, plugins, hooks, commands and skills, which the agent would run later outside it. Claude Code's `~/.claude.json` is the run's own copy, so its MCP servers stay as they were.
- **How it reaches Silt:** its hooks and Silt's tools reach the daemon through a socket that answers only for that run. It can't answer its own permission requests or act as you.

What it reaches beyond that is up to the workspace, and applies only with Silt's sandbox on:
```bash
silt workspace policy Work --network full --ranges 10.20.0.0/16 --devices named --device-nodes /dev/ttyUSB0
```
- `--network`: `none`; `gateway`, the HTTP(S) of tools through Silt only, to the hosts `--hosts` names (`*.github.com,registry.npmjs.org`), or any public one; `full` (the default), any public host but never this machine, its networks or cloud metadata; or `host`, this machine's own network, `127.0.0.1` and Silt's web UI included.
- `--ranges`: address ranges full internet reaches past those always blocked, such as a VPN's.
- `--devices`: `none`, `relayed`, `named` (the nodes `--device-nodes` lists) or `all_usb`. `relayed` is the one to start with: Silt opens each device `--device-nodes` lists on this machine and hands the sandbox a session of it, which outlives the agent's restarts and comes back when the thread is resumed here. A serial port, `/dev/ttyUSB0@115200`, shows up at its own path as a terminal; `adb:SERIAL` gives the sandbox's adb this machine's adb server, filtered so it sees and drives that one device only. The thread's Details, or `silt access show`, lists them, and releasing one ends its session.
- `--ports`, `--paths` (`/path` read-only, `/path:rw` writable), `--gpu on`, `--docker on` and `--ssh-agent on` hand over more of this machine. Docker is root on this machine, and the SSH agent signs as you with every key it holds, anywhere.
- `--nested off` keeps the agent's own sandbox from running inside Silt's.
- A folder `--paths` names is judged where it really is, links followed. It's never handed over when it is, holds or sits in the home folder, Silt's data or a credential's folder (`~/.ssh`, `~/.config`, `~/.aws` and the like): name the project folders themselves. In a writable folder, every git repo keeps its `.git` in place and its git config, hooks and `.silt/` read-only, so nothing the agent does there runs when you next use git. A writable folder is otherwise yours as the checkout is: look at what the agent changed before you run things there. A writable folder too large to look through for repos is left out: name the repos in it, or give it read-only.

### Agents asking for more folders
An agent working across repos, say a library and the app using it, asks for the other folder with Folders, a plugin shipped with Silt and on in every workspace. Its tool, `folders_request`, names the folder, whether it needs to write there, and why. The workspace's folder rules answer first:
```bash
silt workspace policy Work --ask-folders "~/code:ask,~/code/acme:allow:rw,~/code/acme/vendor:never"
```
- Each rule names a root and answers `allow`, `ask` or `never`; `:rw` says writing may be given there. The most specific root wins, and a folder no rule names is asked.
- An org's rules (`silt org policy acme --ask-folders ...`) are judged on their own and the stricter answer holds, so they only tighten yours.
- What `--paths` never hands over, Folders never does either, whatever the rules say.
- An asked folder waits for you as an approval with the folder where it really is, the repos in it and the agent's reason. Approve it once or for the run (`silt approve <id> --grant run`), for the thread a while (`--grant thread`), or always in the workspace (`--grant workspace`, which adds it to `--paths`).

Nothing changes in the running sandbox. When the agent's turn ends, Silt stops it, starts its sandbox again with the folder at its own path, and resumes its session: in the web UI it's told what's now there and goes on by itself; `silt claude` says so in the terminal and resumes Claude Code. `silt access show <thread>` lists the folders granted, and `silt access unfolder <thread> <path>` takes one back, gone at the end of the agent's next turn. Grants are this machine's: a thread resumed elsewhere starts without them.

### GitHub through its plugin
GitHub, shipped with Silt, gives a sandboxed agent clones, pushes and GitHub's API, each by your rules, without its ever holding a token. Turn it on in a workspace, name the repos each environment covers, and give it a credential:
```bash
silt credential add work-github --workspace work --host api.github.com --scheme Bearer --command "gh auth token"
silt plugin environment silt.github acme "acme/*"
silt plugin credential silt.github token work-github --environment acme
silt plugin enable silt.github --workspace work
```
- `github_clone` mirrors a repo for the thread at `/silt/github/<owner>/<repo>.git`, read-only; the agent clones it into its work area, `/silt/work`, and works there.
- `github_push` pushes a branch of its clone: Silt brings the agent's commit into its mirror with git's `upload-pack` alone and pushes exactly that commit, only if the branch is still where it was. The card shows the commits and the files they change.
- Reads go without asking. A push asks; one to the default or a protected branch asks each time; a force-push and a release never go. Issues, comments, pull requests, reviews, merges (bound to the head shown) and workflows ask. `silt plugin rule silt.github push allow --where branch='ada/*'` lets pushes to your own branches go.
- Where it's on, the gateway adds no credential of yours for GitHub's hosts, and `GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN` and `COPILOT_GITHUB_TOKEN` never pass into a sandbox (Copilot's own key goes on at the gateway, for its model only). Anonymous reads of public repos still work.

### A plugin's settings and secrets
- `silt plugin set <id> <key> <value>` sets one of its settings, as its manifest describes it; `--environment` or `--workspace` sets one that differs per environment or workspace, and the plugin's own value holds elsewhere. `null` resets it.
- `silt plugin credential <id> <credential> <name>` gives it one of `silt credential`'s; `silt plugin secret <id> <credential>` keeps one of its own instead, read from stdin into the keyring, or printed by `--command`. Either way Silt adds it to the plugin's requests to the hosts its manifest names, and the plugin never sees it.
- `silt plugin update <id>` lists the commits since the one installed, and asks again for whatever the new version adds. `silt plugin dev <folder> --watch` builds a plugin you're writing again whenever it changes.

### An org's plugins
An org's policy can say which plugins its members' workspaces have, and hold their calls:
```bash
silt org policy acme --plugin-install acme.deploys=https://git.acme.dev/deploys#4f1c2a9 \
  --plugin-allow publisher:silt,acme.deploys --refuse-opaque \
  --plugin-rule "silt.github:push:branch=main=never"
```
- `--plugin-install` installs a plugin from git at that commit on every member's machine, on and locked in the org's workspaces; it's built offline in a sandbox, as any plugin from git is.
- `--plugin-allow` lets only those plugins be on: an id, `publisher:<name>` or `git:<url>`. `--plugin-pin <id>=<version>` lets one run at that version only.
- `--refuse-opaque` refuses every call with an effect Silt can't check, and `--refuse-reach` refuses widening a sandbox, Folders' included.
- `--plugin-rule` asks or nevers a plugin's action, for every call or those matching a field; an org can't allow, so its rules only tighten its members'.

### Credentials the sandbox never sees
A sandbox holds no credentials of yours. Silt's gateway adds them on the way out instead, only for the workspace's agents and only to the host and paths you name:
```bash
silt credential add work-github --workspace work --host api.github.com --scheme Bearer --command "gh auth token" --placeholder GH_TOKEN
printf %s "$TOKEN" | silt credential add work-registry --workspace work --host npm.acme.dev
```
- The secret is printed by `--command`, run on this machine outside the sandbox as each run starts, or read from stdin and kept in the system's keyring.
- `--header` and `--scheme` say how it's sent (`authorization` and `Bearer`, say); `--path` limits it to requests under a path.
- `--placeholder` sets a variable in the sandbox to `silt-placeholder`, for tools that want one before they send anything.
- `silt credential list` lists them without their secrets, and `silt credential remove` removes one.

The agent's own model key, such as `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` or `GEMINI_API_KEY`, goes on at the gateway the same way: the sandbox holds `silt-placeholder` in its place. A sign-in kept in the agent's own folder, such as Claude Code's subscription, isn't shown in the sandbox, so an agent there signs in with a key. Under "through Silt only" with a list of hosts, the agent still reaches its own model's.

The sandbox's tools reach the gateway as their proxy (`HTTPS_PROXY`), and trust its own certificate authority, whose key never leaves Silt's data. It reads only requests to hosts it holds a credential for, and passes the rest through unread.

### Plugins: production through Silt
A sandboxed agent holds no production keys, so it reaches production, test fleets and other systems only through a plugin: a WebAssembly component Silt runs on this machine, outside the sandbox, for the workspaces it's on in. [Plugins](plugins.md) says how they work, are added and updated, and [Writing a plugin](writing-plugins.md) how to make one. `silt plugin list` shows them; Silt ships these, and `silt plugin add <git-url>#<tag>` adds others:

| Plugin | What it gives agents |
|---|---|
| `silt.folders` | asking for more folders (above); on in every workspace |
| `silt.github` | clones, pushes and GitHub's API (above) |
| `silt.graphql` | `graphql_query` and `graphql_mutate` to an API you set per environment |
| `silt.ssh-commands` | `ssh_commands_run`, one command on the hosts you set |
| `silt.local-mcp` | a local MCP server's tools, the server in a sandbox of its own |
| `silt.remote-mcp` | a remote MCP server's tools, over HTTP |

```bash
silt plugin environment silt.graphql prod prod
silt plugin set silt.graphql endpoint https://api.fleet.example/graphql --environment prod
silt plugin set silt.graphql data '["location"]' --environment prod
silt plugin secret silt.graphql token --environment prod --command "fleet token"
silt plugin enable silt.graphql --workspace work
```
- GraphQL API reads each document itself, so a mutation sent as `graphql_query` is refused. `data: ["location"]` keeps location out of what reaches the agent: a request selecting fields such as `lastPosition` or `lat` is refused before it's sent, and a result carrying coordinates, NMEA, GeoJSON, GPX or KML is kept, whoever approved the call. `graphql_schema` keeps the API's schema from introspection, so a field whose type carries location, a `GeoPoint` or a type of `lat` and `lng`, is refused whatever it's called.
- `silt plugin set silt.graphql direct true --environment prod` lets the workspace's sandboxes query it straight through Silt's gateway too, which reads each request: queries go, a mutation only where your rule allows `mutate` there, the data rules hold both ways, and what it can't read, a persisted query's hash, an upload or a subscription's socket, is refused.
- SSH commands runs `ssh_commands_run` on the hosts its `targets` name, through your own `ssh` in batch mode; Local MCP runs the server its `command` names; Remote MCP talks to the server its `url` names. Silt can't check what a command or a local server does, so their cards say so.
- A plugin's credential is added by Silt to its requests, and the plugin never sees it: one of `silt credential`'s (`silt plugin credential`) or its own (`silt plugin secret`).
- A plugin is on only in the workspaces you turn it on in, so your personal projects never get your work's.

Where an environment's rule is `ask` (changes are, unless you say otherwise), the call waits for you. Silt keeps its own copy of the request, and shows it to you with whatever would hide its meaning made visible:
```bash
silt approvals
silt approve 3
```
- Only that copy runs, once, after you approve it; the agent can't change it afterwards or reuse it for another call. An approval answers what you were shown: an answer naming another request is refused. It expires after 15 minutes.
- `silt approve` asks on a terminal, which an agent doesn't have; `--reject` refuses the call. In the web UI it's a card in the thread and in Needs you.
- Approving can also grant calls like it, the same action of the same plugin, for the rest of the run (`--grant run`) or for the thread a while (`--grant thread --minutes 60`). A fork starts with none. An action its plugin marks `once`, as GitHub's pushes to the default branch, allows no grant: each is approved on its own. `silt revoke <grant>` ends one.
- Rules are yours to set per action: `silt plugin rule <id> <action> allow|ask|never`, with `--where field=pattern` for some calls only. A never from any rule wins.
- A call that was sent but whose end Silt didn't see, the daemon stopping included, reads as *outcome unknown*, and isn't sent again by itself.
- The agent waits on a call it was told is pending with `silt_plugin_wait`.
- `silt access show <thread>`, or the thread's Details, lists every call, how it ended, the grants in force and what the gateway refused.

An agent outside Silt's sandbox runs as you, and could answer its own approvals through Silt's socket: approvals hold only for sandboxed runs.

An org caps these with `silt org policy --network`, `--devices`, `--refuse docker,ssh_agent` and `--no-nested`.

Silt's sandbox needs bubblewrap, and pasta for full internet: Silt uses the system's (the `bubblewrap` and `passt` packages), or `silt-bwrap` and `silt-pasta` beside `silt`, which `scripts/build-sandbox-tools.sh` builds statically. On Ubuntu 24.04 and later, bubblewrap needs its AppArmor profile, installed once with root. `silt access doctor` checks all of this, and says how to fix what's missing.

On macOS, Silt's sandbox is Seatbelt, through `sandbox-exec`, with a profile written from the same rules: the Isolation picker's Seatbelt, `--backend seatbelt`, which is what an unset backend means there. There's no root of its own there: the profile closes your home folder and Silt's data but for what the rules name, and the run gets its own `TMPDIR`. Full internet goes through Silt's gateway, as HTTP(S) only, on a port of this machine's loopback that answers only with the run's password; a relayed serial port is a terminal whose path `SILT_SERIAL_PORTS` gives, and adb finds its filter through `ADB_SERVER_SOCKET`. An agent's own sandbox can't run inside Silt's there. macOS's sandbox is new and less tried than Linux's: `silt access doctor` checks it.

Where neither can run, or on Windows, `--sandbox on` asks for the agent's own instead, which not every agent has:

| Agent | Its own sandbox | Hide other workspaces' files |
|---|---|---|
| Claude Code (`silt claude`) | yes | yes |
| Codex (`silt codex`) | yes | no |
| Gemini CLI (`silt gemini`) | yes | no |
| Copilot, Antigravity, OpenCode, and agents run from the web UI | no | no |

An agent with no sandbox of its own is refused there where the rules require one.

Content the agent writes in the checkout is untrusted once it leaves the sandbox: review a `Makefile`, a script or a package's hooks before you run them outside it.

## An org's policy
An org's owners can set the same rules on their server, in the Orgs page's Policy section, or with `silt org policy`. They apply to every member's workspace bound to that org, together with the member's own rules: the stricter of each wins, so a member can add to the org's rules but never loosen them. A machine fetches the policy when it signs in and with each listing, and starts no agent in the workspace before it has.

## Agent accounts
```bash
silt workspace env Work CLAUDE_CONFIG_DIR=~/.config/silt/work/claude
silt workspace env Work COPILOT_HOME=~/.config/silt/work/copilot
```
Every agent started in the workspace gets these variables. With its own `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `COPILOT_HOME`, Work keeps its own sign-ins, and the agents keep their transcripts there, where Silt finds them.

## What workspaces can't stop
Silt enforces the rules for every agent it starts: from `silt <agent>`, `silt resume`, the web UI, the hosted app and phones. It can't stop anyone running `claude` or `copilot` without Silt.

Silt's history tools answer an agent only about its own workspace, whatever its rules. Without a sandbox, though, an agent runs as you and could still read Silt's files and your credentials directly. Silt's sandbox, or **Hide other workspaces' files** with the agent's own, is what keeps it out.
