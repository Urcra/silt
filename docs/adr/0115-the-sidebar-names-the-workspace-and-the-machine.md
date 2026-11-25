# 0115: The sidebar names the workspace at its top and the machine at its bottom

- Status: accepted
- Date: 2026-11-23
- Amends: [ADR 0044](0044-the-standard-agent-workspace.md) (the sidebar's header and foot), [ADR 0032](0032-several-servers-and-local-projects.md) and [ADR 0065](0065-workspaces.md) (where the workspace switcher sits and what it shows), [ADR 0045](0045-windows-natively-and-in-wsl.md) and [ADR 0108](0108-machines-over-ssh.md) (Runs on), [ADR 0100](0100-update-channels.md) (the update pill)

## Context
The sidebar's header read `silt  [All workspaces ˅]  🔍`, and its foot `● this machi…  [Updating · 25%]  ⚙`. Three ideas shared those two lines and none was named:
- **The workspace** was a small bordered chip beside the wordmark. It read as a filter, though it also picks the server, the rules and the account.
- **The machine** was "this machine" or a server's name for it, cut short, with a dot that meant sync rather than "agents run here". In the desktop app a second row, **Runs on**, named the machine the window shows, so the foot named a machine twice in different words.
- **Settings** was drawn as eight rays, which read as the sun, the theme's icon, squeezed into the corner beside the update pill.

A thread another machine holds said "Not on this machine": where it isn't, not where it is. The daemon couldn't say more. A thread's lease names a device only once another device takes it over (ADR 0010), and the listing named no devices.

## Decision
**The workspace is the sidebar's title.** The switcher fills the header: the workspace's tile, its name, and a line under it, its server or `this machine only` (`view only` and the workspaces' names for All workspaces). Everything below is in it. Search, which was an icon beside the switcher, is a row under New chat with its shortcut. The menu adds **Signed in**: each server's account, with whether it syncs. An account belongs to a server, and a server to a workspace, so the avatar leaves the foot.

**The machine is the sidebar's foot.** One row: the machine this window shows, its tile (a laptop, or a terminal for one over SSH) with a dot for how it stands, its name, and one line, `Synced · 2 running`, `Sync failing · 1 running` or `Not synced · nothing running`. It opens a menu:
- the machines the desktop app knows, This computer first, each with how it stands, and the one shown checked. Picking one switches as Runs on did, which it replaces;
- **Add a machine**, in the desktop app only;
- **Devices and sync**, with the server and whether it syncs.

A browser has one machine, so its menu lists only This computer and Devices and sync.

**Settings is a cog** of its own beside the machine, and the icon is a cog everywhere. **An update** is a strip above the machine, with its progress while it runs and Relaunch in the desktop app, so nothing squeezes the machine's name.

**A thread elsewhere names its machine**, `On homelab · 2d`, and a moved one `Moved to home`. The server keeps the device that registered each thread (`threads.made_on`), and its listing gives the person's devices by name and where each listed thread was made. A daemon names a thread's holder by its lease, else where it was made, when that is one of the person's own devices and not this one. The hosted app does the same from the server's own tables. Others' devices are never named. Protocol 23.

## Consequences
- The sidebar no longer shows the wordmark. The hosted app, which has no switcher, keeps it as its title.
- A thread registered before protocol 23 has no `made_on`, so it reads "Not on this machine" until a device takes its lease.
- The machine menu shows how many agents run only on the machine shown: the others' counts need their daemons asked over the tunnel, already listed as a task.
