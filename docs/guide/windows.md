# Windows and WSL

Silt runs on Windows itself, and inside WSL 2 as on any Linux machine. Use whichever side your repository and your agent live on, or both.

## On Windows
`silt.exe` is built from source, in either of two ways:

- **With Cargo on Windows**, as in [Getting started](index.md): `cargo install --locked --git https://github.com/Urcra/silt silt-cli`. It installs into Cargo's `bin` folder, which rustup puts on your `PATH`, and updates the same way.
- **From a clone in WSL**, with Rust installed on Windows too: `just install-wsl` builds with Windows' own cargo and installs `silt.exe`, the desktop app and the speech helper into `%LOCALAPPDATA%\Silt`, as well as Silt in the distribution. Add that folder to your `PATH` yourself. `silt://` links open the app only once something registers them, which this doesn't.

Then everything in [Getting started](index.md) works as it does on Linux: `silt import`, `silt claude` in a clone, `silt open`.

- Silt keeps its data in `%LOCALAPPDATA%\silt\data`, closed to other users of the machine. `SILT_HOME` names another place.
- The daemon runs in the background with no window, and closing the terminal that started it leaves it running. `silt daemon stop` stops it.
- Claude Code's hooks start `silt.exe` directly, without Git Bash or PowerShell in between.
- A checkout doesn't write files Windows can't hold under their name, such as `aux.txt` or `a:b`, which a thread from Linux may have. They stay in the thread for the machines that can.
- Keep repositories on a local NTFS drive. Network drives are polled, which is slower to notice changes.

## In WSL
Inside a WSL 2 distribution, install and use Silt as on Linux, built with Cargo as in [Getting started](index.md). Keep the repository on the distribution's own file system, such as `~/src`, rather than under `/mnt/c`: there file changes can't be watched and are polled instead, and writes are much slower.

Each distribution with Silt is a machine of its own, with its own daemon and data, as is Windows itself. To move a thread between Windows and a distribution, sign both in to the same server and [hand it off](handoff.md), as between any two machines.

## One window for both
The desktop app on Windows shows this computer's Silt, and in the machine menu at the bottom of the sidebar each WSL distribution's. Picking a distribution shows its web UI in the same window. A VPS or another computer you reach over SSH can be listed there too: see [Machines over SSH](machines.md).

When the app launches, it starts Silt on Windows and in every distribution that has it, through `wsl.exe`, so each one records, watches its checkouts and syncs without a `silt open` in PowerShell and another in WSL. The window shows one at a time.

`silt open` in a WSL shell opens the Windows app on that distribution and the folder you ran it in, when the app is installed; without it, `silt open` opens your browser as before. On Windows, `silt open` in a `\\wsl.localhost\<distribution>\…` folder opens it in that distribution's Silt the same way. Other commands there say which distribution to run them in, since its files belong to its own Silt.
