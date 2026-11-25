# 0046: No server needed on one computer

- Status: accepted
- Date: 2026-10-25
- Amends: [ADR 0045](0045-windows-natively-and-in-wsl.md) (handoff between Windows and WSL through a server)

## Context
ADR 0045 made Windows and each WSL distribution separate machines, each with its own daemon and data, and moved work between them by handoff through a home server both sign in to.

Trying it on a Windows computer with a WSL distribution showed what that costs. The desktop app shows both sides in one window, yet a thread recorded in WSL can't be continued on Windows, or the other way, until a server is set up and both sides are signed in. Without one, parts of the web UI asked a server anyway: the inbox answered 400 "not connected to a server", as did the notification settings under it.

A person working alone on one computer has nobody to share with and no other machine to reach. A server there is overhead that buys nothing.

## Decision
**Without a server is a first-class way to use Silt.** Everything one person does on one computer works without one: recording, search, history, forks, merge requests between their own threads, and the web UI and desktop app.

**Windows and its WSL distributions count as one computer.** They hand threads to each other, and see each other's threads, without a server. Each side keeps its own daemon and data, as ADR 0045 has it; what changes is that they sync with each other directly.

**A server is for what needs one:**
- working with other people: sharing, reviews, the inbox's notifications from others, orgs;
- moving work between separate computers.

**Without a server, a page answers empty rather than failing.** It asks for what only a server has (devices, orgs, notification settings, others' notifications) only when there is one.

How the two daemons on one computer find and sync with each other is still to be designed. Candidates are a direct connection between them, since WSL 2 forwards `localhost` to Windows, or the desktop app relaying between the backends it already knows. Either way it reuses the sync protocol `ClientSync` and `Room` already speak (ADR 0032), and stays correct when only one side's daemon runs.

## Consequences
- docs/guide/windows.md stops telling a Windows and WSL user to set up a server, once the direct sync exists.
- Every server-only request in the web UI is checked against a machine with no server; the inbox already answers empty.
- A machine signed in to a server keeps using it for handoff between Windows and WSL too, so nothing changes for people who already have one.
