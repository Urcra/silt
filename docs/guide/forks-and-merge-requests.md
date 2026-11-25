# Forks and merge requests

A fork is a new thread that starts with another thread's code and conversation. Fork a teammate's half-finished work to carry it on in your own agent, or one of your own threads to try another direction. Neither thread is changed by the other until you choose: pull brings the source's newer work into the fork, and a merge request offers the fork's work back, for its owner to approve.

## Forking
In a clone of the repository:

```sh
silt fork <thread>
```

Silt makes a checkout with the thread's files, and restores the conversation into a new session of your agent, which continues from there. The source thread, and its agent's session, stay as they were.

- **From an earlier turn.** `silt fork --turn 3 <thread>` starts from where the third turn ended, as the thread's page numbers them. The web UI has "fork from turn N" on each turn.
- **In another agent.** `--agent codex` continues in Codex, or any other agent, as `silt resume` does.
- **Kept to yourself.** `--private` makes a fork only you see, until a merge request offers it back.
- **In your own clone.** `--here` uses your clone instead of a managed checkout, if its working tree is clean.

`silt log` lists the fork as a fork of its source. The web UI's thread page has a Fork button too.

## Bringing in the source's newer work
While you work in the fork, its source may go on. Bring its newer code and conversation in:

```sh
silt pull
```

Run it in the fork's checkout, or name the fork. The source's edits merge with yours, and your checkout is updated. Your agent hears on its next prompt what came in, and `silt show` lists it as upstream progress.

## Offering the work back
When the fork has something worth keeping:

```sh
silt mr create --title "Handle expired tokens"
```

The summary defaults to the files changed and the agent's last message; `--summary` writes your own. The merge request offers the fork as it is now. Work you do on the fork afterwards needs a new merge request.

To help reviewers, add a change guide, a walk-through of what changed and why:
- write it yourself with `silt mr guide <mr> "<text>"`;
- or, without the text, have the fork's agent write it. The agent has to be running in the web UI on your machine.

## Reviewing
The source's owner hears about the merge request in their inbox (`silt inbox`), and reviews it:

```sh
silt mr list
silt mr show <mr>
```

`silt mr show` gives the summary, the diff since the fork, the fork's conversation, and conflict hints: lines that both threads changed. Silt merges them anyway; the hints say where to look. The web UI's merge request page shows the same, with the change guide. With `--hide-git`, or **Hide git's changes** on the page, the diff leaves out what git rewrote in the fork, such as a pull of the target. With `--hide-others`, or **Hide other agents' changes**, it leaves out the edits of other agents that worked in the fork's clone at the same time.

In the web UI, tick each file as **Seen** once you've read it. The count shows how much is left. Your ticks follow you to your other machines, and a file that changes afterwards shows as unseen again.

Then decide:
- `silt mr approve <mr>` merges the fork into its source. Run it on the machine working on the source. Its checkout gets the change, and its agent hears about it on its next prompt.
- `silt mr request-changes <mr> "<reason>"` keeps it open. The fork's owner and their agent hear the reason, and the merge request shows whether the fork has moved on since.
- `silt mr decline <mr> --reason "<why>"` closes it. Both threads carry on.

A merge never deletes the fork. It stays readable, and can be continued, pulled into, forked again or offered again.

## Comments
Comment on code or on any entry of a thread's history, from the web UI or the CLI:

```sh
silt comment <thread> --file src/auth.rs --lines 10-14 "Should this refresh first?"
silt comment <thread> --entry 42 "This is where it went wrong"
silt comments <thread>
silt comment <thread> --reply <comment> "Fixed in the next turn"
silt comments <thread> --resolve <comment>
```

Comments stay anchored to the code they're on as it changes, and show as outdated once that code is gone. Replies sit under the comment they answer. A reply tells the comment's author and everyone who replied before. In the web UI's changes panel, line comments from the gutter can be sent to the thread's agent.

See [the reference](../reference/cli.md) for every option, and [ADR 0010](../adr/0010-forks-pulls-and-merge-requests.md) and [ADR 0025](../adr/0025-comment-replies-and-requested-changes.md) for the design.
