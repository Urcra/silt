# 0108: Machines over SSH

- Status: accepted
- Date: 2026-11-19
- Amends: [ADR 0041](0041-the-desktop-app.md) (the window shows other machines' daemons), [ADR 0045](0045-windows-natively-and-in-wsl.md) (a third kind of backend, beside this computer and WSL's distributions)
- Amended by: [ADR 0109](0109-installing-and-updating-silt-over-ssh.md): a machine without `silt`, or with another build of it, can get this computer's
- Amended by: [ADR 0115](0115-the-sidebar-names-the-workspace-and-the-machine.md): The sidebar names the workspace at its top and the machine at its bottom

## Context
The desktop app on Windows shows each WSL distribution's Silt in the same window, under **Runs on** (ADR 0045). It needs no server for that. It runs the distribution's `silt open --no-browser`, which prints `http://127.0.0.1:<port>/#token=…`, and WSL 2 forwards the port to Windows.

People also work on a VPS or a machine at home, for its GPU or its uptime, or to leave an agent running there. Today that takes a server. Both machines sign in to it, and the hosted app drives the other machine's agents through it (ADR 0026). Without a server, the person SSHes in and runs `silt claude` in a terminal, or forwards the daemon's port by hand.

An SSH connection gives the same thing as WSL's forwarding, a loopback port reaching the other machine's daemon. And the person's SSH setup already knows how to reach their machines: keys, the agent, `~/.ssh/config` and its `ProxyJump`, and `known_hosts`.

## Decision
**A machine over SSH is a third kind of backend, `ssh:<name>`.** It's a daemon on another computer, with its own data, shown in this window as a distribution's is. Nothing about it needs a server. As with WSL, moving a thread between that machine and this one still does, by handoff (handoff.md).

**The person adds machines in Settings › Machines.** The section is shown only in the desktop app.
- **A machine** has a name, a destination and an optional port. The name is what Runs on shows and what its id is made of. The destination is `user@host`, `host`, or a `Host` alias from `~/.ssh/config`.
  - A name is letters, digits, `.`, `_`, `-` and spaces, up to 64. A destination is an optional `user@` and a host of letters, digits, `.`, `_`, `-` and `:` (for IPv6), up to 255, never starting with `-`. A port is 1–65535. Anything else is refused, so neither can become an `ssh` option.
  - Machines are kept in `machines.json` in the app's data dir, with entries that don't pass these checks left out: the file is the person's to edit.
- **The page asks the app** with links, as it does for the theme and zoom: `silt://machines/add`, `change`, `remove`, `test` and `disconnect`, with the machine's fields in the query.
  - The app takes them only while the window shows this computer or a WSL distribution. A machine over SSH serves its own page, and must not get the app to connect elsewhere or forget a machine. Its Settings › Machines lists the machines, and can open one, but edits them only from this computer.
  - Each link also carries a key the app makes at launch and gives the window's main frame in `window.__siltDesktop.key`. An artifact's frame can navigate too, but never sees the key.
  - A `silt://open/` link may pick an SSH machine the app knows, but not a project on it: no `silt open` on that machine can reach this window.
- **The page learns the machines** from `window.__siltBackends`, whose entries gain `kind` (`native`, `wsl` or `ssh`) and, for SSH, `destination`, `port`, `state`, `since`, `lastShown` and `version`. `state` is `idle`, `connecting` (with the `step` under way), `connected` or `failed` (with a `reason`). A test's steps arrive as `silt-machine-test` events. A `silt-backends` event brings the list again whenever it, or a machine's state, changes. An older UI on the other machine ignores the new fields, and its Runs on still switches.

**The app connects with the system's OpenSSH client:** `ssh` on Linux and macOS, and `ssh.exe` on Windows, which ships with Windows 10 and 11.
- Every run is `ssh -T -o BatchMode=yes -o ConnectTimeout=10 -o ForwardAgent=no -o ForwardX11=no [-p <port>] <destination> <command>`, and a command also takes `ClearAllForwardings=yes`, so forwards from `~/.ssh/config` don't ride along.
  - `BatchMode` means no prompts: the window has no terminal to type a password or a passphrase into. Keys, the SSH agent and the person's config sign in.
  - A host whose key isn't in `known_hosts` is refused, never accepted: whether to trust it is the person's call. The error says to run `ssh <destination>` once in a terminal.
- **Finding `silt`.** Every connect first probes the machine through the person's login shell: who it signed in as, `command -v silt` and its `--version`. A machine's `silt` can move as it's updated, so nothing is remembered.
- **Running it.** Commands run through the person's own login shell, as in WSL (`IN_THEIR_SHELL`), so a daemon started there finds the agents their terminal does. The remote command line is fixed text and the remembered path, quoted for `sh`.
- **The tunnel.** The app runs `<silt> open --no-browser`, which starts the daemon if none runs, and reads the address. Then it forwards the same port, `ssh -N -o ExitOnForwardFailure=yes -L 127.0.0.1:<port>:127.0.0.1:<port> <destination>`.
  - The port is the same on both ends because the daemon answers only `Host: 127.0.0.1:<its port>`, which guards it against DNS rebinding. That check stays as it is.
  - Daemons pick a free port at random, so a clash with one here is rare. When the port is taken here, the error says so, and a `silt daemon restart` there picks another.
  - The page loads from `http://127.0.0.1:<port>/`, so the window's checks (`web_url`, `stays_in_app`, `may_listen`) are unchanged.
- **One connection.** On Linux and macOS, the runs share one connection (`ControlMaster=auto`, `ControlPersist=60`), with the control socket in a directory of the app's own, closed to other users. The tunnel has its own (`ControlPath=none`), so ending it closes only it. Windows' OpenSSH can't share connections, so there each run connects again.
- **Once connected.** The tunnel stays up until the person disconnects, changes or removes the machine, or quits the app, so switching back is instant. Settings shows each machine's state, and the app notices a tunnel that ends.
  - When the tunnel or the daemon goes, the window's existing check finds the UI gone, and it opens and forwards again with backoff.
  - The daemon on the other machine runs detached, so it outlives the SSH session, and its agents keep working with the window closed. A systemd unit (`packaging/systemd/silt-daemon.service`) keeps it across reboots there.
- **Not at launch.** SSH machines aren't started when the app launches, nor restarted when it quits (`restart_replaced`). The app connects when one is picked, or when Settings tests one, so an offline VPS doesn't slow the launch. The last backend shown still opens first, and if that's an SSH machine, the app connects to it then.

**What a failure says.** ssh's stderr and exit code are read into a few kinds, each with its next step:
- the host is unknown or changed;
- the sign-in was refused, with no key or agent;
- the machine can't be reached, or timed out;
- `ssh` isn't installed here;
- `silt` isn't installed there;
- the port is in use here.

Settings shows each on its machine, and the starting page shows it for the machine being opened. A switch that fails while the window shows another backend keeps it, and the page shows the reason in a toast and on the machine's row.

**Runs on shows in the desktop app always,** not only with WSL, since it's where "Add a machine…" is.

## Consequences
- A VPS works with Silt from the desktop app with nothing but SSH: no server, no open ports on the VPS, and no credentials of Silt's own. Whoever can SSH in can already read the daemon's token.
- The person's own `ssh` decides how to connect, so jump hosts, hardware keys, 1Password's or Secretive's agents and their config all work, and Silt keeps no SSH keys.
- A machine needing a password or an interactive prompt can't be added until it has a key. That's the trade for not handling passwords.
- The other machine's page is the other machine's build. Settings › Machines and the new Runs on states appear there only once it runs a build with them.
- Nothing is synced between the machines by this. The person who wants a thread on both still hands it off through a server. The hosted app and notifications still need one too.
- The tray and notifications cover SSH machines only while connected.

## Alternatives considered
- **A daemon listening beyond loopback, with TLS.** It would expose the API to the network, and need certificates and a second sign-in. SSH already authenticates.
- **An SSH client inside the app (`russh`).** It would lose the person's config, agent, `known_hosts` and jump hosts, and Silt would hold keys.
- **Forwarding to a free port here and loosening the `Host` check.** The check would have to accept ports it didn't pick, weakening the rebinding guard for no gain over keeping the port.
- **Connecting every machine at launch, as distributions start.** A VPS can be slow or down, and connecting would hold a connection to each for the app's whole run.
- **Prompting for passwords in the app.** It would put passwords through Silt's window, and they would have to be stored or asked again each time. Keys are the norm for a VPS.
