# 0065: Workspaces: a server, defaults, rules and history apart for each kind of work

- Status: accepted
- Date: 2026-11-14
- Amends: [ADR 0032](0032-several-servers-and-local-projects.md) (a project's home, and what the WorkspaceSwitcher lists), [ADR 0023](0023-projects.md) (project defaults), [ADR 0048](0048-an-agents-modes-are-its-permissions.md) (modes a workspace caps), [ADR 0059](0059-chats-without-a-project.md) (where a chat syncs), [ADR 0011](0011-history-tools-search-and-web-ui.md) (what the history tools may read)
- Amended by: [ADR 0092](0092-local-models.md): Local models, run by Silt, and workspaces that stay on the machine
- Amended by: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md): Silt sandboxes the agents it starts, and production is reached through plugins
- Reversed by: [ADR 0114](0114-one-set-of-permissions-and-agents-by-their-marks.md): One set of permissions, and agents shown by their marks
- Amended by: [ADR 0115](0115-the-sidebar-names-the-workspace-and-the-machine.md): The sidebar names the workspace at its top and the machine at its bottom

## Context
People keep personal and work projects on one machine, and the two come with different terms:
- **Agents.** An employer's data processing agreement may allow some agents and not others, such as Copilot but not Claude Code or Codex.
- **Permissions and sandboxing.** A team may forbid skipping permissions, or require the agent's sandbox.
- **Where history goes.** Work may need its own server, and personal projects another or none.
- **History apart.** An agent working on a personal project shouldn't read a work conversation through Silt's history tools, and the reverse holds too.

**What Silt has today.**
- **Homes (ADR 0032).** One daemon signs in to several servers, and each family has a home. A project without a home of its own syncs with the default server. Nothing says that everything under `~/work`, or every `github.com/acme/*` repository, goes to the work server.
- **The web UI's workspace** is only a view: an org on a server, or "This machine".
- **Defaults, not rules.** Project defaults (ADR 0023), `chats.json` and `agents-seen.json` suggest an agent and a preset. Nothing refuses anything:
  - a CLI agent's extra arguments go to it unread, so `silt claude --dangerously-skip-permissions` works anywhere;
  - any preset and any of the agent's modes can be chosen.
- **No history isolation.**
  - `silt mcp --thread <id>` only sets a default: every tool takes any thread id, and `silt_thread_search` searches every server and the whole local index.
  - The daemon's socket checks only the user id.
  - `silt.sqlite` and `servers.json`, which holds each server's token, are readable by any agent running as the person.

**What the agents offer** (November 2026):

| Agent | Per run | Enforced machine-wide | A separate home |
|---|---|---|---|
| Claude Code | `--settings`, above user and project settings: `permissions.disableBypassPermissionsMode`, `disableAutoMode`, `sandbox.enabled`, `sandbox.failIfUnavailable`, `sandbox.allowUnsandboxedCommands`, filesystem denies, `permissions.deny` | `managed-settings.json` in `/etc/claude-code/`, MDM, server-managed settings | `CLAUDE_CONFIG_DIR` |
| Codex | `-c sandbox_mode=…`, `-c approval_policy=…` | `requirements.toml` (`allowed_sandbox_modes`, `allowed_approval_policies`) in `/etc/codex/` | `CODEX_HOME` |
| Copilot CLI | `--deny-tool`; allow-all is opt-in (`--allow-all-tools`, `--allow-all`, `--yolo`, `COPILOT_ALLOW_ALL`) | the organization's Copilot policy, on or off | `COPILOT_HOME` |

What each agent enforces covers the whole machine, so it can't tell work from personal. Silt starts the agent, knows the project it starts in, and can pass each agent's own locks for that one run.

## Decision
**A workspace is a set of projects with one server, its defaults and its rules.** It's a local record, kept in `workspaces.json` in the data dir:
- an id (a slug) and a name;
- **server:** one sign-in's URL and, optionally, one of its orgs; or none, for this machine only;
- **match rules:** remote patterns (`github.com/acme/*`, `gitlab.acme.dev/**`) and folder roots (`~/work`);
- **defaults** for new threads: agent, preset, and chats' preset, and per agent its model, effort and mode, by the ids the agent lists them with, since each agent's are its own. One the agent no longer lists, or a mode the rules refuse, leaves it as the agent last ran;
- **env:** variables given to every agent started in it, such as `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `COPILOT_HOME`, so work keeps its own agent sign-ins and transcripts;
- **rules** (below).

A machine has at least one workspace, and one is the default. On upgrade, the daemon makes **Default**, with the default server, and nothing changes until the person makes a second.

**Which workspace a project is in.**
- A project the person put in a workspace (`project_workspaces`) stays there.
- Otherwise the most specific matching rule decides. A remote pattern beats a folder root, and a longer match beats a shorter one.
- Otherwise the project is in the default workspace.

A family's workspace is pinned when it's made (`family_workspaces`), as its home is. A chat is in the workspace chosen when it was started.

**Home follows the workspace.** A new family's home is its workspace's server, or this machine for a workspace with none. A project's own home (`silt project home`) still overrides that, unless the rules lock the home.
- A project with only local threads can move to any workspace.
- A project with threads on a server can move only to a workspace with that server. Otherwise the move is refused and says why: threads never move between servers (ADR 0032).

**Rules refuse; defaults only preselect.** Each rule is optional; one left unset restricts nothing.
- **Agents:** the agents allowed, named the same for a CLI agent and its ACP adapter (`claude`, `codex`, `copilot`, `gemini`, `agy`, `opencode`), and whether `agents.json`'s custom agents are allowed.
- **The most an agent may do without asking:** a cap on the preset, ordered Ask for everything < Plan ≤ Ask before commands < Full access.
  - An agent's modes are capped through the preset each maps to (ADR 0048), so `bypassPermissions`, `auto` and Autopilot are refused under a cap below Full access.
  - Under a cap, a mode Silt doesn't know is refused.
- **Sandbox required:** the agent runs only with its own sandbox on, and fails rather than running without it. An agent Silt can't start sandboxed is refused.
- **Isolated**, shown as "Hide other workspaces' files": the sandbox also denies reads of Silt's data dir and of other workspaces' folders and agent homes. Isolated implies sandbox required. It's named for what it hides, since history tools are scoped to their workspace without it (below), and "isolated history" read as if they weren't.
- **Home locked:** its projects can't be given another server, or this machine only. A workspace with no server and a locked home keeps everything on this machine.

**An org can set rules for its projects.**
- An org's owners set a policy on its server, with the same rules.
- A machine fetches it for each workspace bound to that org, with each listing, and keeps the last one it fetched.
- What applies is the stricter of the workspace's own rules and the org's, rule by rule: allowed agents intersect, the lower cap wins, and any lock that's on stays on.
- The person can add to the org's rules but never loosen them.
- Until a machine has fetched the policy once, it refuses to start agents in that workspace, saying it's waiting for the org's policy. Signing in fetches it.

**Where the rules are enforced.** Wherever Silt starts or changes an agent, so the terminal, the web UI, the hosted app and phones all get the same answer:
- **The CLI.**
  - `silt <agent>` refuses an agent the rules don't allow.
  - Each adapter knows its agent's flags that skip asking or the sandbox, such as `--dangerously-skip-permissions`, `--permission-mode bypassPermissions`, `--dangerously-bypass-approvals-and-sandbox`, `--sandbox danger-full-access`, `--yolo` and `--allow-all-tools`. A run under a cap below Full access that passes one is refused.
  - The adapter then passes the agent's own locks: Claude Code's `--settings` with bypass mode disabled, its sandbox, and the denies; Codex's `sandbox_mode` and `approval_policy`; Copilot's `--deny-tool` and no allow-all.
- **The daemon.**
  - Starting an ACP agent, choosing a preset, a mode or an agent setting, and the same commands relayed from the control channel are all refused with the rule.
  - Silt's own answers to an agent's permission requests never go above the cap.
- **A refusal names the workspace, the rule and where it came from**: the person's rules, or the org's on its server.

**Silt's history tools answer for the agent's workspace only.**
- Each run gets a token, kept hashed in the store with its thread and workspace, so it works without a daemon.
  - The CLI and the daemon make one for each agent they start.
  - It reaches `silt mcp` as `--token`, in the MCP server's configuration. The agent can see it there, which gives it nothing: the token only opens its own workspace.
- A socket request carrying a token:
  - may only read history;
  - finds, searches and reads only threads in the token's workspace;
  - searches only that workspace's server.
- Requests without a token are the person's own, as before.
- The web UI's "All workspaces" view is for the person. No agent searches across workspaces.

**What this doesn't promise.** Silt enforces its rules for the agents it starts. It can't stop someone running `claude` without Silt. Outside an isolated workspace, an agent running as the person can still read Silt's files directly. The guide says both.

**In the web UI.**
- The WorkspaceSwitcher lists "All workspaces" and then each workspace, with its initial in a colour of its own, its server and its org's rules.
- Settings › Workspaces holds each workspace's server, match rules, defaults, agent environment, and a table of its rules: the person's, the org's and those in effect.
- Projects are grouped by workspace, each saying why it's there.
- New thread and the composer show refused agents, presets and modes greyed out, with the reason under each.
- The hosted app's Orgs page has a Policy tab for owners.


**From the command line:**
- `silt workspace list`, `new`, `assign`, `which` and `policy` manage them;
- `silt chat --workspace` picks a chat's workspace.

## Consequences
- **A new pure crate, `silt-workspace`,** holds workspaces, match rules, rules, resolution, merging and the start check. `Preset` moves there from the daemon.
- **Stores.** The daemon's store gains `project_workspaces`, `family_workspaces` and `run_tokens`. The server's gains `org_policies`, and `OrgChange` gains `Policy`.
- **Views.** `ThreadSummary` and the projects view say each thread's and project's workspace.
- **Adapters** report whether they can start their agent sandboxed: Claude Code with isolation, Codex and Gemini CLI sandboxed only, the others and every ACP agent not at all, so a workspace that requires the sandbox refuses those. Where an agent offers no sandbox, or no way to pass its locks per run, the guide says which rules refuse it. For Copilot's sandbox and Claude's ACP adapter, that's to be checked against the versions released at the time.
- **Agent homes.** With a workspace's `env`, an agent's session files live in that workspace's home. The daemon tells the adapters which home each workspace's folders use, so finding, restoring and importing Claude Code's, Codex's and Copilot's sessions look there; without a daemon, the hook runs in the agent's own environment, which already has it.
