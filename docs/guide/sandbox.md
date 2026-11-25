# Silt's sandbox: what it protects, and what it doesn't

Silt's sandbox runs an agent, with everything it starts, in a world of its own on your machine. It sees the thread's checkout and the tools it builds with, and little else. A workspace turns it on with `silt workspace policy Work --silt-sandbox on`, and [Workspaces](workspaces.md#silt-s-sandbox) says what each of its rules hands over. This page says what it's for, how it works, and where it stops.

## What it's for
The sandbox is meant for an agent that is eager rather than hostile. An agent can try very hard to get its task done. It's debugging a bug that only shows in production, so it goes looking for the production database's password in your shell's variables, your `~/.aws`, your `~/.pgpass` or your SSH keys. Or it decides the fix needs a change on a server it can reach over your VPN, or it pushes a branch with your forge token. None of this is malice, and an agent asked to "find out why it fails" may still do it.

In the sandbox, that agent finds nothing to do it with:
- **No credentials.** Your SSH, cloud and forge credentials, keyrings and password managers aren't there, and neither is the agent's own stored sign-in. Of your shell's variables, only an allowlist goes in, and never one named as a token, key, secret or password. A credential the workspace gives it is added by Silt's gateway on the way out, only to the host it's for, so the agent holds only a placeholder.
- **No way in to production.** Full internet never reaches this machine, its networks, your VPN's private ranges or cloud metadata. Production is reached only through [plugins](plugins.md), which Silt runs outside the sandbox. A change there waits for you to approve Silt's own copy of the request, and data rules can keep a class of data, such as location data, from the agent entirely.
- **No acting as you.** It can't reach your `ssh-agent`, Docker, D-Bus or Silt's own socket. Its channel to Silt answers only for its own run, and can't approve anything.
- **Nothing that runs later by itself.** It can't change what git, Silt or the agent itself would run outside the sandbox without your asking: the checkout's `.git` and its `config`, `hooks` and `commondir`, `.silt`, and the agent's settings, memory, plugins, hooks, MCP servers, commands and skills.
- **Only its own project.** It sees its project's sessions, not other projects', and none of Silt's data.

## What it isn't for
The sandbox is not a wall against an adversary who controls the agent, such as a prompt injection that tells it to attack you. It can't be, because the agent's work is code you will run.

An attacker who can make the agent write anything can write a test. When you then run `npm test` or `cargo test` on your machine, outside the sandbox, the test runs as you, with everything the sandbox kept from it. The same goes for a build script, a `Makefile`, a dependency the agent added, a `.envrc`, an editor's task file, or a git hook a tool installs from the repository. Nothing the sandbox does can tell good code from bad code that you later choose to run.

So the sandbox keeps the agent's reach small while it works, and you decide what it made is safe:
- **Review the agent's changes before you trust them.** Read the diff of everything it changed in the folders it could write (below) before you run its tests, build it, or merge it, as you would a colleague's pull request. Silt's thread shows every file it edited, and `silt diff` shows what changed.
- **Run untrusted work in the sandbox too.** Tests and builds the agent runs itself stay inside. A command you run from your own shell doesn't.
- **Expect what it can read to be sent out.** With full internet, an agent can send what it sees, the code and what you showed it, anywhere public. Choose `--network gateway` with a list of hosts, or `none`, where that matters.

## What it can change
These are the folders whose changes outlive the run, and so the ones to review:
- **The thread's checkout**, all of it but what's pinned above. This is the work.
- **The git dir's objects, refs and index**, so it can commit. Branches and commits it made show in `git log`.
- **Build caches** shared with your own builds: `~/.cache`, `~/.npm`, `~/.cargo/registry` and `~/.cargo/git`, `~/go/pkg`, pnpm's store and Gradle's caches. They're writable so builds are fast; a build you run later reads them, so clear them if a run went somewhere you didn't expect.
- **The agent's own folder**, such as `~/.claude`, for its sessions in this project.
- **What plugins give it:** a folder Folders granted writable, and the work area at `/silt/work` where it clones what GitHub's plugin keeps for the thread ([Plugins](plugins.md)).
- **What the workspace hands over writable**, with `--paths /path:rw`, `--docker on` or `--ssh-agent on`. Docker is root on your machine, and the SSH agent signs as you, so a workspace that hands those over gives up much of the above.

## How it works
On Linux, Silt builds the sandbox with bubblewrap, from your distribution's `bubblewrap` package. `silt access doctor` says whether this machine can make one. On macOS it uses Seatbelt, which is written but hasn't been tried on a real Mac yet (below). Containers and virtual machines are to follow.
- **A root of its own.** The sandbox starts from an empty root and Silt mounts in what it needs: the system's folders and your toolchains read-only, then the checkout, the git dir and the agent's folder writable, then what must stay read-only on top of those. What isn't mounted can't be found, under any name: a link that leads to your home folder, Silt's data or a credential isn't followed into it.
- **Its own processes and network.** It has its own process, user, IPC and host-name namespaces, so it sees and signals only its own processes, and stopping the run ends all of them. Its network is its own too: none at all, a relay to Silt's gateway only, or full internet through pasta, with routes it can't change that make this machine, its networks and cloud metadata unreachable.
- **A filter on system calls.** It can't type into your terminal, use `io_uring`, open the kernel's keyrings, or reach a virtual machine's host over vsock.
- **The gateway.** Its HTTP(S) leaves through Silt's gateway, which refuses hosts the rules don't allow and addresses on this machine and its networks, however they're written, and adds the workspace's credentials for the hosts they're for. The sandbox trusts the gateway's own certificate authority, whose key stays outside.
- **One channel to Silt.** Its hooks and Silt's tools reach the daemon through a socket that answers only requests carrying the run's token, and only for its own thread.

## Where it stops
- **Host network** (`--network host`) gives the sandbox this machine's own network, `127.0.0.1` and Silt's web UI included. It's for when nothing else works.
- **On macOS**, Seatbelt has no process namespace, so a process that detaches can outlive its run, and full internet goes through the gateway only.
- **The agent's own model** is reached, so what the agent reads goes to its provider, as it would without the sandbox.
- **A sign-in kept in the agent's folder**, such as Claude Code's subscription, isn't shown in the sandbox, so an agent there signs in with a key the gateway adds.
