# Import your history

Silt records the sessions you run through it. Your agents' older sessions are already on disk, in each agent's own files, and `silt import` brings them in. From the first day, `silt search` covers everything you did before Silt too.

## Import
```sh
silt import
```

It imports every session of every agent Silt knows: Claude Code, Codex, Qudiks, Gemini CLI, Antigravity CLI, GitHub Copilot CLI and OpenCode. Each session becomes a thread of its own, in the project of the clone it ran in, and keeps the times it ran at. It prints a line for each:

```
0198a2b3c4d5  claude-code session from 2026-03-04  github.com/acme/app
0198a2b3c4e6  codex session from 2026-05-11  /home/me/scratch (no project)
imported 2 sessions into threads kept on this machine; `silt send --all` sends them to your server
```

Running it again imports only what's new: a session already recorded or imported is never imported twice. A thread you purge stays gone.

To import part of it, name directories, an agent, or both:

```sh
silt import ~/src/app
silt import --agent codex
```

OpenCode answers only for the directory it's asked in. Silt asks it in every clone it knows and wherever the other agents' sessions ran. For a directory Silt has never seen, name it.

## What a thread holds
An imported thread is the conversation: every message, tool call and result, subagents included, searchable and readable with `silt show`, the web UI and the MCP tools.

The code comes later. The thread starts from the commit your clone was at when the session started. The first `silt resume` or `silt fork` checks that commit out and continues from there. The edits the session made show in its tool calls, not as a diff.

A session that ran outside any git clone, or in one that's gone, has no project. It can be read and searched, but not continued.

## Kept here until you send it
Imported threads stay on this machine, whatever your projects' settings. `silt log` says "this machine only", and `silt project home` leaves them where they are. Nothing of them leaves until you send them:

```sh
silt send <thread>
silt send --project github.com/acme/app
silt send --all
```

They go to your default server, or the one `--server` names, and from there to your other machines.

Everything sent is redacted on the way out (see [Secrets](agents.md#secrets)): values of environment variables whose names look secret, the values in your secrets file, and well-known token formats. `silt send` says how many secrets it found. Old sessions can hold secrets you've since rotated out of your environment. Before sending, add any that still matter to the `secrets` file in Silt's data dir (`~/.local/share/silt/secrets`, or under `$SILT_HOME`), one per line. `silt send` reads it again first.
