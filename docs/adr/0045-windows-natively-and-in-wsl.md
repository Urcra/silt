# 0045: Windows, natively and in WSL

- Status: accepted
- Date: 2026-10-24
- Amends: [ADR 0016](0016-untrusted-peer-content.md) (what a checkout refuses to write), [ADR 0041](0041-the-desktop-app.md) (Linux only, one daemon per window)
- Amended by: [ADR 0119](0119-no-windows-ci-or-releases.md): no Windows CI job and no Windows release builds
- Amended by: [ADR 0046](0046-no-server-needed-on-one-computer.md): No server needed on one computer
- Amended by: [ADR 0064](0064-the-desktop-app-starts-every-backend.md): The desktop app starts every backend when it launches
- Amended by: [ADR 0108](0108-machines-over-ssh.md): Machines over SSH
- Amended by: [ADR 0115](0115-the-sidebar-names-the-workspace-and-the-machine.md): The sidebar names the workspace at its top and the machine at its bottom
- Amended by: [ADR 0139](0139-macos-and-windows-ci-advisory-at-first.md): macOS and Windows CI, advisory at first

## Context
Silt runs on Linux and WSL2, macOS was next (M25), and Windows came after it. Most people who could try Silt work on Windows, so Windows comes first now.

Today a Windows user runs everything inside WSL. A repository on `C:` works only if it's reached from WSL through `/mnt/c`. That path has no inotify, so the watcher polls it: a 10k-file scan takes 5 s instead of 6 ms, and writes are about 100× slower (docs/research/probes.md). An agent run natively on Windows isn't captured at all.

The `silt` binary doesn't build for Windows:
- the control channel is a Unix socket;
- the daemon detaches through `CommandExt::process_group`;
- the data dir, tokens and scripts get Unix modes (`paths.rs`, `disk.rs`, `remote.rs`, `admin.rs`, `silt-acp`'s registry, `silt-git`'s conventions);
- links open with `xdg-open`.

A Windows user's projects live on both sides, so the split has to be clear: the window runs on Windows, and everything that touches a project runs where the project is. Mixing the two sides breaks in predictable ways:
- Windows-side git and icons fail on Linux paths;
- an agent runs PowerShell against a `\\wsl$` path;
- a launcher on Windows starts a second, Windows-side server.

So paths are translated only at the app's boundary, and a start never relies on shell init files. A person should be able to open the window from either side without having to know which side it runs on.

## Decision
**Windows comes before macOS.** M28 (Windows) goes ahead of M25 (macOS), and the work they share lands in M28: case-insensitive file systems, and a second platform in CI and in the agent adapters' paths.

**A native Windows `silt`, from the start.** On Windows, the CLI, the daemon, capture, hooks, ACP agents and the web UI are the same as on Linux. Release builds cover x86_64 and aarch64 (`*-pc-windows-msvc`).
- **The control channel** becomes a named pipe, `\\.\pipe\silt-<hash of the data dir>`. Each data dir, so each test's temporary `SILT_HOME`, gets its own pipe. Its security descriptor admits only the current user and SYSTEM, and it refuses remote clients. The first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so nobody can take the name before the daemon. A client checks that the pipe's server runs as the same user before sending anything, as a client on Unix checks the socket's owner.
- **Platform code lives in `silt-os`,** the one crate allowed `unsafe`, for its Win32 calls. The other crates call it rather than carrying `cfg`s of their own.
- **The data dir** is `$SILT_HOME`, else `%LOCALAPPDATA%\silt\data`: the desktop app's installer puts the app in `%LOCALAPPDATA%\Silt`, which Windows doesn't tell apart from `%LOCALAPPDATA%\silt`, and the data stays out of its way. The store and blobs are too big for a roaming profile. Where Linux uses 0700 and 0600 modes, Windows sets a protected DACL that grants only the current user and SYSTEM, on the data dir and on each token file Silt writes. The data dir's is set on the directory alone, inherited by what's created in it later: what's already there sits in the user's profile, closed to others, and walking a large store at every start would be slow.
- **Starting the daemon** (`ensure_daemon`, ADR 0036) uses `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP`, plus `CREATE_BREAKAWAY_FROM_JOB`, so a terminal or agent that kills its job on exit doesn't take the daemon with it. A job that refuses breaking away keeps the daemon, and a hook starts it again (ADR 0036). It gets a hidden console of its own rather than none (`DETACHED_PROCESS`), so the programs it runs, git above all, don't each open a window. `silt daemon stop` asks through the control channel, as on Linux; no signals are involved.
- **Watchers** use `ReadDirectoryChangesW` through `notify`. A buffer overflow is a rescan, as an inotify overflow is. Network drives and SMB shares poll, as 9p and NFS do on Linux.
- **Hooks** name `silt.exe` by its absolute path, in double quotes, which cmd, PowerShell and Git Bash read alike for a path without `"`, `$`, `` ` `` or `%`. How each agent CLI runs a hook command on Windows (cmd, PowerShell or Git Bash) is probed before an adapter claims Windows. Claude Code comes first (ADR 0007).
- **Agent adapters** find their session files where the Windows build of each agent writes them, under `%USERPROFILE%` or `%APPDATA%`. Each adapter names the platforms it's tested on, as M22 asks for versions.
- **Links and executables.** Directory links are junctions, which need no privilege. File links that Windows refuses without Developer Mode are copies, refreshed when Silt writes the convention. Executable bits follow `core.fileMode`, which git turns off on Windows (`tracks_file_mode`).
- **Line endings.** Deltas carry what git stores, after the clean filter. Checkouts write what git would check out, after the smudge filter. So a CRLF checkout on Windows and an LF one on Linux sync without every line changing.
- **Paths in deltas** stay `/`-separated and relative to the project. OS paths such as a session's cwd are rewritten at resume, as they already are between two Linux machines. The wire format doesn't change, so `PROTOCOL` doesn't bump.
- **`silt server`** builds and runs on Windows, since the multi-machine tests start one. The documented deployment stays Linux: no Windows service, and self-hosting.md is unchanged.

**What a checkout refuses on Windows (amends ADR 0016).** A Linux peer can send a tree that Windows can't hold, or treats specially.
- On every platform, `is_writable` already refuses `.git` in any case, with trailing dots or spaces, and its 8.3 alias `git~1`. It now also refuses `.git` followed by an NTFS stream (`.git::$INDEX_ALLOCATION`), the last name Git's `core.protectNTFS` guards.
- On Windows it also refuses any component that:
  - contains `\`, `:`, `<`, `>`, `"`, `|`, `?`, `*` or a control character;
  - is a reserved device name (`CON`, `PRN`, `AUX`, `NUL`, `COM0`–`COM9`, `LPT0`–`LPT9`, the superscript-digit forms too), with or without an extension.
- A refused file stays in the log and the tree. The checkout skips it, as it already skips paths outside the write policy, but no longer silently: the thread lists the files this machine couldn't write.
- Two paths that differ only by case can't both be written on NTFS's default case-insensitive directories. The checkout writes the first and lists the second the same way, as M25 planned for macOS.
- Long paths: Rust's standard library already opens a path past 260 characters through `\\?\`, and every git command Silt runs gets `core.longpaths=true`, as does a project's clone. On Windows git's hooks path is `NUL`, which can't be a directory, rather than `/dev/null`, which would be one on the current drive.

**WSL: each environment is its own machine.** A distribution with Silt installed runs its own Linux daemon, with its own data dir and agent homes, and appears as its own device. Work moves between Windows and a distribution by handoff through a home server, as between any two machines (ADR 0009). Nothing is shared on disk, so the two sides can't disagree about config.
- The Windows daemon never touches a distribution's files. `silt open` on Windows in a clone under `\\wsl.localhost\<distro>\` or `\\wsl$\<distro>\` reopens it in that distribution's backend, at the Linux path (see below). Every other command there refuses, and names the distribution and the command to run inside it.
- A WSL daemon on `/mnt/<drive>` keeps the polling fallback and its warning. The guide sends Windows repositories to the native `silt` instead.
- A local-only project (ADR 0032) stays on the machine that has it. Handoff between Windows and WSL needs a server both sign in to.

**The desktop app on Windows shows several backends (amends ADR 0041).** A backend is the native daemon or one distribution's daemon. The window shows one backend at a time.
- **The native backend** is started as on Linux: `silt.exe open --no-browser`, with `silt.exe` beside the app. The app starts it in the data dir rather than `/`.
- **A distribution's backend** is started with `wsl.exe -d <distro> --exec <silt> open --no-browser`. `<silt>` is an absolute path, found once through a login shell (`sh -lc 'command -v silt'`) and remembered per distribution, so later starts never depend on shell init files.
  - Its address comes back through the pipe, as before.
  - The daemon listens on `127.0.0.1` inside the distribution. WSL2 forwards that port to Windows in NAT mode and shares the loopback in mirrored mode, so `web_url`'s check and the window's polling are unchanged.
  - A fixed `SILT_WEB_PORT` used on both sides collides in mirrored mode, and the second daemon fails to bind, as it would on one Linux machine.
- The app lists every backend:
  - the native one;
  - each distribution `wsl.exe --list --quiet` lists. Its `silt` is looked for when it's first picked, so the app doesn't start every distribution.

  The list is a native menu and the tray, since the page gets no Tauri IPC (ADR 0041). The last backend used opens first. The tray count and notifications cover every backend.
- **A daemon must outlive the `wsl.exe` that started it.** If the probe shows a distribution stopping once its last `wsl.exe` exits, the app keeps one `wsl.exe` open for each distribution it shows. Capture doesn't depend on this, because a hook starts the daemon (ADR 0018, ADR 0036); only watchers and sync would pause.
- **Packaging:** an NSIS installer, per user with no admin rights, and an MSI for managed machines. Both carry `silt.exe` beside the app (Tauri's `externalBin`, set only in `tauri.release.conf.json`, so a plain `cargo build` of the app doesn't need it) and register `silt://`. The NSIS installer puts its directory on the user's `PATH` and takes it off at uninstall; an MSI's machines get theirs from their admins. Binaries and installers are Authenticode-signed in CI when the repository holds the certificate, and update bundles will carry Tauri's updater signature, as on Linux. The keys live only in CI's secrets.

**`silt open` in WSL opens the Windows app.** In a distribution where the Windows app is installed, `silt open` shows this distribution's backend and the project it ran in, in the Windows window, then returns. The person never has to run a Linux GUI through WSLg or keep a browser tab.
- **Detection.** `silt` is in WSL when `WSL_DISTRO_NAME` is set and Windows interop is on (`/proc/sys/fs/binfmt_misc/WSLInterop` exists). The app counts as installed when `silt://` is registered for the user (`reg.exe query HKCU\Software\Classes\silt`). Without interop or the app, `silt open` falls back to the browser through `wslview`, as today.
- **The link.** `silt open` opens `silt://open/?project=<clone>&backend=wsl:<distro>` through Windows' handler for the scheme (`rundll32 url.dll,FileProtocolHandler <link>`, which hands the link on whole; cmd splits it at `&`, and `explorer.exe` at `=`). This adds one parameter to ADR 0041's links, `backend`, which names a backend and nothing else.
- **No token in the link.** The link starts nothing on its own terms. The app treats it as untrusted, like any `silt://` link a web page could open:
  - `backend` must name the native backend or a distribution `wsl.exe --list --quiet` lists;
  - `project` is only the directory that backend's `silt open` runs in: a plain path, without `..` or control characters. That `silt` picks the project from it as it would in a shell, and a directory outside every clone lands on the empty thread;
  - the app then attaches as it always does, by running `wsl.exe -d <distro> --cd <project> --exec <silt> open --no-browser` itself. The token comes back through that pipe (ADR 0041).
- **A distribution not seen before** is found the first time a link names it: the login-shell probe for `silt` runs once and is remembered, as for a distribution found at start. A distribution without `silt` shows the app's install hint rather than an error.
- **From Windows the other way.** `silt open` in `\\wsl.localhost\<distro>\home\…` translates the path once, to `/home/…`, and opens the same link with that distribution as its backend. The path is only translated here, at the boundary, never inside the daemon.
- **Natively on Windows,** `silt open` opens the app on the native backend, as is planned for every platform.

**CI and the gate.**
- A CI job on GitHub's Windows runners runs build, clippy and tests for `x86_64-pc-windows-msvc`.
- A second Windows job sets up WSL 2 and runs the desktop backend tests: a native backend and a distribution's backend, each started from the app.
- The local gate runs clippy for `x86_64-pc-windows-gnu` when that target and MinGW are installed, and says it skipped it otherwise, as it does for the desktop app. MinGW needs no Windows SDK, unlike `cargo-xwin`, and Wine runs its test binaries for a quick look.
- A real Windows 11 machine does the final pass.

## Consequences
- macOS waits for Windows, and inherits its second-platform work: platform `cfg`s, a non-Linux CI job, case folding.
- The Unix-only code moves behind small platform modules, for the control channel, detaching, file modes, links and the opener. Linux keeps its behaviour, so its tests stay as they are.
- `fakes.rs`'s shell stand-ins don't run on Windows. The fakes become a small Rust test binary, scripted like `silt-fake-agent`, so the multi-machine tests run on both platforms.
- The write policy grows a platform argument. Its tests cover each refused Windows name on Windows, and check that Linux still writes them.
- `silt open` behaves the same wherever it runs: in a WSL shell, in PowerShell, or in a `\\wsl.localhost` folder, it ends in the one Windows window on the right backend.
- A Windows user who works in both Windows and WSL has two devices, and hands off between them through a server. That's the async handoff ADR 0006 is about, but it means signing both in.
- Windows comes ahead of macOS.
