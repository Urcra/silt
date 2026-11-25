# Hand off a thread to another machine

Start a task on your laptop, then finish it on your workstation. The other machine picks up the code, the conversation and the agent's own session, so the agent remembers everything it did.

Both machines need to be signed in to the same server, as the same person. Run `silt auth login --server <url>` on each.

## On the first machine
Work as usual, through Silt:

```sh
silt claude
```

When you stop, everything is already recorded: the conversation, each file the agent or you changed, and any commits. Commits that only this machine has go to the server with the thread. Note the thread's id from `silt log`:

```sh
silt log
```

## On the other machine
In a clone of the same repository, continue the thread:

```sh
silt resume <thread>
```

Silt does three things:
- It makes a checkout of the thread under Silt's data directory, with its files as they were left.
- It restores the agent's session there.
- It starts the agent, which continues where it stopped.

With `--here` it uses your clone instead, if its working tree is clean. With `--agent codex`, or any other agent, the conversation goes on in that agent. It gets its own session translated from the conversation, or a new one primed with it.

The web UI does the same from a thread's page. In the server's web app, "Continue on" starts the thread on one of your machines that's online.

## While both machines have it
Only one machine at a time works in a thread. When the second machine resumes it, it takes the thread over. If the first machine records anything afterwards, that goes into a fork, so nothing is lost and nothing is mixed. `silt log` marks a thread that moved elsewhere as `moved`.

## Trying something else instead
To keep the original as it is, fork it rather than resume it:

```sh
silt fork <thread>
```

`silt fork --turn 3` starts from where its third turn ended. Bring the source's newer work into the fork with `silt pull`. Offer the fork's work back with `silt mr create`, and its owner approves or declines it.

See the [command-line reference](../reference/cli.md) for every option.
