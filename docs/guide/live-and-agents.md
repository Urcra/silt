# Live collaboration and agents from the web UI

This guide covers working on one thread together in real time, and running agents from the web UI: any agent that speaks the Agent Client Protocol (ACP). The design is in [ADR 0014](../adr/0014-live-collaboration-and-silt-driven-agents.md) and [ADR 0017](../adr/0017-acp-agents-only.md).

## Working on a thread together
1. The thread's owner shares it: `silt thread share <thread> <user>` (`--remove` to undo). Anyone who can see a thread can comment on it; only its owner and collaborators can change it.
2. Everyone opens it in the web UI with `silt open`.

The thread page then shows, live:
- **Who else is here**, the file and line they have open and their draft of the next prompt. "Follow" next to someone shows you what they look at, file and line, as they move; picking a file or typing stops following.
- **The thread's files.** Edits made in the browser merge with everyone else's as they type and reach the owner's checkout.
- **The agent at work**, if one was started from the web UI, with what it is saying right now.

## Agents from the web UI
- **On a thread:** pick an agent under "Run an agent here" and press "Start agent". It runs in the thread's checkout on your machine. If there is no checkout yet, Silt checks the thread out from a clone it knows: one you recorded a session in or ran `silt open` from. It remembers them across restarts.
- **In a new thread:** the new thread (`silt open` lands on it) starts an agent in one of your clones, in the clone itself (Local) or in a fresh checkout of it (Worktree).
- **Send.** "Send" sends your draft together with everyone else's on the thread, each marked with who wrote it. Drafts clear once they were sent.
- **Permissions.** When the agent asks before acting, the request shows on the thread page with its options.
- **Stop and continue.** "Cancel turn" interrupts the agent; "Stop agent" ends it. Starting the same agent again continues its session if the agent can load sessions. Otherwise it gets the conversation so far with its first prompt.

The session is recorded like any other: it has history and search, and `silt resume <thread> --agent claude` or `silt fork` carry it into an agent CLI.

### Which agents
- **The ACP registry's agents**, such as Claude Code, Codex, Gemini CLI and OpenCode through their ACP adapters. OpenCode also runs on your own API key for any provider, including local models. Silt refreshes the list daily and falls back to a built-in copy offline. Agents distributed through `npx` or `uvx` need those commands; binary ones are downloaded once and checked against their checksum. The picker lists only the ones installed on the machine: those whose CLI is on the daemon's `PATH` (`claude` for Claude Code, `codex`, `gemini`, `opencode`, …) and binary ones downloaded already. Google Antigravity runs as `agy acp` when `agy` is installed, signed in as you signed agy in, rather than as the registry's separate download; when it needs signing in, the card's **Log in from the terminal** runs `agy acp --login`. [Qudiks](https://github.com/viterkim/qudiks), the Codex fork, is listed when `qudiks` is installed, and runs through Codex's adapter with `CODEX_PATH` naming your `qudiks`, so it works with the provider you set it up with, such as GitHub Copilot.
- **Silt (local model)**, Silt's own agent, on a model this machine runs with llama.cpp: download one in Settings › Local models or with `silt model pull`. In Chat mode it only talks; its other modes give it tools to read, search, edit and write files and run commands in the project, asking as the preset says. What you send it stays on this machine. Claude Code runs on a local model too: choose `local:<name>` in its model setting, or run `silt claude --model local:<name>`.
- **Your own**, from `agents.json` in Silt's data directory (`~/.local/share/silt` unless `SILT_HOME` is set):
  ```json
  [{"id": "my-agent", "name": "My agent", "command": "/usr/local/bin/my-agent", "args": ["--acp"], "env": {"MY_AGENT_MODE": "fast"}}]
  ```

Agents get only part of the daemon's environment:
- what tools need, such as `PATH`, `HOME` and the locale, plus proxy and certificate settings;
- their own vendor's variables: `ANTHROPIC_*` for Claude, `OPENAI_*` for Codex, and `QUDIKS_*` besides for Qudiks, `GEMINI_*` and `GOOGLE_*` for Gemini, and `AGY_*` and `ANTIGRAVITY_*` besides for Antigravity, `GITHUB_*` and `GH_*` for Copilot.

To give agents more, list the variable names in `agent-env` in Silt's data directory, one per line. A custom agent can also name them in `"pass_env": [...]` in `agents.json`. Start the daemon (`silt daemon stop`, then any `silt` command) with the keys the agents need.

Agents the registry distributes as binaries are checked against the registry's checksum before they run. One without a checksum is refused unless `SILT_ALLOW_UNVERIFIED_AGENTS=1` is set.

What a command may do is up to the agent: Silt shows its permission requests, and the agent's own sandbox, where it has one, contains what it runs. In threads shared with others, turn that sandbox on: the agent reads their content, which can carry instructions.

## Secrets
Files follow git: what you would commit is synced as it is, so keep secrets out of tracked files as you would for git, for example in a git-ignored `.env`. The conversation is different, because agents read your environment and print what they find. Before a message, tool call or transcript goes to the server, Silt replaces:
- values of environment variables whose names contain `KEY`, `TOKEN`, `SECRET`, `PASSWORD` or the like;
- values listed in the `secrets` file in Silt's data directory, one per line (`#` starts a comment);
- tokens in well-known formats, such as GitHub's `ghp_…` and Anthropic's `sk-ant-…`.

The replacements name the secret by a short hash (`[REDACTED:<id>]`), keyed with a key that never leaves the machine, so nobody can test a guess against it. They stay in place everywhere, including on your own machines. A machine keeps redacting a secret it has redacted once, even after it leaves the environment. The design is in [ADR 0019](../adr/0019-redact-to-gits-threat-model.md) and [ADR 0123](../adr/0123-redaction-ids-keyed-per-machine.md).
