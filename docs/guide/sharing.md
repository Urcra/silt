# Sharing and who sees what

On a server, every thread is in a project, and every project is in an org. Who sees a thread follows from there, unless the thread says otherwise. This page covers choosing who sees your threads and who may write into them, links for people without an account, and deleting threads. Setting up orgs and teams on your server is in [Self-hosting](../self-hosting.md).

## Who sees a thread
There are three choices:
- `org`: everyone in the project's org;
- `team:<name>`: one team of the org;
- `private`: only you.

The most specific choice wins:
1. **The thread's own choice:** `silt visibility private --thread <thread>`. Only its owner sets it.
2. **For a fork without one:** its source's.
3. **The project's choice:** `silt visibility team:core`, run in a clone of it.
4. **The org's default for new projects:** `silt org default <org> <visibility>`, set by its owners.

On a thread's page, the Share dialog sets the same under "Who sees it", or "As its project says" to follow the project.

A private thread reaches others only through the merge requests it offers. `silt fork --private <thread>` makes a fork that is private from the start.

## Working on a thread together
Anyone who sees a thread can read it, comment on it and fork it. To let someone write into it with you, live in the web UI:

```sh
silt thread share <thread> alice
silt thread share <thread> alice --remove
```

The Share dialog's "People who may also write into it" does the same. Working on it together is in [Live collaboration](live-and-agents.md).

## Links for anyone
A read-only link lets anyone holding it read a thread's conversation, without an account:

```sh
silt thread link <thread> --days 7
silt thread link <thread> --days 1 --frozen
```

Or use "Make a link" in the Share dialog.
- **How long:** a link always expires, after a day, a week or 30 days.
- **What it shows:** the thread as it goes on, or with `--frozen` as it is now.
- **Ending it:** revoke a link from the Share dialog at any time. Making the thread private ends its links too.
- **Turned off:** an org's owners can turn links off for the whole org.
- **One artifact:** **Share › Read-only link…** on an open artifact makes a link to it alone, at the version you see or following new ones; nothing else of the thread shows. Its page runs sandboxed, as in Silt. To send an artifact without a server, **Share › Download as one HTML file** makes a file that opens in any browser ([ADR 0068](../adr/0068-artifacts.md)).

## Projects
A project is a repository, known by its normalized origin URL. The web UI's Settings, under Projects, lists your orgs' projects, with this machine's clone of each, and clones one that isn't here. In a clone:

```sh
silt project set --name "The app" --agent claude-acp --preset ask_before_commands
silt project move acme
silt project home local
```

In order, these:
1. name the project and choose what its new threads start with;
2. move it into another org;
3. keep its new threads on this machine. See [several servers](servers.md).

## Deleting threads
```sh
silt thread delete <thread>
silt thread trash
silt thread restore <thread>
silt thread purge <thread>
```

- **Deleting** moves one of your threads to the trash, on every device.
- **`silt thread trash`** lists what's there, and **restoring** brings a thread back.
- **Purging** removes a thread for good, keeping only what its forks build on. The server purges threads left in the trash after 30 days, unless its config says otherwise.

The design is in [ADR 0013](../adr/0013-accounts-orgs-and-self-hosting.md), [ADR 0023](../adr/0023-projects.md) and [ADR 0027](../adr/0027-read-only-links.md).
