# What Silt sends over the network

Silt sends no telemetry and no crash reports. Everything it sends over the network on its own is listed here, with how to stop it.

## Your servers
A machine talks to the servers you sign in to, and only those. A thread syncs with its home server alone, and a project kept on this machine syncs nowhere ([Several servers](servers.md)). Before a conversation leaves the machine, secret-looking values in it are replaced ([Live collaboration and agents](live-and-agents.md)).

## Checks for updates
- **A build from a clone** runs `git fetch` in that clone when the daemon starts and every six hours after, to see whether `main` moved.
- Plugins installed from git are checked for newer versions the same way, from their own repositories.

`SILT_UPDATE_CHECK=never` in the daemon's environment turns both off. A debug build never checks.

## Plan usage
When the message box opens for Claude Code, Codex or GitHub Copilot, the web UI shows how much of your plan is left. To find out, the daemon reads that agent's own sign-in on this machine and asks the provider directly. For Copilot that's a GitHub token from `COPILOT_GITHUB_TOKEN`, `GH_TOKEN` or `GITHUB_TOKEN`, or the sign-in GitHub's Copilot editors keep. It asks:
- `api.anthropic.com` for Claude Code;
- `chatgpt.com` for Codex;
- `api.github.com` for Copilot.

The answer stays on this machine. Nothing is asked for an agent you aren't signed in to.

## Agents
- **The ACP registry,** the list of agents Silt can start from the web UI, is fetched from `cdn.agentclientprotocol.com` at most once a day. A copy built into Silt stands in when it can't be reached.
- **Adding an agent from the registry** downloads it from where the registry says: an npm or PyPI package, or a release archive whose checksum the registry gives.

## Only when you use them
- **Local models:** searching for and downloading a model from Hugging Face, and llama.cpp's release from GitHub ([Local models](local-models.md)).
- **Dictation:** the speech models, from Hugging Face and the speech engine's GitHub releases, the first time you dictate, checked against pinned hashes.
- **Artifacts:** a library an agent's page loads from a CDN is fetched once by Silt and kept, so the page itself reaches no network ([Artifacts](artifacts.md)).
- **Plugins:** installing a plugin from git clones its repository with your own sign-in ([Plugins](plugins.md)). What a plugin reaches is what its manifest declares and you granted.
- **Phones on your network:** off until you turn it on. Then the machine listens on your local network for the phones you pair, and drops connections from anywhere else ([The hosted app and phones](hosted-app-and-phones.md)).
