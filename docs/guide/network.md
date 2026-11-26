# What Silt sends over the network

Silt sends no telemetry, analytics or crash reports, and never will. There is no Silt service: nothing Silt does reaches its authors or anyone else you didn't choose. This page lists everything Silt connects to on its own, what each connection carries, and how to turn it off. Settings › Network has a switch for each, and **Offline** turns them all off at once.

## Offline
Turn on **Offline** in Settings › Network, or set `SILT_OFFLINE=1` in the daemon's environment. Silt then connects only to the servers you signed in to:
- it checks for no updates, for Silt or for its plugins;
- it asks no provider for plan usage, so the message box shows only what the agent recorded on this machine;
- it lists the agents built into Silt, without fetching the registry;
- artifacts get only the libraries and fonts Silt already kept, so a page you opened before still works, and a new one renders without what's missing.

What you ask for still happens: downloading a model, installing a plugin or an agent, or **Check now** on the Updates page.

The switches are kept in `network.json` in Silt's data folder, and take effect at once. `SILT_OFFLINE=1` overrides the setting, and Settings › Network says so. If `network.json` can't be read, Silt stays offline until it's fixed or you change a switch, which writes it again.

## Your servers
A machine talks to the servers you sign in to, and only those. A thread syncs with its home server alone, and a project kept on this machine syncs nowhere ([Several servers](servers.md)). Before a conversation leaves the machine, secret-looking values in it are replaced ([Live collaboration and agents](live-and-agents.md)).

## Checks for updates
- **A build of a clone,** on the dev channel, runs `git fetch` in that clone when the daemon starts and every six hours after, to see whether `main` moved.
- **A release,** stable or nightly, asks `api.github.com` for Silt's latest release as often. The request carries Silt's version in its user agent, and nothing else.
- **Plugins** installed from git are checked for newer versions once a day, from their own repositories.

**Check for updates** in Settings › Network turns all of them off, as does `SILT_UPDATE_CHECK=never` in the daemon's environment. A debug build never checks.

## Plan usage
When the message box opens for Claude Code, Codex or GitHub Copilot, the web UI shows how much of your plan is left. To find out, the daemon reads that agent's own sign-in on this machine and asks the agent's provider directly. For Copilot that's a GitHub token from `COPILOT_GITHUB_TOKEN`, `GH_TOKEN` or `GITHUB_TOKEN`, or the sign-in GitHub's Copilot editors keep. It asks:
- `api.anthropic.com` for Claude Code;
- `chatgpt.com` for Codex;
- `api.github.com` for Copilot.

The sign-in goes only to the provider it belongs to, and the answer stays on this machine. Nothing is asked for an agent you aren't signed in to. **Plan usage** in Settings › Network turns it off.

## The agent registry
The list of agents Silt can start from the web UI comes from the ACP registry, fetched from `cdn.agentclientprotocol.com` at most once a day. A copy built into Silt stands in when it can't be reached, and always when **Agent registry** is off in Settings › Network.

## Artifacts' libraries and fonts
An agent's page can load libraries and fonts from a few CDNs: `cdnjs.cloudflare.com`, `cdn.jsdelivr.net`, `unpkg.com`, `cdn.tailwindcss.com`, `code.jquery.com`, and Google Fonts (`fonts.googleapis.com` and `fonts.gstatic.com`).
- **By default they come through Silt.** The daemon fetches each pinned file once, keeps it (at most 512 MiB), and serves it to the page. The viewer's browser never contacts a CDN, and pages you opened before work offline.
- **With the switch off,** each page loads them from the CDNs itself, so every viewer's browser contacts them.
- The hosted app and read-only links do the same through the server, unless `artifact_proxy = false` is in its `server.toml` ([Self-hosting](../self-hosting.md)).
- An artifact you download as one HTML file keeps the CDNs' addresses, as it works without Silt.

See [Artifacts](artifacts.md).

## Your agents
The agent CLIs you run through Silt talk to their own providers: Claude Code to Anthropic, Codex to OpenAI, Gemini CLI to Google, and so on. That traffic goes through their own CLIs, SDKs and ACP, under their own settings, including any telemetry or update checks of theirs. Silt doesn't see or change it.
- **A local model** keeps the conversation on this machine ([Local models](local-models.md)). For Claude Code on a local model, Silt also turns off Claude Code's nonessential traffic.
- **A Local only workspace** keeps its threads off every server. Under Full access, a command the agent runs can still reach the network, so choose the preset to match, and turn on the sandbox ([Sandbox](sandbox.md)).

## Only when you use them
- **Adding an agent from the registry** downloads it from where the registry says: an npm or PyPI package, or a release archive whose checksum the registry gives.
- **Local models:** searching for and downloading a model from Hugging Face, and llama.cpp's release from GitHub ([Local models](local-models.md)).
- **Dictation:** the speech models, from Hugging Face and the speech engine's GitHub releases, the first time you dictate, checked against pinned hashes.
- **Plugins:** installing a plugin from git clones its repository with your own sign-in ([Plugins](plugins.md)). What a plugin reaches is what its manifest declares and you granted.
- **Forges:** pushing to GitHub or opening a pull request goes to the forge with your own sign-in ([Git and forges](git-and-forges.md)).
- **Phones on your network:** off until you turn it on. Then the machine listens on your local network for the phones you pair, and drops connections from anywhere else. Notifications to a paired phone go through its browser's push service, encrypted for that browser, so the service sees only that one was sent ([The hosted app and phones](hosted-app-and-phones.md)).

## A server
A server connects only to what its `server.toml` and its people set up:
- the browsers' push services, for people who allow notifications in the hosted app, with each notification encrypted for their browser;
- the forges named under `forges`, and the sign-in provider under `auth`;
- the CDNs, once for each file artifacts load through it, while `artifact_proxy` is on.

It checks for no updates and reports to no one ([Self-hosting](../self-hosting.md)).
