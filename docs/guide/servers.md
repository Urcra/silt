# Work with several servers, or keep a project on one machine

One machine can be signed in to several servers: your company's and an open-source project's, say. Each thread syncs only with its own server, and nothing of it reaches the others.

## Signing in
Sign in to each server:

```sh
silt auth login --server https://silt.work.example
silt auth login --server https://silt.oss.example
silt auth status
```

The first server you signed in to is the default.

## Where threads go
A new thread goes where its project says, else to the default server. A teammate's thread syncs with the server it came from. To choose where a project's new threads go, run this in a clone of it:

```sh
silt project home https://silt.oss.example
```

You can also pick the server under "New threads sync with" in the web UI's Settings, under Projects. Threads already on a server stay there, as other machines and people may hold them.

## Keeping a project on this machine
Some work shouldn't reach any server. Keep it local:

```sh
silt project home local
```

Its new threads never sync. `silt log` and the thread's page say "this machine only", and sharing them is refused. To move them to a server later, choose one with `silt project home <url>`. They sync there from then on, with all their history.

## In the web UI
The workspace switcher at the top of the sidebar picks what you see:
- everything;
- one server, or one org on it;
- this machine's own threads.

The sidebar, Needs you and the graph follow it, and Settings' Orgs and Devices are that server's. The inbox, search and Settings' Projects gather what every server has.

To sign out of one server, run `silt auth logout --server <url>`.
