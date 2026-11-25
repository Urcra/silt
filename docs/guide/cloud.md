# Record Claude Code cloud sessions

> **Experimental.** This works, but its design is still a proposal ([ADR 0038](../adr/0038-cloud-sessions.md)), and it depends on how Claude Code on the web starts sessions, which can change.

Claude Code on the web runs each session in a container that is thrown away afterwards. Silt can record those sessions as threads and push them to your server, so that you can search them, resume them on your own machine, or fork them. See [ADR 0038](../adr/0038-cloud-sessions.md) for why it works this way.

There, Claude Code starts itself, so `silt claude` can't start it. Instead, hooks in the repository start the recording.

## Set it up
1. **Allow the server.** In the cloud environment's settings, allow your Silt server's host in the network policy.
2. **Give the environment an account.** Add these as environment variables:
   - `SILT_SERVER`: your server's URL;
   - `SILT_USER`: your account on it;
   - `SILT_PASSWORD`: its password.

   Use your own account: a thread belongs to the account that recorded it, and only its owner resumes it rather than forking it. Anyone who can edit the environment can read the password, and `silt server user sign-out <you>` signs every container out, along with your other devices.
3. **Install and sign in.** Make the environment's setup script run [`packaging/claude-cloud/setup.sh`](https://github.com/Urcra/silt/blob/main/packaging/claude-cloud/setup.sh), or the same steps:
   - install `silt`;
   - run `silt auth login --server "$SILT_SERVER" --user "$SILT_USER" --password-stdin`, with the password on stdin.
4. **Add the hooks.** Copy [`packaging/claude-cloud/settings.json`](https://github.com/Urcra/silt/blob/main/packaging/claude-cloud/settings.json) to `.claude/settings.json` in your repository, or merge its `hooks` into the one you have.
   - Each hook does nothing unless `CLAUDE_CODE_REMOTE` is `true` and `silt` is installed. So on your own machine, where `silt claude` adds its own hooks, the session isn't recorded twice.
   - `SessionStart` runs `silt track --from-hook`, which starts recording the session the hook names.
   - The other hooks are the ones `silt claude` adds.

Start a cloud session in that repository. Its first hook starts the recording, and every `Stop` pushes the thread to the server. Then, on your machine, `silt log` or `silt search` finds it, and `silt resume <thread>` continues it there.

## Start and steer them from the web UI
On a machine where `claude` is signed in to your claude.ai account, Silt's web UI starts cloud sessions and talks to them, without a terminal ([ADR 0040](../adr/0040-cloud-sessions-from-the-web-ui.md)). It runs your own `claude`, so your sign-in never leaves it.
- **Start one.** On a new thread, pick **Cloud** under the composer, describe the task, and start it. Silt runs `claude --cloud` in the project's clone, which starts the session from the clone's GitHub remote at its current branch, so push the branch first. The session is listed under **Cloud sessions**, with a link to claude.ai, until its thread arrives. A repository set up as above records it; elsewhere, only the link is shown.
- **Send it a follow-up.** A cloud thread says so at its top, links to claude.ai, and has a box for a follow-up. Silt passes it on with `claude -p --cloud`, and the reply arrives as the thread's next turn.
- **Bring it here.** **Resume** brings the thread's code and conversation to this machine, like any thread. From then on it follows the session here rather than the cloud one, so its follow-up box goes away.

The server's hosted app shows cloud threads and their links, but can't start or steer them: only a machine with `claude` signed in can.

## Limits
- **Unpushed work is lost with the container.** A thread is pushed when each turn stops, and by the daemon while the session runs. Whatever the container still held when it was reclaimed is gone, but so is the session itself.
- **A resumed cloud session becomes a new thread.** A container that starts again for the same session begins with an empty store, so Silt doesn't know the session's thread and records it from the start as another one.
- **Resuming into a cloud session isn't possible.** Claude Code chooses the session there, so `silt resume` can't hand it a recorded one. Threads go from the cloud to your machines, not back.
- **Starting reads what `claude --cloud` shows.** It only runs in a terminal, so Silt gives it one and waits up to a minute for it to name the session. If a Claude Code release changes what it shows, starting fails with the terminal's last lines as the error.
- **The setup script builds `silt` from source,** without the web UI, which takes a few minutes in each new container. The environment's network policy must allow `github.com`, and crates.io's hosts unless the package registries are allowed.
