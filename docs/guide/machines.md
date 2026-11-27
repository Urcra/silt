# Machines over SSH

The desktop app can show Silt from another computer you reach over SSH, such as a VPS or a machine at home with a GPU, in the same window as this computer's. No server is needed, and the other machine opens no ports: the app forwards its daemon's port over SSH ([ADR 0108](../adr/0108-machines-over-ssh.md)).

## Before you add one
- Install Silt on the other machine, or let the app install it there when you add it (see below).
- Sign in to it with a key: the app never asks for a password or a passphrase, as there's no terminal to type them into. Your SSH agent, your keys and `~/.ssh/config`, with its `Host` aliases and `ProxyJump`, all work as in your terminal.
- Connect once with `ssh` in a terminal, so its host key is in `known_hosts`. The app won't trust a key it hasn't seen.

## Add a machine
Open **Settings › Machines**, or pick **Add a machine** in the machine menu at the bottom of the sidebar. Give it a name, which is what the sidebar shows, a destination, `user@host` or a `Host` from `~/.ssh/config`, and a port if it isn't 22.

**Test and add** signs in, finds `silt` there, starts its daemon with `silt open --no-browser` and forwards its port, showing each step. When a step fails, it says why and what to do, and you can try again or add the machine anyway.

Each machine's row shows whether it's connected, and its menu edits it, tests the connection, disconnects it or removes it from the list. The list is kept in `machines.json` in the app's data folder.

## Switch to it
Pick it in the machine menu at the bottom of the sidebar, or **Open** on its row. The app connects only then, not at launch, so a machine that's off doesn't slow the app down. While it connects, the sidebar says so and the window stays where it is; if it can't, the window stays and says why. The last machine shown opens first next time.

Once connected, a machine stays connected until you disconnect it or quit the app, so switching back and forth is quick.

The window then shows that machine's own web UI: its threads, its agents and its files. Its Settings › Machines lists your machines and can open them, but adds, changes and removes them only from this computer.

## Every machine's threads in one list
With **All machines** picked in the machine menu, the sidebar lists the threads of every connected machine together, sorted by what moved last as always, so you see what changed on each without switching ([ADR 0136](../adr/0136-every-connected-machines-threads-in-one-list.md)). It's picked from the start; picking one machine in the menu lists only that machine's, as before.
- A project worked on from two machines is one group, matched by its origin, and **Needs you** gathers from every machine.
- Each thread on another machine is marked with it: a chip with its name, unless you pick another way.
- Opening one switches the window to its machine and opens it there. Pin, snooze and settle work on it from here.
- Each machine's row in the menu says how it stands, such as `over SSH · 1 needs you · 2 running`.
- Only connected machines are listed: picking All machines connects nothing. The foot of the sidebar says while a machine's threads load, and when one is lost; the app reconnects it.
- **New thread** asks which machine to start on. Picking another switches to it, with what you typed and the project.

**Settings › Machines** picks how a thread's machine is marked: a machine chip, the chip in the machine's colour, in the line under the title (`Working · on nuc`), a colour stripe, its initial, or nothing. **Mark the shown machine's threads too** marks the machine the window shows as well. Each machine's colour is picked on its row. These hold on every machine's page, and are kept in `view.json` in the app's data folder.

The app asks each machine for its threads every few seconds over the connection it already has, so nothing new is opened on the other machine. A machine running an older build is listed too, but its own page shows only its threads until it's updated.

## Install or update Silt there
When adding a machine finds no `silt` there, **Install Silt on …** puts this computer's build on it, then adds it and connects; a machine already in the list has **Install Silt there** on its row. When a connected machine runs another build than this computer's, its row says which this computer has, and **Update …**, or **Update Silt there** in its menu, puts this computer's build in place of the one it has. The app never does either by itself.

The machine gets the same build as this computer, whichever channel it's on, so the window and the machine agree:
- When it runs the same OS and CPU, the app copies its own `silt`, with `silt-bwrap` and `silt-pasta` when they're beside it, over SSH, showing how much has gone.
- Otherwise it says what differs, and how to build it there with `cargo install --locked --git https://github.com/Urcra/silt silt-cli`.

A fresh install goes in `~/.local/bin`, and an update replaces the `silt` it found. Each is checked to run there before it's put in place, so one that can't, say for a C library older than this computer's, leaves nothing behind and the error says what it needs. Silt never uses `sudo`: a `silt` your user there can't replace, such as one in `/usr/local/bin` owned by root, is left as it is, and the error says how to fix it.

After an update the machine's daemon restarts on the new build, at once when no agent works in it, else once they finish, so an update never cuts an agent off. The row says when it's waiting.

## What stays on each machine
Each machine keeps its own threads, as with [WSL](windows.md); All machines lists them together, but doesn't move them. The other machine's daemon keeps running after the app disconnects, so its agents carry on; `packaging/systemd/silt-daemon.service` keeps it running there across reboots. To move a thread between machines, sign both in to a server and [hand it off](handoff.md).

## When it can't connect
- **The host key isn't trusted:** run `ssh <destination>` once in a terminal and accept the key, or fix `known_hosts` if it changed.
- **The sign-in was refused:** add your key to the machine's `~/.ssh/authorized_keys`, or to your SSH agent.
- **It can't be reached:** check it's on and reachable from here.
- **`ssh` isn't installed here:** install OpenSSH's client. Windows 10 and 11 come with it.
- **`silt` isn't installed there,** or not on your login shell's `PATH` nor in `~/.local/bin`: press **Install Silt there**.
- **The port is in use here:** the other machine's daemon picked a port something on this computer already uses. `silt daemon restart` there picks another.
