# Your agents

Silt doesn't run a model of its own. You keep using your agent CLI, and Silt records it. This page covers which agents work, several agents in one project, moving a thread between them, and what Silt gives an agent: its history tools, your repository's skills and rules, and a prepared checkout. Running agents from the web UI over ACP is in [Live collaboration and agents](live-and-agents.md).

## Recording an agent
| Agent | Run it with |
|---|---|
| Claude Code | `silt claude` |
| Codex | `silt codex` |
| Qudiks | `silt qudiks` |
| Gemini CLI | `silt gemini` |
| Antigravity CLI | `silt agy` |
| OpenCode | `silt opencode` |
| GitHub Copilot CLI | `silt copilot` |

`silt run <agent>` does the same for any of them. Arguments after the command go to the agent. With `--isolated`, the agent works in a managed checkout instead of the current one.

Silt records the session as the agent writes it, and every file edit, credited to the tool call or the person that made it. To record a session you started without Silt, run `silt track` in its directory.

Recording doesn't need the daemon. Without one, the agent's hooks record in their own process, and the session is pushed to the server with `silt sync --once` when the agent stops. A daemon, started by any `silt` command or kept running by `packaging/systemd/silt-daemon.service`, also pulls others' work in the background.

The agent needs no setup. Silt passes it hooks, its MCP server and instructions for this run only, in each agent's own way of configuring a run, and leaves your agent's configuration as it was.

## Several agents in one project
Agents working in one project on this machine are told about each other, whether they share a clone or each has its own checkout.

When an agent starts, Silt tells it which other agents are working in the project. For each one it says:
- its thread;
- where it works: your working tree, its own checkout, or another clone;
- what it was asked;
- the files it edited last;
- whether it's working or waiting for you.

The agents already there hear that a new one started.

Agents can then coordinate through two of Silt's tools:
- `silt_agents` lists the other agents again.
- `silt_message` sends one of them a message, such as which files it's about to change. A busy agent gets the message after its next tool call. One waiting for you gets it with your next prompt.

Silt tells the agents that these messages come from another agent, not from you. Nothing is shown in your terminal. A second `silt claude` in a busy clone starts as usual.

An agent can also hand part of its work to an agent on another harness or model, such as a review by Antigravity: see [Agents asking other agents](asking-agents.md).

### Two agents in one clone
Each agent started in a clone gets its own thread, even while another agent works there. Every edit goes into each of those threads, as the files on disk are the same:
- In the thread of the agent that made it, it's credited to that agent's tool call.
- In the other threads, `silt show` lists it as "✎ by another agent", naming the agent and its session.
- When both agents have a tool call open and nothing says which of them made an edit, such as an edit by a shell command, it's listed as "✎ by one of these agents".

`silt diff --hide-others`, or **Hide other agents' changes** in the web UI, leaves the other agents' edits out of a thread's diff. A pull request is made of the clone's commits, so it carries both agents' work. To keep two agents' work apart all the way to their pull requests, run one with `--isolated`.

## Moving a thread to another agent
`--agent` on `silt resume` or `silt fork` continues a thread in another agent:

```sh
silt resume <thread> --agent codex
```

- **Claude Code, Codex, Qudiks, Gemini CLI and Copilot** get a session of their own, translated from the conversation. Tool calls become text, as the other agent has other tools, and thinking is left out.
- **OpenCode** starts a new session, primed with the end of the conversation. It reads the rest with Silt's history tools.
- **Antigravity CLI** resumes its own conversation on the machine that ran it. Anywhere else, in a fork, or coming from another agent, it starts a new conversation primed the same way, as agy resumes only from its own store, which Silt can't write.

[Qudiks](https://github.com/viterkim/qudiks) is a fork of Codex that also runs on GitHub Copilot, OpenAI API credits or a local OpenAI-compatible server. Silt records its sessions as Codex's, from `$QUDIKS_HOME`, else `$CODEX_HOME`, else `~/.qudiks`, the order Qudiks reads them in. Its provider flags pass through, as in `silt qudiks --github`. Without `QUDIKS_HOME`, a `CODEX_HOME` you set is shared by both, and `silt import` lists the sessions there as Codex's: nothing in a session says which of the two wrote it. Its installer turns off approvals and the sandbox; a workspace that doesn't allow skipping them turns both back on for runs through Silt. The web UI and the desktop app run it too, through Codex's ACP adapter ([Live collaboration and agents](live-and-agents.md)).

Antigravity CLI is what Google moved most Gemini CLI users to in June 2026; Gemini CLI still works with a paid API key or a Gemini Code Assist licence. `silt agy` runs agy with a `.gemini` of the run's own, through its `--gemini_dir`, that links everything to yours and adds Silt's MCP server and instructions.

## History tools
Every agent run through Silt gets Silt's MCP server. Its tools search and read the thread's whole history, before and after compaction, the threads it was forked from, and other threads. They also find which prompt changed a file, and who else is changing a path. [History and search](history-and-search.md) lists them.

## Skills and rules
Silt hands each agent your repository's skills and rules, as far as the agent doesn't read them itself.
- **Skills** are found in `.silt/skills`, then `.agents/skills`, then `~/.agents/skills`. A name found earlier wins.
  - Claude Code and Copilot get them natively.
  - The others get them in their instructions, with the `silt_skills` and `silt_use_skill` tools.
  - Skills marked user-invocable show as slash commands, through MCP prompts.
- **Rules** come from the repository's first rules file: `AGENTS.md`, `AGENT.md`, `CLAUDE.md`, `GEMINI.md`, `.rules`, `.cursorrules`, `.windsurfrules`, `.clinerules` or `.github/copilot-instructions.md`. Nested `AGENTS.md` files on the way to the working directory, and `~/.agents/AGENTS.md`, are added too.

## Preparing a checkout
A managed checkout, from `silt resume`, `silt fork` or `--isolated`, starts with only what git has. Two files in the repository set it up, with `.silt/` in place of `.agents/` taking precedence:
- **`.agents/linked`** lists git-ignored paths, such as `node_modules` or `.env`, to share between the clone and its checkouts. The shared copy is kept in Silt's data dir, and is seeded once from the clone.
- **`.agents/prepare`** is a script run in the checkout before the agent starts, in your terminal. When the thread's script differs from the one in your clone, as a teammate's fork's may, Silt shows it and asks before running it, once per version. It gets `SILT_THREAD`, `SILT_CHECKOUT` and `SILT_SOURCE` in its environment.

Your own clone, used with `--here`, is left as it is.

## Secrets
Before anything from the conversation leaves your machine, Silt replaces values that look like secrets: environment variables named like keys or tokens, values in the `secrets` file of Silt's data dir, and tokens in well-known formats. Files follow git: what you would commit is synced as it is. [Live collaboration and agents](live-and-agents.md) has the details.

The design is in [ADR 0007](../adr/0007-agent-session-capture.md), [ADR 0012](../adr/0012-agent-adapters-and-repo-conventions.md) and [ADR 0019](../adr/0019-redact-to-gits-threat-model.md).
