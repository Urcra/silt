# Folders

An agent in Silt's sandbox sees its thread's checkout and nothing else of your files. With Folders it can ask for more: another repo it needs, a dataset, the library the app uses.

## What the agent does
It calls `folders_request` with the folder, whether it needs to write there, and why.

## What decides
The workspace's folder rules answer first (Settings › Workspaces › What its agents can reach, or `silt workspace policy <workspace> --ask-folders`). Each names a root and says allowed, asked or never, and whether writing may be given there. The most specific root wins, and a folder no rule names is asked. An org's rules only tighten yours.

Some folders are never handed over, whatever the rules say: `/`, your home folder itself, Silt's data, a credential's folder (such as `~/.ssh`), and anything holding one of these. A folder is judged where it really is, every link followed.

## What you see
A card with the folder where it really is, whether it's writable, the git repos in it, and the agent's reason. Grant it for this run, for this thread on this machine, or always in the workspace.

## What happens then
Nothing changes in the running sandbox: bubblewrap can't add a folder to one. When the agent's turn ends, Silt starts its sandbox again with the folder at its own path and resumes the same session, saying what's now there. Every repo in a writable folder keeps its git config, hooks and `.silt/` read-only. Revoking a grant narrows the same way.

The folder is yours, bound in as it is: what the agent changes there isn't captured with the thread. Review it before running anything it wrote.
