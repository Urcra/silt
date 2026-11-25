# 0102: A sandboxed agent asks for more folders

- Status: accepted
- Date: 2026-11-18
- Amends: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md) (what a run reaches is set when it starts)
- Amended by: [ADR 0104](0104-one-plugin-model.md): Folders is a WebAssembly plugin shipped with Silt, asking through the `reach` capability

## Context
A sandboxed agent sees its thread's checkout and nothing else of the person's files. Work often spans more than one repo: a library and the app using it, or a service and its client. The only way in was "More folders", a workspace rule fixed before the run starts, and it had gaps:
- **Folders were mounted after the credentials were hidden,** so `~` or `~/.local/share` brought `~/.ssh` and Silt's store back.
- **Nothing was resolved:** a folder saved from Settings as `~/x` reached bubblewrap as a relative path, and a link was followed into whatever it named.
- **A writable repo's `.git/config` and hooks were writable,** so what the agent put there ran on the host the next time the person used git there (B2.3 again).

bubblewrap can't add a mount to a sandbox that's running.

## Decision
**An agent asks for a folder with `silt_folder_request`,** on its run's channel, giving the path, whether it needs to write, and why.
- **The workspace's rules answer first.** Folder rules name roots, each allowed, asked or never, and whether writing may be given. The most specific root wins, and a folder no rule names is asked. An org's rules tighten the person's, as other rules do (ADR 0065).
- **What is never handed over, whatever the rules say:**
  - `/`;
  - the home folder;
  - Silt's data;
  - a credential's folder;
  - anything that holds one of these or is inside one.

  A folder is judged where it really is, every link followed.
- **An asked folder waits as an approval,** bound to the hash of Silt's copy of the request, which holds the real path and whether it's writable. The person grants it for this run, this thread on this machine, or always in the workspace, which adds it to the workspace's own folders.
- **A granted folder reaches the agent at a restart.** When the turn ends, Silt stops the agent, starts its sandbox again with the folder, and resumes the same session with a note saying what's now there. The run's token, grants and device relays carry over. A revoked grant narrows the same way.
- **The folder is the person's own,** bound in as it is. Its edits aren't captured with the thread.
- **Every repo in a writable folder** has its `.git` pointer, git config, hooks, `commondir` and `.silt/` mounted read-only, found by walking the folder without following links. A folder too large to walk isn't given writable.

The workspace's own "More folders" go through the same checks.

## Consequences
- **A run's reach can now widen,** but only at a restart, so the shield still shows what the running sandbox has.
- **An agent's turn ends when a folder is granted.** Resuming needs an agent that can resume; for one that can't, the person starts it again.
- **Grants are this machine's.** A thread resumed elsewhere starts without them.
- **A writable folder is trusted as the checkout is,** for everything but git's own files: the guide says to review what the agent changed there before running it.
