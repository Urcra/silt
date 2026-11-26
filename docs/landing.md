# Your agent's work, anywhere you are

Silt records your coding agent's sessions, the code and the conversation, so you can pick them up on another machine, fork them, and find anything an agent ever said. It's open source, and you host it yourself.

[Get started](guide/index.md) · [Host a server](self-hosting.md) · [Source](https://github.com/Urcra/silt)

## Stop on your laptop, continue at your desk
Run your agent through Silt and work as usual. Later, on any of your machines, run `silt resume`. You get the same files, the same conversation, and the agent's own session, so it remembers what it was doing. There's no work-in-progress commit to make first. [How a hand-off works](guide/handoff.md).

```sh
silt claude                 # on the laptop
silt resume <thread>        # at the desk
```

## Fork it, then merge it back
Try something else from any turn, with the code and conversation as they were then. Bring newer work across with `silt pull`. Offer a fork back as a merge request, and approve it or let both lines of work carry on. Reverting to a turn keeps what came after, greyed out, rather than throwing it away.

## Nothing is lost to compaction
Every message, tool call and edit is kept, before and after the agent compacts its context. Search it all, find which turn changed a file or made a commit, and follow it on a graph of every thread, fork and merge.

## The agents you already use
Claude Code, Codex, Qudiks, Gemini CLI, Antigravity CLI, OpenCode and GitHub Copilot CLI. Carry a thread from one agent to another with `silt resume --agent`. Agents that speak ACP can also run from Silt's web UI.

## For a team, on your own server
One binary runs both the sync server and your machine's daemon. A team gets:
- orgs and sign-in with GitHub or OIDC;
- forks of each other's work, with merge requests and review;
- live editing with presence;
- a web app for phones, where you can answer your agents from anywhere.

Work for several groups from one machine, or keep a project off every server. [Several servers](guide/servers.md).

## Your data stays yours
Silt sends no telemetry, now or in the future, and has no service of its own to send anything to. Your threads go to the servers you run and nowhere else. Turn on Offline and Silt connects only to those servers. What leaves is what your agent sends its own model provider, and with a local model, the conversation stays on your machine. [What Silt sends over the network](guide/network.md).

Silt is licensed under the AGPL-3.0.
